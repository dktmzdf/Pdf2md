use pdf2md::{Error, LOW_CONFIDENCE, convert, parse_range, report};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: pdf2md <input.pdf> [--pages N|A-B] [--models DIR] | --version";

/// 오류 종류를 종료 코드로 바꾼다. README와 SKILL.md의 종료 코드 표와 같아야 한다.
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(match e {
                Error::Usage(_) => 2,
                Error::Unreadable(_) => 3,
                Error::MissingModels { .. } => 4,
                Error::Ocr(_) => 5,
                Error::Write(_) => 6,
            })
        },
    }
}

fn run() -> Result<(), Error> {
    // String이 아니라 OsString으로 받는다. 파일 이름이 UTF-8이 아닐 수 있다(Linux는 아무 바이트나 허용).
    let mut args = std::env::args_os().skip(1);
    let (mut input, mut range, mut models) = (None, None, None);
    while let Some(a) = args.next() {
        let shown = a.to_string_lossy().into_owned();
        let mut value = || args.next().ok_or_else(|| Error::Usage(format!("{shown} needs a value\n{USAGE}")));
        match a.to_str() {
            Some("--pages") => {
                let v = value()?;
                let v = v.to_str().ok_or_else(|| Error::Usage(format!("invalid page range {v:?}")))?;
                range = Some(parse_range(v)?);
            },
            Some("--models") => models = Some(PathBuf::from(value()?)),
            Some("-h" | "--help") => {
                println!("{USAGE}");
                return Ok(());
            },
            // 설치 스크립트가 plugin 버전과 비교하는 데 쓴다. PDF는 읽지 않는다.
            Some("--version") => {
                println!("pdf2md {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            },
            _ if input.is_none() && !shown.starts_with("--") => input = Some(PathBuf::from(a)),
            _ => return Err(Error::Usage(format!("unexpected argument '{shown}'\n{USAGE}"))),
        }
    }
    let input = input.ok_or_else(|| Error::Usage(USAGE.to_string()))?;
    let models = models
        .or_else(|| std::env::var_os("PDF2MD_MODEL_DIR").map(PathBuf::from))
        .unwrap_or_else(default_model_dir);

    let out = input.with_extension("md");
    let conv = convert(&input, range, &models)?;
    // 변환이 성공한 뒤에만 쓴다. 변환 중 실패하면 출력 파일이 생기지 않는다.
    std::fs::write(&out, &conv.markdown)
        .map_err(|e| Error::Write(format!("cannot write {}: {e}", out.display())))?;
    println!("wrote {}", out.display());
    print!("{}", report(&conv.ocr_pages, LOW_CONFIDENCE));
    Ok(())
}

/// 실행 파일 옆의 `models/`. skill 폴더에 바이너리와 모델을 같이 넣어 배포하기 위해서다.
/// 심볼릭 링크로 실행하면 `current_exe()`가 원본 경로로 풀리니, skill에는 복사본을 둬야 한다.
fn default_model_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("models")))
        .unwrap_or_else(|| PathBuf::from("models"))
}
