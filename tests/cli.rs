//! `pdf2md` 바이너리를 실제로 실행하는 통합 테스트. fixture PDF는 대부분 테스트 중에 pdf_oxide writer로 만든다.
//! OCR이 필요한 테스트는 모델 파일이 있어야 한다. `PDF2MD_MODEL_DIR`을 지정한다(README 참고).

use pdf_oxide::api::Pdf;
use pdf_oxide::geometry::Rect;
use pdf_oxide::writer::{DocumentBuilder, PageSize, Table, TableCell};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const KO_SCAN: &[u8] = include_bytes!("fixtures/ko_scan.png");
const KO_TABLE_SCAN: &[u8] = include_bytes!("fixtures/ko_table_scan.png");
const SCAN_TOP: &[u8] = include_bytes!("fixtures/scan_top.png");
const SCAN_BOTTOM: &[u8] = include_bytes!("fixtures/scan_bottom.png");
const SCAN_BODY: &[u8] = include_bytes!("fixtures/scan_body.png");
const FIG_A: &[u8] = include_bytes!("fixtures/fig_a.png");
const FIG_B: &[u8] = include_bytes!("fixtures/fig_b.png");

/// 테스트 전용: fixture PDF 한 페이지의 종류. `build`가 이 목록대로 PDF를 만든다.
enum FixturePage<'a> {
    Text(&'a str),
    Table,
    TextAndFigure(&'a str),
    Scan(&'a [u8]),
    /// 위아래 두 장의 이미지 띠로 저장된 스캔 페이지 하나.
    SplitScan,
    /// 큰 스캔 이미지와, 이미지 밖 text layer에 있는 머리글과 쪽 번호.
    ScanWithHeader,
    /// 본문 한 줄과, 픽셀 크기는 같지만 내용이 다른 그림 두 개. 그림 폭은 `size` pt.
    TwoFigures(f32),
    /// 서식 표시 테스트용: 큰 굵은 제목, 굵은 본문 한 줄, 보통 본문 한 줄.
    HeadingAndBold,
}

fn build(pages: &[FixturePage]) -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    for page in pages {
        let p = b.page(PageSize::A4);
        match page {
            FixturePage::Text(t) => p.at(72.0, 760.0).text(t).done(),
            FixturePage::Table => {
                let row = |c: [&str; 3]| c.map(TableCell::text).to_vec();
                let t = Table::new(vec![
                    row(["Name", "Qty", "Price"]),
                    row(["Apple", "3", "1.20"]),
                    row(["Pear", "5", "0.80"]),
                ]);
                p.at(72.0, 760.0).table(t.with_header_row()).done()
            }
            FixturePage::TextAndFigure(t) => p
                .at(72.0, 760.0)
                .text(t)
                .image_from_bytes(KO_SCAN, Rect::new(72.0, 300.0, 120.0, 30.0))
                .unwrap()
                .done(),
            FixturePage::Scan(png) => p
                .image_from_bytes(png, Rect::new(0.0, 400.0, 595.0, 200.0))
                .unwrap()
                .done(),
            FixturePage::SplitScan => p
                .image_from_bytes(SCAN_TOP, Rect::new(0.0, 571.0, 595.0, 149.0))
                .unwrap()
                .image_from_bytes(SCAN_BOTTOM, Rect::new(0.0, 422.0, 595.0, 149.0))
                .unwrap()
                .done(),
            FixturePage::ScanWithHeader => p
                .at(72.0, 800.0)
                .text("ACME REPORT")
                .image_from_bytes(SCAN_BODY, Rect::new(20.0, 120.0, 555.0, 640.0))
                .unwrap()
                .at(280.0, 60.0)
                .text("Page 12")
                .done(),
            FixturePage::TwoFigures(w) => {
                let h = w * 0.3;
                p.at(72.0, 780.0)
                    .text("Two figures follow.")
                    .image_from_bytes(FIG_A, Rect::new(72.0, 700.0 - h, *w, h))
                    .unwrap()
                    .image_from_bytes(FIG_B, Rect::new(72.0, 650.0 - 2.0 * h, *w, h))
                    .unwrap()
                    .done()
            }
            FixturePage::HeadingAndBold => p
                .at(72.0, 760.0)
                .heading(1, "Quarterly Summary")
                .font("Helvetica-Bold", 12.0)
                .text("Revenue grew strongly")
                .font("Helvetica", 12.0)
                .text("Costs stayed flat")
                .done(),
        };
    }
    b.build().unwrap()
}

/// 테스트마다 새 디렉터리를 쓴다. 병렬로 도는 테스트끼리 파일이 섞이지 않게 하려는 것이다.
fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pdf2md-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_pdf(dir: &Path, pages: &[FixturePage]) -> PathBuf {
    let pdf = dir.join("doc.pdf");
    std::fs::write(&pdf, build(pages)).unwrap();
    pdf
}

fn models() -> PathBuf {
    let d = std::env::var_os("PDF2MD_MODEL_DIR")
        .map(PathBuf::from)
        .expect("OCR tests need PDF2MD_MODEL_DIR pointing at det.onnx/rec.onnx/inference.yml (see README)");
    assert!(d.join("rec.onnx").is_file(), "no models in {}", d.display());
    d
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pdf2md"))
        .args(args)
        .output()
        .unwrap()
}

fn run_ok(args: &[&str]) -> (String, String) {
    let o = run(args);
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    );
    assert!(
        o.status.success(),
        "exit {:?}\nstdout: {out}\nstderr: {err}",
        o.status.code()
    );
    (out, err)
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn writes_md_next_to_pdf_with_page_markers_in_order() {
    let d = workdir("markers");
    let pdf = write_pdf(
        &d,
        &[
            FixturePage::Text("alpha one"),
            FixturePage::Text("beta two"),
            FixturePage::Text("gamma three"),
        ],
    );
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    let pos: Vec<usize> = (1..=3)
        .map(|n| md.find(&format!("<!-- p.{n} -->")).expect("marker"))
        .collect();
    assert!(pos[0] < pos[1] && pos[1] < pos[2], "{md}");
    assert!(
        md.find("beta").unwrap() > pos[1] && md.find("beta").unwrap() < pos[2],
        "{md}"
    );
}

#[test]
fn overwrites_existing_md() {
    let d = workdir("overwrite");
    let pdf = write_pdf(&d, &[FixturePage::Text("fresh content")]);
    std::fs::write(d.join("doc.md"), "hand written notes").unwrap();
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(
        md.contains("fresh content") && !md.contains("hand written"),
        "{md}"
    );
}

#[test]
fn figures_become_placeholders_without_image_files() {
    let d = workdir("figure");
    let pdf = write_pdf(
        &d,
        &[
            FixturePage::Text("intro"),
            FixturePage::TextAndFigure("see figure below"),
        ],
    );
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("[image p.2]"), "{md}");
    assert!(!md.contains("[image p.1]"), "{md}");
    let files: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        files.len(),
        2,
        "only doc.pdf and doc.md expected: {files:?}"
    );
}

