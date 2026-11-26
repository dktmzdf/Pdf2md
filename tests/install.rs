//! 설치 스크립트 테스트. Linux는 `install.sh`, Windows는 `install.ps1`을 시험한다.
//! Release와 HuggingFace를 로컬 HTTP 서버로 흉내 내고, 실제로 빌드한 바이너리와
//! `PDF2MD_MODEL_DIR`의 실제 모델 파일을 내려준다.
#![cfg(any(target_os = "linux", target_os = "windows"))]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/skills/pdf2md/scripts");
#[cfg(target_os = "linux")]
const TARGET: &str = "x86_64-unknown-linux-gnu";
#[cfg(target_os = "windows")]
const TARGET: &str = "x86_64-pc-windows-msvc";
const EXE: &str = if cfg!(windows) { "pdf2md.exe" } else { "pdf2md" };
const ARCHIVE_EXT: &str = if cfg!(windows) { "zip" } else { "tar.gz" };
const DET_COMMIT: &str = "e7bd63d9c674c96bd4fdab435af0efb383919c18";
const REC_COMMIT: &str = "5c6f574b8e2230adf4287b33e736d71b9fabd28e";

/// `root` 아래 파일을 GET으로 내려주는 최소 HTTP 서버. 받은 요청 수를 센다.
fn serve(root: PathBuf) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            while reader.read_line(&mut line).map(|n| n > 2).unwrap_or(false) {
                line.clear(); // 나머지 헤더는 버린다
            }
            let mut s = stream;
            match std::fs::read(root.join(path.trim_start_matches('/'))) {
                Ok(body) => {
                    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(&body);
                },
                Err(_) => {
                    let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                },
            }
        }
    });
    (base, hits)
}

/// 바이너리 하나를 담은 Release 압축과 `SHA256SUMS`를 `rel`에 만든다(OS별 도구 사용).
fn pack_release(rel: &Path) {
    let archive = format!("pdf2md-{TARGET}.{ARCHIVE_EXT}");
    let (prog, args): (&str, Vec<String>) = if cfg!(windows) {
        let ps = format!(
            "Compress-Archive -Path {EXE} -DestinationPath {archive}; Remove-Item {EXE}; \
             $h = (Get-FileHash -Algorithm SHA256 {archive}).Hash.ToLower(); \
             Set-Content -Path SHA256SUMS -Value \"$h  {archive}\""
        );
        ("powershell", vec!["-NoProfile".into(), "-Command".into(), ps])
    } else {
        ("sh", vec!["-c".into(), format!("tar -czf {archive} {EXE} && rm {EXE} && sha256sum {archive} > SHA256SUMS")])
    };
    let ok = Command::new(prog).args(&args).current_dir(rel).status().unwrap().success();
    assert!(ok, "packing the fake release failed");
}

/// 가짜 Release(`release/`)와 가짜 HF(`hf/`)를 만든다.
fn fixture(name: &str) -> PathBuf {
    let models = PathBuf::from(
        std::env::var_os("PDF2MD_MODEL_DIR").expect("install tests need PDF2MD_MODEL_DIR (see README)"),
    );
    let root = std::env::temp_dir().join(format!("pdf2md-install-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let rel = root.join("release");
    std::fs::create_dir_all(&rel).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_pdf2md"), rel.join(EXE)).unwrap();
    pack_release(&rel);
    let hf = root.join("hf");
    for (from, to) in [
        ("det.onnx", format!("deepghs/paddleocr/resolve/{DET_COMMIT}/det/ch_PP-OCRv4_det/model.onnx")),
        ("rec.onnx", format!("PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/{REC_COMMIT}/inference.onnx")),
        ("inference.yml", format!("PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/{REC_COMMIT}/inference.yml")),
    ] {
        let dst = hf.join(to);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(models.join(from), dst).unwrap();
    }
    root
}

fn install(root: &Path, base: &str, extra: &[(&str, &str)]) -> Output {
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let mut c = if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(Path::new(SCRIPTS).join("install.ps1"));
        c
    } else {
        let mut c = Command::new("sh");
        c.arg(Path::new(SCRIPTS).join("install.sh"));
        c
    };
    c.arg(&data);
    c.env("PDF2MD_RELEASE_BASE", format!("{base}/release")).env("PDF2MD_HF_BASE", format!("{base}/hf"));
    for (k, v) in extra {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn installed(root: &Path) -> Vec<bool> {
    let bin = root.join("data/bin");
    [EXE, "models/det.onnx", "models/rec.onnx", "models/inference.yml"].map(|f| bin.join(f).is_file()).to_vec()
}

#[test]
fn script_versions_match_cargo() {
    let v = env!("CARGO_PKG_VERSION");
    let sh = std::fs::read_to_string(Path::new(SCRIPTS).join("install.sh")).unwrap();
    assert!(sh.contains(&format!("\nVERSION={v} ")), "VERSION in install.sh");
    let ps = std::fs::read_to_string(Path::new(SCRIPTS).join("install.ps1")).unwrap();
    assert!(ps.contains(&format!("\n$Version = '{v}'")), "$Version in install.ps1");
}

#[test]
fn fresh_install_then_up_to_date_without_network() {
    let root = fixture("fresh");
    let (base, hits) = serve(root.clone());
    let o = install(&root, &base, &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(installed(&root), [true; 4]);
    let v = Command::new(root.join("data/bin").join(EXE)).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&v.stdout).trim(), format!("pdf2md {}", env!("CARGO_PKG_VERSION")));

    let before = hits.load(Ordering::SeqCst);
    let o = install(&root, &base, &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(hits.load(Ordering::SeqCst), before, "an up-to-date install must not download anything");
}

#[test]
fn binary_checksum_mismatch_installs_nothing() {
    let root = fixture("binsum");
    std::fs::write(root.join("release/SHA256SUMS"), format!("{}  pdf2md-{TARGET}.{ARCHIVE_EXT}\n", "0".repeat(64))).unwrap();
    let (base, _) = serve(root.clone());
    let o = install(&root, &base, &[]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("checksum mismatch"));
    assert!(!root.join("data/bin").join(EXE).exists());
}

#[test]
fn model_checksum_mismatch_keeps_model_out() {
    let root = fixture("modelsum");
    let yml = root.join(format!("hf/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/{REC_COMMIT}/inference.yml"));
    std::fs::write(&yml, "tampered").unwrap();
    let (base, _) = serve(root.clone());
    let o = install(&root, &base, &[]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("checksum mismatch for inference.yml"));
    assert!(!root.join("data/bin/models/inference.yml").exists());
}

#[test]
fn unsupported_platform_installs_nothing() {
    let root = fixture("unsupported");
    let (base, hits) = serve(root.clone());
    let o = install(&root, &base, &[("PDF2MD_OS", "Darwin"), ("PDF2MD_ARCH", "arm64")]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("unsupported platform"));
    assert_eq!(installed(&root), [false; 4]);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}
