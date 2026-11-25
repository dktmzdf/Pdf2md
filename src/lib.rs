//! PDF -> Markdown conversion with offline Korean OCR (pdf_oxide + tract).

use image::{DynamicImage, RgbaImage};
use pdf_oxide::PdfDocument;
use pdf_oxide::document::PageInfo;
use pdf_oxide::converters::ConversionOptions;
use pdf_oxide::extractors::images::PdfImage;
use pdf_oxide::ocr::{OcrConfig, OcrEngine, PageType, detect_page_type};
use pdf_oxide::rendering::{RenderOptions, render_page};
use std::fmt;
use std::path::{Path, PathBuf};

/// OCR pages whose mean confidence is below this are flagged as low (design D6).
pub const LOW_CONFIDENCE: f32 = 0.90;
/// Files expected in the model directory (design D8).
pub const MODEL_FILES: [&str; 3] = ["det.onnx", "rec.onnx", "dict.txt"];
/// Render resolution for cropping image regions to OCR (fix-review-findings D1).
const RENDER_DPI: u32 = 200;
/// Images smaller than this share of the page (logos, icons) are not OCR'd.
const MIN_IMAGE_AREA: f32 = 0.005;

pub struct OcrPage {
    pub page: usize,
    pub confidence: f32,
    /// Only part of the page's image content could be OCR'd (render failed).
    pub partial: bool,
}

/// Pixel rectangle on the rendered page: (x0, y0, x1, y1), top-left origin.
type Px = (f32, f32, f32, f32);

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
    /// The PDF was converted but the output file could not be written.
    Write(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) | Error::Unreadable(m) | Error::Ocr(m) | Error::Write(m) => f.write_str(m),
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
        // Text-layer content (headers, page numbers, body) always comes from the text layer.
        let mut body = doc.to_markdown(p, &opts).map_err(|e| unreadable(&e))?.trim_end().to_string();
        let images = doc.extract_images(p).unwrap_or_default();
        let mut figures = images.len();
        if let (Some(engine), PageType::ScannedPage | PageType::HybridPage) = (&engine, &kind) {
            eprintln!("OCR p.{}", p + 1);
            if let Some(o) = ocr_images(&doc, p, &images, engine, body.trim().is_empty())? {
                ocr_pages.push(OcrPage { page: p + 1, confidence: o.confidence, partial: o.partial });
                if !o.text.is_empty() {
                    body = [body, o.text].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("\n\n");
                }
                // On a scanned page the OCR'd images are the scan itself, not figures (design D4).
                if kind == PageType::ScannedPage {
                    figures -= o.images;
                }
            }
        }
        markdown.push_str(&body);
        markdown.push_str("\n\n");
        for _ in 0..figures {
            markdown.push_str(&format!("[image p.{}]\n\n", p + 1));
        }
    }
    Ok(Conversion { markdown, ocr_pages })
}

struct PageOcr {
    text: String,
    confidence: f32,
    partial: bool,
    /// Number of embedded images whose content went into `text`.
    images: usize,
}