#[test]
fn native_table_is_plain_rows() {
    let d = workdir("table");
    let pdf = write_pdf(&d, &[FixturePage::Table]);
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    // 칸은 공백으로 구분해서 한 줄에 둔다. GFM 표나 HTML 표로 내지 않는다.
    // pdf_oxide는 표 앞에 칸이 붙은 같은 글자("Apple31.20...")를 한 번 더 낸다(README 알려진 한계).
    assert!(
        md.lines()
            .any(|l| l.split_whitespace().eq(["Apple", "3", "1.20"])),
        "{md}"
    );
    assert!(!md.lines().any(|l| l.trim_start().starts_with('|')), "{md}");
    assert!(!md.contains("<table"), "{md}");
}

#[test]
fn formatting_marks_are_not_emitted() {
    let d = workdir("styled");
    let pdf = write_pdf(&d, &[FixturePage::HeadingAndBold]);
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    for t in [
        "Quarterly Summary",
        "Revenue grew strongly",
        "Costs stayed flat",
    ] {
        assert!(md.contains(t), "missing {t}: {md}");
    }
    assert!(!md.lines().any(|l| l.starts_with('#')), "{md}");
    assert!(!md.contains("**"), "{md}");
}

/// 객체 번호 순서대로 PDF를 쓰고 xref를 붙인다. 객체 1이 Catalog다.
fn raw_pdf(objs: &[(usize, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = vec![0; objs.len()];
    for (i, (n, body)) in objs.iter().enumerate() {
        assert_eq!(*n, i + 1, "objects must be numbered 1..");
        offs[i] = out.len();
        out.extend(format!("{n} 0 obj\n").bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .bytes(),
    );
    out
}

fn stream(data: &str) -> Vec<u8> {
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len()).into_bytes()
}

/// calibre 전자책처럼 y축을 뒤집은 `FontMatrix [0.001 0 0 -0.001 0 0]`의 Type 3 글꼴로 두 줄을 그린다.
/// 글자 모양(CharProcs)은 빈 도형이다. 텍스트 추출은 ToUnicode와 위치만 쓴다.
fn type3_pdf(lines: &[&str]) -> Vec<u8> {
    let mut chars: Vec<char> = lines.concat().chars().collect();
    chars.sort();
    chars.dedup();
    let code = |c: char| 33 + chars.iter().position(|&x| x == c).unwrap();
    let width = |c: char| if c == ' ' { 300 } else { 550 };
    let hex = |s: &str| {
        s.chars()
            .map(|c| format!("{:02X}", code(c)))
            .collect::<String>()
    };
    let first = 7; // CharProcs 객체 시작 번호
    let names: Vec<String> = chars
        .iter()
        .enumerate()
        .map(|(i, c)| format!("/g{} {} 0 R", code(*c), first + i))
        .collect();
    let diffs: Vec<String> = chars.iter().map(|c| format!("/g{}", code(*c))).collect();
    let widths: Vec<String> = chars.iter().map(|c| width(*c).to_string()).collect();
    let bfchar: Vec<String> = chars
        .iter()
        .map(|c| format!("<{:02X}> <{:04X}>", code(*c), *c as u32))
        .collect();
    let font = format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 -0.001 0 0] \
         /CharProcs << {} >> /Encoding << /Type /Encoding /Differences [33 {}] >> \
         /FirstChar 33 /LastChar {} /Widths [{}] /ToUnicode 5 0 R /Resources << >> >>",
        names.join(" "),
        diffs.join(" "),
        32 + chars.len(),
        widths.join(" ")
    );
    let cmap = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T3 def \
         1 begincodespacerange <00> <FF> endcodespacerange {} beginbfchar\n{}\nendbfchar endcmap \
         CMapName currentdict /CMap defineresource pop end end",
        chars.len(),
        bfchar.join("\n")
    );
    // 글꼴 행렬이 y를 뒤집으므로 텍스트 행렬도 y를 뒤집어 글자가 똑바로 서게 한다.
    let content: String = lines
        .iter()
        .enumerate()
        .map(|(i, l)| format!("1 0 0 -1 72 {} Tm <{}> Tj ", 700 - 30 * i, hex(l)))
        .collect();
    let mut objs = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 6 0 R >>".to_vec()),
        (4, font.into_bytes()),
        (5, stream(&cmap)),
        (6, stream(&format!("BT /F1 20 Tf {content}ET"))),
    ];
    for (i, c) in chars.iter().enumerate() {
        objs.push((first + i, stream(&format!("{} 0 d0", width(*c)))));
    }
    raw_pdf(&objs)
}

