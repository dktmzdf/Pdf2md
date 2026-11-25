use pdf2md::{Error, LOW_CONFIDENCE, convert, parse_range, report};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: pdf2md <input.pdf> [--pages N|A-B] [--models DIR]";

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
    // OsString, not String: file names need not be valid UTF-8 (Linux allows any bytes).
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
    // Written only after a successful conversion, so failures never leave a partial file.
    std::fs::write(&out, &conv.markdown)
        .map_err(|e| Error::Write(format!("cannot write {}: {e}", out.display())))?;
    println!("wrote {}", out.display());
    print!("{}", report(&conv.ocr_pages, LOW_CONFIDENCE));
    Ok(())
}

/// `models/` next to the executable, so a skill folder can ship binary + models together.
fn default_model_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("models")))
        .unwrap_or_else(|| PathBuf::from("models"))
}