/// OCRs what page `p` shows (design D1 of fix-review-findings and fix-scan-ocr-coverage):
/// merge the image regions, drop merged regions under `MIN_IMAGE_AREA`, render the page once
/// and OCR the kept regions top to bottom. A page with no such region but also no text layer
/// (glyphs drawn as vector outlines) is OCR'd whole. Falls back to the largest embedded image
/// (marked partial) when rendering or placement fails. `None` when nothing to OCR.
fn ocr_images(
    doc: &PdfDocument,
    p: usize,
    images: &[PdfImage],
    engine: &OcrEngine,
    no_text_layer: bool,
) -> Result<Option<PageOcr>, Error> {
    let ocr_err = |e: &dyn fmt::Display| Error::Ocr(format!("OCR failed on page {}: {e}", p + 1));
    let largest = |images: &[PdfImage]| -> Result<Option<PageOcr>, Error> {
        let Some(img) = images.iter().max_by_key(|i| i.width() as u64 * i.height() as u64) else {
            return Ok(None);
        };
        let out = engine.ocr_image(&img.to_dynamic_image().map_err(|e| ocr_err(&e))?).map_err(|e| ocr_err(&e))?;
        Ok(Some(PageOcr { text: out.text_in_reading_order(), confidence: out.total_confidence, partial: true, images: 1 }))
    };
    if images.iter().any(|i| i.bbox().is_none()) {
        return largest(images); // an image we cannot place on the page: cannot crop regions
    }
    let info = doc.get_page_info(p).map_err(|e| ocr_err(&e))?;
    let bx = render_box(&info);
    let box_area = (bx.2 - bx.0) * (bx.3 - bx.1);
    // Merge first so a scan cut into many small tiles still forms one large region.
    let regions: Vec<(Px, usize)> = images
        .iter()
        .map(|i| {
            let b = i.bbox().unwrap();
            ((b.x, b.y, b.x + b.width, b.y + b.height), 1)
        })
        .collect();
    let kept: Vec<(Px, usize)> = merge_regions(regions)
        .into_iter()
        .filter(|(r, _)| (r.2 - r.0) * (r.3 - r.1) >= MIN_IMAGE_AREA * box_area)
        .collect();
    if kept.is_empty() && !no_text_layer {
        return Ok(None);
    }
    let rendered = render_page(doc, p, &RenderOptions::with_dpi(RENDER_DPI).as_raw())
        .ok()
        .and_then(|r| RgbaImage::from_raw(r.width, r.height, r.data));
    let Some(page) = rendered else {
        return if kept.is_empty() { Ok(None) } else { largest(images) };
    };

    let dims = page.dimensions();
    let mut crops: Vec<(Px, usize)> = if kept.is_empty() {
        // Nothing but vector drawing on a page without a text layer: OCR the whole page.
        vec![((0.0, 0.0, dims.0 as f32, dims.1 as f32), 0)]
    } else {
        kept.iter()
            .map(|(r, n)| (to_raster((r.0, r.1, r.2 - r.0, r.3 - r.1), bx, info.rotation, dims), *n))
            .collect()
    };
    crops.sort_by(|a, b| a.0.1.total_cmp(&b.0.1));
    let (mut texts, mut weighted, mut spans, mut used) = (Vec::new(), 0.0f32, 0usize, 0usize);
    for (r, n) in crops {
        let (x, y) = (r.0.max(0.0) as u32, r.1.max(0.0) as u32);
        if x >= page.width() || y >= page.height() {
            continue;
        }
        let (w, h) = (((r.2 - r.0) as u32).min(page.width() - x), ((r.3 - r.1) as u32).min(page.height() - y));
        if w == 0 || h == 0 {
            continue;
        }
        used += n;
        let crop = image::imageops::crop_imm(&page, x, y, w, h).to_image();
        let out = engine.ocr_image(&DynamicImage::ImageRgba8(crop)).map_err(|e| ocr_err(&e))?;
        weighted += out.total_confidence * out.spans.len() as f32;
        spans += out.spans.len();
        let t = out.text_in_reading_order();
        if !t.is_empty() {
            texts.push(t);
        }
    }
    let confidence = if spans == 0 { 0.0 } else { weighted / spans as f32 };
    Ok(Some(PageOcr { text: texts.join("\n\n"), confidence, partial: false, images: used }))
}

/// The page area the renderer draws: MediaBox ∩ CropBox, or the MediaBox when they do not
/// overlap. Mirrors pdf_oxide 0.3.78 `rendering::page_render_box` (crate-private); recheck it
/// when upgrading pdf_oxide. Returns `(x0, y0, x1, y1)` in PDF user space.
fn render_box(info: &PageInfo) -> Px {
    let m = &info.media_box;
    let media = (m.x, m.y, m.x + m.width, m.y + m.height);
    let Some(c) = &info.crop_box else { return media };
    let b = (c.x.max(media.0), c.y.max(media.1), (c.x + c.width).min(media.2), (c.y + c.height).min(media.3));
    if b.2 <= b.0 || b.3 <= b.1 { media } else { b }
}