#[test]
fn type3_flipped_font_matrix_keeps_word_order() {
    let d = workdir("type3");
    let pdf = d.join("doc.pdf");
    std::fs::write(
        &pdf,
        type3_pdf(&["first line words here", "second line words here"]),
    )
    .unwrap();
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    // 같은 문단의 두 줄은 한 줄로 이어질 수 있다. 단어 순서와 단어 사이 공백만 본다.
    let body: Vec<&str> = md
        .lines()
        .filter(|l| !l.starts_with("<!--"))
        .flat_map(str::split_whitespace)
        .collect();
    assert_eq!(
        body.join(" "),
        "first line words here second line words here",
        "{md}"
    );
}

#[test]
fn repeated_headers_are_kept() {
    let d = workdir("headers");
    let pdf = write_pdf(
        &d,
        &[
            FixturePage::Text("ACME REPORT page body one"),
            FixturePage::Text("ACME REPORT page body two"),
        ],
    );
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert_eq!(md.matches("ACME REPORT").count(), 2, "{md}");
}

#[test]
fn page_range_selects_and_validates() {
    let d = workdir("range");
    let pages: Vec<String> = (1..=10).map(|n| format!("page body {n}")).collect();
    let pages: Vec<FixturePage> = pages.iter().map(|t| FixturePage::Text(t)).collect();
    let pdf = write_pdf(&d, &pages);
    let md = || std::fs::read_to_string(d.join("doc.md")).unwrap();

    run_ok(&[s(&pdf), "--pages", "3-5"]);
    let m = md();
    assert_eq!(m.matches("<!-- p.").count(), 3, "{m}");
    assert!(
        m.contains("<!-- p.3 -->") && m.contains("<!-- p.5 -->") && !m.contains("<!-- p.6 -->")
    );

    run_ok(&[s(&pdf)]);
    assert_eq!(md().matches("<!-- p.").count(), 10);

    std::fs::remove_file(d.join("doc.md")).unwrap();
    let o = run(&[s(&pdf), "--pages", "8-12"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("exceeds page count"));
    assert!(!d.join("doc.md").exists());
}

#[test]
fn unreadable_pdfs_fail_without_output() {
    let d = workdir("unreadable");
    let corrupt = d.join("corrupt.pdf");
    std::fs::write(&corrupt, b"%PDF-1.7\nthis is not a pdf").unwrap();
    let o = run(&[s(&corrupt)]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!d.join("corrupt.md").exists());

    let plain = write_pdf(&d, &[FixturePage::Text("secret")]);
    let locked = d.join("locked.pdf");
    Pdf::open(&plain)
        .unwrap()
        .save_encrypted(&locked, "userpw", "ownerpw")
        .unwrap();
    let o = run(&[s(&locked)]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!d.join("locked.md").exists());
}

#[test]
fn mixed_digital_and_scanned_pages() {
    let d = workdir("mixed");
    let pdf = write_pdf(
        &d,
        &[
            FixturePage::Text("digital page text"),
            FixturePage::Scan(KO_SCAN),
        ],
    );
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("digital page text"), "{md}");
    assert!(md.contains("객체지향") && md.contains("책임"), "{md}");
    assert!(
        out.contains("OCR pages: 1") && out.contains("- p.2 confidence"),
        "{out}"
    );
}

