//! PDF -> Markdown conversion with offline Korean OCR (pdf_oxide + tract).

use pdf_oxide::PdfDocument;
use pdf_oxide::converters::ConversionOptions;
use pdf_oxide::ocr::{OcrConfig, OcrEngine, PageType, detect_page_type};
use std::fmt;
use std::path::{Path, PathBuf};

/// OCR pages whose mean confidence is below this are flagged as low (design D6).
pub const LOW_CONFIDENCE: f32 = 0.90;
/// Files expected in the model directory (design D8).
pub const MODEL_FILES: [&str; 3] = ["det.onnx", "rec.onnx", "dict.txt"];

pub struct OcrPage {
    pub page: usize,
    pub confidence: f32,
}

pub struct Conversion {
    pub markdown: String,
    pub ocr_pages: Vec<OcrPage>,
}

#[derive(Debug)]
pub enum Error {
    /// Bad CLI input such as an invalid or out-of-range page range.
    Usage(String),
    /// The PDF cannot be opened or read (corrupt, password protected).
    Unreadable(String),
    /// OCR is needed but model files are missing.
    MissingModels { dir: PathBuf, missing: Vec<&'static str> },
    Ocr(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) | Error::Unreadable(m) | Error::Ocr(m) => f.write_str(m),
            Error::MissingModels { dir, missing } => write!(
                f,
                "OCR models missing in {}: {}. Place det.onnx, rec.onnx and dict.txt there \
                 (see README), or set PDF2MD_MODEL_DIR / --models.",
                dir.display(),
                missing.join(", ")
            ),
        }
    }
}

/// Parses `N` or `A-B` (1-based, inclusive).
pub fn parse_range(s: &str) -> Result<(usize, usize), Error> {
    let bad = || Error::Usage(format!("invalid page range '{s}' (expected N or A-B)"));
    let (a, b) = s.split_once('-').unwrap_or((s, s));
    let (a, b): (usize, usize) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
    if a == 0 || a > b {
        return Err(bad());
    }
    Ok((a, b))
}

pub fn convert(pdf: &Path, range: Option<(usize, usize)>, model_dir: &Path) -> Result<Conversion, Error> {
    let unreadable = |e: &dyn fmt::Display| Error::Unreadable(format!("cannot read {}: {e}", pdf.display()));
    let doc = PdfDocument::open(pdf).map_err(|e| unreadable(&e))?;
    // An encrypted PDF with an empty user password is readable; only a real open password blocks us.
    if doc.is_encrypted() && !doc.authenticate(b"").unwrap_or(false) {
        return Err(Error::Unreadable(format!("{} is password protected", pdf.display())));
    }
    let count = doc.page_count().map_err(|e| unreadable(&e))?;
    let (first, last) = range.unwrap_or((1, count));
    if last > count {
        return Err(Error::Usage(format!("page range {first}-{last} exceeds page count {count}")));
    }

    let pages: Vec<(usize, PageType)> = (first - 1..last)
        .map(|p| Ok((p, detect_page_type(&doc, p).map_err(|e| unreadable(&e))?)))
        .collect::<Result<_, Error>>()?;
    let needs_ocr = pages.iter().any(|(_, t)| *t != PageType::NativeText);
    let engine = if needs_ocr { Some(load_engine(model_dir)?) } else { None };

    let opts = ConversionOptions::default();
    let mut markdown = String::new();
    let mut ocr_pages = Vec::new();
    for (p, kind) in pages {
        markdown.push_str(&format!("<!-- p.{} -->\n\n", p + 1));
        let native = || doc.to_markdown(p, &opts).map_err(|e| unreadable(&e));
        let body = match (&engine, &kind) {
            (Some(engine), PageType::ScannedPage | PageType::HybridPage) => {
                eprintln!("OCR p.{}", p + 1);
                match ocr_largest_image(&doc, p, engine)? {
                    Some((text, confidence)) => {
                        ocr_pages.push(OcrPage { page: p + 1, confidence });
                        if kind == PageType::HybridPage {
                            format!("{}\n\n{text}", native()?.trim_end())
                        } else {
                            text
                        }
                    },
                    None => native()?,
                }
            },
            _ => native()?,
        };
        markdown.push_str(body.trim_end());
        markdown.push_str("\n\n");
        for _ in 0..image_count(&doc, p, &kind) {
            markdown.push_str(&format!("[image p.{}]\n\n", p + 1));
        }
    }
    Ok(Conversion { markdown, ocr_pages })
}

fn load_engine(dir: &Path) -> Result<OcrEngine, Error> {
    let missing: Vec<_> = MODEL_FILES.into_iter().filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        return Err(Error::MissingModels { dir: dir.to_path_buf(), missing });
    }
    // Defaults match the chosen models: MaxSide 960 det, rec height 48 (design D8).
    OcrEngine::new(dir.join("det.onnx"), dir.join("rec.onnx"), dir.join("dict.txt"), OcrConfig::default())
        .map_err(|e| Error::Ocr(format!("cannot load OCR models from {}: {e}", dir.display())))
}

/// OCRs the page's largest embedded image (the scan). `None` when the page has no image.
fn ocr_largest_image(doc: &PdfDocument, p: usize, engine: &OcrEngine) -> Result<Option<(String, f32)>, Error> {
    let ocr_err = |e: &dyn fmt::Display| Error::Ocr(format!("OCR failed on page {}: {e}", p + 1));
    let images = doc.extract_images(p).map_err(|e| ocr_err(&e))?;
    let Some(img) = images.iter().max_by_key(|i| i.width() as u64 * i.height() as u64) else {
        return Ok(None);
    };
    let out = engine.ocr_image(&img.to_dynamic_image().map_err(|e| ocr_err(&e))?).map_err(|e| ocr_err(&e))?;
    Ok(Some((out.text_in_reading_order(), out.total_confidence)))
}

/// Images that get a placeholder. A scanned page's scan is the page itself, not a figure.
fn image_count(doc: &PdfDocument, p: usize, kind: &PageType) -> usize {
    let images = doc.extract_images(p).unwrap_or_default();
    // An image and its soft mask come back as two entries of the same size; count them once.
    let mut sizes: Vec<_> = images.iter().map(|i| (i.width(), i.height())).collect();
    sizes.sort_unstable();
    sizes.dedup();
    let n = sizes.len();
    if *kind == PageType::ScannedPage { n.saturating_sub(1) } else { n }
}

/// Human-readable OCR page report for stdout (spec: OCR 페이지 보고).
pub fn report(ocr_pages: &[OcrPage], low: f32) -> String {
    if ocr_pages.is_empty() {
        return "OCR pages: none\n".to_string();
    }
    let mut s = format!("OCR pages: {} (text was recognized from images and may contain errors)\n", ocr_pages.len());
    for o in ocr_pages {
        let flag = if o.confidence < low { " (low)" } else { "" };
        s.push_str(&format!("- p.{} confidence {:.2}{flag}\n", o.page, o.confidence));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("3-5").unwrap(), (3, 5));
        assert_eq!(parse_range("4").unwrap(), (4, 4));
        for bad in ["0-2", "5-3", "a", "1-", ""] {
            assert!(parse_range(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn report_flags_low_confidence() {
        let pages = [OcrPage { page: 2, confidence: 0.97 }, OcrPage { page: 4, confidence: 0.85 }];
        let r = report(&pages, LOW_CONFIDENCE);
        assert!(r.contains("- p.2 confidence 0.97\n"));
        assert!(r.contains("- p.4 confidence 0.85 (low)\n"));
        assert_eq!(report(&[], LOW_CONFIDENCE), "OCR pages: none\n");
    }
}