/// Maps a PDF user-space rect `(x, y, w, h)` (bottom-left origin) onto the rendered page
/// raster (top-left origin). `page` is the rendered area `(x0, y0, x1, y1)` from `render_box`.
/// The renderer applies `/Rotate` clockwise, so do the same here; the scale comes from the
/// raster itself because the renderer may lower the DPI.
fn to_raster(r: (f32, f32, f32, f32), page: Px, rot: i32, raster: (u32, u32)) -> Px {
    let rot = rot.rem_euclid(360);
    let (w0, h0) = if rot % 180 == 0 { raster } else { (raster.1, raster.0) };
    let (w0, h0) = (w0 as f32, h0 as f32);
    let s = w0 / (page.2 - page.0);
    let map = |x: f32, y: f32| {
        let (u, v) = ((x - page.0) * s, (page.3 - y) * s);
        match rot {
            90 => (h0 - v, u),
            180 => (w0 - u, h0 - v),
            270 => (v, w0 - u),
            _ => (u, v),
        }
    };
    let (a, b) = (map(r.0, r.1), map(r.0 + r.2, r.1 + r.3));
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

/// Merges overlapping or touching regions (a scan split into strips or tiles becomes one
/// region), adding up how many images each merged region holds.
fn merge_regions(regions: Vec<(Px, usize)>) -> Vec<(Px, usize)> {
    const SLACK: f32 = 1.0;
    let mut merged: Vec<(Px, usize)> = Vec::new();
    for (mut q, mut n) in regions {
        while let Some(i) = merged
            .iter()
            .position(|(m, _)| q.0 <= m.2 + SLACK && m.0 <= q.2 + SLACK && q.1 <= m.3 + SLACK && m.1 <= q.3 + SLACK)
        {
            let (m, k) = merged.remove(i);
            q = (q.0.min(m.0), q.1.min(m.1), q.2.max(m.2), q.3.max(m.3));
            n += k;
        }
        merged.push((q, n));
    }
    merged
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

/// Human-readable OCR page report for stdout (spec: OCR 페이지 보고).
pub fn report(ocr_pages: &[OcrPage], low: f32) -> String {
    if ocr_pages.is_empty() {
        return "OCR pages: none\n".to_string();
    }
    let mut s = format!("OCR pages: {} (text was recognized from images and may contain errors)\n", ocr_pages.len());
    for o in ocr_pages {
        let low_flag = if o.confidence < low { " (low)" } else { "" };
        let partial_flag = if o.partial { " (partial: only part of the page could be read)" } else { "" };
        s.push_str(&format!("- p.{} confidence {:.2}{low_flag}{partial_flag}\n", o.page, o.confidence));
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
    fn report_flags_low_and_partial() {
        let pages = [
            OcrPage { page: 2, confidence: 0.97, partial: false },
            OcrPage { page: 4, confidence: 0.85, partial: false },
            OcrPage { page: 5, confidence: 0.95, partial: true },
        ];
        let r = report(&pages, LOW_CONFIDENCE);
        assert!(r.contains("- p.2 confidence 0.97\n"));
        assert!(r.contains("- p.4 confidence 0.85 (low)\n"));
        assert!(r.contains("- p.5 confidence 0.95 (partial: only part of the page could be read)\n"));
        assert_eq!(report(&[], LOW_CONFIDENCE), "OCR pages: none\n");
    }

    #[test]
    fn raster_mapping_follows_page_rotation() {
        // Letter-ish page 100x200pt rendered at 2 px/pt; rect at bottom-left corner, 10x20pt.
        let media = (0.0, 0.0, 100.0, 200.0);
        let rect = (0.0, 0.0, 10.0, 20.0);
        assert_eq!(to_raster(rect, media, 0, (200, 400)), (0.0, 360.0, 20.0, 400.0));
        // 90 clockwise: the page's bottom-left goes to the raster's top-left.
        assert_eq!(to_raster(rect, media, 90, (400, 200)), (0.0, 0.0, 40.0, 20.0));
        assert_eq!(to_raster(rect, media, 180, (200, 400)), (180.0, 0.0, 200.0, 40.0));
        assert_eq!(to_raster(rect, media, 270, (400, 200)), (360.0, 180.0, 400.0, 200.0));
        assert_eq!(to_raster(rect, media, -90, (400, 200)), to_raster(rect, media, 270, (400, 200)));
    }

    #[test]
    fn raster_mapping_uses_the_rendered_box_origin() {
        // CropBox (50,300)-(545,700) rendered at 1 px/pt: a rect at (60,500) 100x50 sits at
        // x = 60-50 = 10 and y = 700-550 = 150 from the top-left of the raster.
        assert_eq!(to_raster((60.0, 500.0, 100.0, 50.0), (50.0, 300.0, 545.0, 700.0), 0, (495, 400)), (10.0, 150.0, 110.0, 200.0));
    }

    #[test]
    fn render_box_is_media_and_crop_intersection() {
        use pdf_oxide::geometry::Rect;
        let media = Rect::new(0.0, 0.0, 595.0, 842.0);
        let info = |crop| PageInfo { media_box: media, crop_box: crop, rotation: 0 };
        assert_eq!(render_box(&info(None)), (0.0, 0.0, 595.0, 842.0));
        assert_eq!(render_box(&info(Some(Rect::new(50.0, 300.0, 495.0, 400.0)))), (50.0, 300.0, 545.0, 700.0));
        // No overlap: fall back to the media box, like the renderer.
        assert_eq!(render_box(&info(Some(Rect::new(700.0, 900.0, 10.0, 10.0)))), (0.0, 0.0, 595.0, 842.0));
    }

    #[test]
    fn touching_tiles_merge_and_count_images() {
        let tiles: Vec<(Px, usize)> = (0..4).map(|i| ((i as f32 * 10.0, 0.0, (i + 1) as f32 * 10.0, 10.0), 1)).collect();
        let far = ((500.0, 500.0, 505.0, 505.0), 1);
        let merged = merge_regions(tiles.into_iter().chain([far]).collect());
        assert_eq!(merged.len(), 2);
        assert!(merged.contains(&((0.0, 0.0, 40.0, 10.0), 4)));
        assert!(merged.contains(&far));
    }
}