#[test]
fn scanned_table_is_plain_text() {
    let d = workdir("scantable");
    let pdf = write_pdf(&d, &[FixturePage::Scan(KO_TABLE_SCAN)]);
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    for cell in ["외환보유액", "4,431.6", "유가증권", "3,824.3"] {
        assert!(md.contains(cell), "missing {cell}: {md}");
    }
    assert!(
        !md.lines().any(|l| l.starts_with('|')) && !md.contains("<table"),
        "{md}"
    );
}

#[test]
fn digital_pdf_reports_no_ocr_pages() {
    let d = workdir("noocr");
    let pdf = write_pdf(&d, &[FixturePage::Text("just text")]);
    let (out, _) = run_ok(&[s(&pdf), "--models", "/nonexistent"]);
    assert!(out.contains("OCR pages: none"), "{out}");
}

#[test]
fn missing_models_fail_only_when_ocr_needed() {
    let d = workdir("nomodels");
    let pdf = write_pdf(&d, &[FixturePage::Scan(KO_SCAN)]);
    let o = run(&[s(&pdf), "--models", "/nonexistent"]);
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("det.onnx") && err.contains("README"), "{err}");
    assert!(!d.join("doc.md").exists());
}

/// 새 네트워크 namespace 안에서 실행한다(loopback조차 내려가 있음).
/// 네트워크에 접근하려는 시도가 있으면 변환이 실패한다.
#[cfg(target_os = "linux")]
#[test]
fn converts_scan_with_network_disabled() {
    let d = workdir("offline");
    let pdf = write_pdf(&d, &[FixturePage::Scan(KO_SCAN)]);
    let o = Command::new("unshare")
        .args([
            "-rn",
            env!("CARGO_BIN_EXE_pdf2md"),
            s(&pdf),
            "--models",
            s(&models()),
        ])
        .output()
        .expect("unshare (util-linux) is required for this test");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        std::fs::read_to_string(d.join("doc.md"))
            .unwrap()
            .contains("객체지향")
    );
}

#[test]
fn scan_split_into_two_images_is_read_completely() {
    let d = workdir("splitscan");
    let pdf = write_pdf(&d, &[FixturePage::SplitScan]);
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(
        md.contains("위쪽 조각의 첫 번째") && md.contains("아래쪽 조각의 두 번째"),
        "{md}"
    );
    assert!(
        !md.contains("[image p.1]"),
        "scan strips are not figures: {md}"
    );
    assert!(
        out.contains("- p.1 confidence") && !out.contains("partial"),
        "{out}"
    );
}

#[test]
fn scan_keeps_text_layer_header_and_page_number() {
    let d = workdir("headerscan");
    let pdf = write_pdf(&d, &[FixturePage::ScanWithHeader]);
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    for want in ["ACME REPORT", "Page 12", "스캔 본문은 이미지 안에"] {
        assert!(md.contains(want), "missing {want}: {md}");
    }
}

#[test]
fn hybrid_page_ocrs_every_image_region() {
    let d = workdir("hybrid");
    let pdf = write_pdf(&d, &[FixturePage::TwoFigures(400.0)]);
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(
        out.contains("OCR pages: 1"),
        "page should be OCR'd as hybrid: {out}\n{md}"
    );
    for want in ["Two figures follow.", "첫째 그림", "둘째 그림"] {
        assert!(md.contains(want), "missing {want}: {md}");
    }
}

#[test]
fn same_size_figures_get_one_placeholder_each() {
    let d = workdir("twofigs");
    let pdf = write_pdf(&d, &[FixturePage::TwoFigures(200.0)]);
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert_eq!(md.matches("[image p.1]").count(), 2, "{md}");
}

#[cfg(unix)]
#[test]
fn accepts_non_utf8_file_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let d = workdir("nonutf8");
    let name = OsStr::from_bytes(b"r\xffport.pdf");
    let pdf = d.join(name);
    std::fs::write(&pdf, build(&[FixturePage::Text("bytes in the name")])).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_pdf2md"))
        .arg(&pdf)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "exit {:?}: {}",
        o.status.code(),
        String::from_utf8_lossy(&o.stderr)
    );
    let md = std::fs::read_to_string(d.join(OsStr::from_bytes(b"r\xffport.md"))).unwrap();
    assert!(md.contains("bytes in the name"));
}

#[test]
fn write_failure_has_its_own_exit_code() {
    let d = workdir("writefail");
    let pdf = write_pdf(&d, &[FixturePage::Text("fine pdf")]);
    std::fs::create_dir(d.join("doc.md")).unwrap();
    let o = run(&[s(&pdf)]);
    assert_eq!(o.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot write"));
}

/// 글자를 벡터 윤곽선으로 그린 페이지: text layer, 폰트, 이미지가 모두 없다.
/// Helvetica 텍스트 PDF("VECTOR OUTLINE TEXT" / "SCAN 2026 PAGE")를
/// `gs -dNoOutputFonts -sDEVICE=pdfwrite`로 변환해서 만들었다.
const VECTOR_TEXT: &[u8] = include_bytes!("fixtures/vector_text.pdf");

#[test]
fn scan_with_crop_box_is_read() {
    let d = workdir("cropbox");
    let plain = write_pdf(&d, &[FixturePage::Scan(KO_SCAN)]);
    let cropped = d.join("cropped.pdf");
    let mut pdf = Pdf::open(&plain).unwrap();
    // MediaBox보다 작고 원점도 다르다. 스캔 글자(x 약 30..500pt, y 400..600)는 전부 보인다.
    pdf.set_page_crop_box(0, [20.0, 300.0, 575.0, 700.0])
        .unwrap();
    pdf.save(&cropped).unwrap();
    let (out, _) = run_ok(&[s(&cropped), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("cropped.md")).unwrap();
    assert!(
        md.contains("객체지향") && md.contains("책임"),
        "{out}\n{md}"
    );
}

#[test]
fn scan_split_into_small_tiles_is_read() {
    // 1200x300 px 스캔을 100x75 px 타일 48개로 자른다. 타일 하나는 페이지 면적의 약 0.37%다.
    let scan = image::load_from_memory(KO_SCAN).unwrap();
    let (cols, rows) = (12u32, 4u32);
    let (tw, th) = (scan.width() / cols, scan.height() / rows);
    let (x0, top, w_pt) = (0.0f32, 600.0f32, 595.0f32);
    let scale = w_pt / scan.width() as f32;
    let mut b = DocumentBuilder::new();
    let mut page = b.page(PageSize::A4);
    for r in 0..rows {
        for c in 0..cols {
            let mut png = std::io::Cursor::new(Vec::new());
            scan.crop_imm(c * tw, r * th, tw, th)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            let rect = Rect::new(
                x0 + (c * tw) as f32 * scale,
                top - ((r + 1) * th) as f32 * scale,
                tw as f32 * scale,
                th as f32 * scale,
            );
            page = page.image_from_bytes(png.get_ref(), rect).unwrap();
        }
    }
    page.done();
    let d = workdir("tiles");
    let pdf = d.join("doc.pdf");
    std::fs::write(&pdf, b.build().unwrap()).unwrap();
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(
        md.contains("객체지향") && md.contains("책임"),
        "{out}\n{md}"
    );
    assert!(out.contains("- p.1 confidence"), "{out}");
    assert!(md.matches("[image p.1]").count() < 48, "{md}");
}

#[test]
fn vector_outline_text_is_ocrd() {
    let d = workdir("vector");
    let pdf = d.join("doc.pdf");
    std::fs::write(&pdf, VECTOR_TEXT).unwrap();
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("VECTOR") && md.contains("2026"), "{out}\n{md}");
    assert!(out.contains("- p.1 confidence"), "{out}");
}

#[test]
fn prints_version_without_reading_a_pdf() {
    let o = run(&["--version"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        format!("pdf2md {}\n", env!("CARGO_PKG_VERSION"))
    );
}
