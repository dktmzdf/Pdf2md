//! PDF를 Markdown으로 변환한다. 스캔 페이지는 오프라인 한국어 OCR(pdf_oxide + tract)로 읽는다.

use image::{DynamicImage, RgbaImage};
use pdf_oxide::PdfDocument;
use pdf_oxide::document::PageInfo;
use pdf_oxide::converters::ConversionOptions;
use pdf_oxide::extractors::images::PdfImage;
use pdf_oxide::ocr::{OcrConfig, OcrEngine, PageType, detect_page_type};
use pdf_oxide::rendering::{RenderOptions, render_page};
use std::fmt;
use std::path::{Path, PathBuf};

/// OCR 평균 신뢰도가 이 값보다 낮은 페이지는 보고에 `(low)`로 표시한다(add-pdf2md-skill design D6).
pub const LOW_CONFIDENCE: f32 = 0.90;
/// 모델 디렉터리에 있어야 하는 파일(add-pdf2md-skill design D8).
pub const MODEL_FILES: [&str; 3] = ["det.onnx", "rec.onnx", "dict.txt"];
/// OCR할 이미지 영역을 잘라낼 때 쓰는 렌더링 해상도(fix-review-findings design D1).
const RENDER_DPI: u32 = 200;
/// 합친 이미지 영역이 페이지 면적의 이 비율보다 작으면(로고, 아이콘) OCR하지 않는다.
const MIN_IMAGE_AREA: f32 = 0.005;

pub struct OcrPage {
    pub page: usize,
    pub confidence: f32,
    /// 렌더링 실패 등으로 페이지 이미지 내용 일부만 OCR했다.
    pub partial: bool,
}

/// 사각형 `(x0, y0, x1, y1)`. 렌더링 결과의 픽셀 좌표(왼쪽 위 원점)나 PDF 좌표(왼쪽 아래 원점)에 쓴다.
type Px = (f32, f32, f32, f32);

pub struct Conversion {
    pub markdown: String,
    pub ocr_pages: Vec<OcrPage>,
}

#[derive(Debug)]
pub enum Error {
    /// 잘못된 CLI 입력. 예: 형식이 틀리거나 문서 범위를 넘는 페이지 범위.
    Usage(String),
    /// 입력 PDF를 열거나 읽을 수 없다(손상, 열람 암호).
    Unreadable(String),
    /// OCR이 필요한데 모델 파일이 없다.
    MissingModels { dir: PathBuf, missing: Vec<&'static str> },
    /// OCR 모델 로드나 실행에 실패했다.
    Ocr(String),
    /// 변환은 했지만 출력 `.md` 파일을 쓸 수 없다.
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

/// `N` 또는 `A-B` 형식의 페이지 범위를 읽는다(1부터 세고, 끝 포함).
pub fn parse_range(s: &str) -> Result<(usize, usize), Error> {
    let bad = || Error::Usage(format!("invalid page range '{s}' (expected N or A-B)"));
    let (a, b) = s.split_once('-').unwrap_or((s, s));
    let (a, b): (usize, usize) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
    if a == 0 || a > b {
        return Err(bad());
    }
    Ok((a, b))
}

/// PDF의 지정 범위(없으면 전체)를 Markdown으로 바꾼다. OCR이 필요한 페이지가 있을 때만 모델을 읽는다.
pub fn convert(pdf: &Path, range: Option<(usize, usize)>, model_dir: &Path) -> Result<Conversion, Error> {
    let unreadable = |e: &dyn fmt::Display| Error::Unreadable(format!("cannot read {}: {e}", pdf.display()));
    let doc = PdfDocument::open(pdf).map_err(|e| unreadable(&e))?;
    // 사용자 암호가 빈 문자열인 암호화 PDF는 읽을 수 있다. 실제 열람 암호가 있을 때만 거부한다.
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
        // 머리글, 쪽 번호, 본문 같은 디지털 글자는 항상 text layer에서 가져온다. OCR하지 않는다.
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
                // 스캔 페이지에서 OCR한 이미지는 스캔 자체라 그림이 아니다. placeholder에서 뺀다(fix-review-findings design D4).
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
    /// `text`에 내용이 들어간 embedded image 수.
    images: usize,
}

/// 페이지 `p`에 보이는 이미지 내용을 OCR한다(fix-review-findings, fix-scan-ocr-coverage design D1~D3).
/// 1. 이미지 영역을 먼저 합치고, `MIN_IMAGE_AREA`보다 작은 영역은 버린다.
/// 2. 페이지를 한 번 렌더링해서 남은 영역을 위에서 아래 순서로 잘라 OCR한다.
/// 3. 남은 영역이 없는데 text layer도 없으면(벡터 도형 글자) 페이지 전체를 OCR한다.
/// 렌더링이나 이미지 위치 파악에 실패하면 가장 큰 이미지만 OCR하고 partial로 표시한다.
/// OCR할 것이 없으면 `None`.
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
        return largest(images); // 위치(bbox)를 모르는 이미지가 있으면 영역을 잘라낼 수 없다
    }
    let info = doc.get_page_info(p).map_err(|e| ocr_err(&e))?;
    let bx = render_box(&info);
    let box_area = (bx.2 - bx.0) * (bx.3 - bx.1);
    // 먼저 합쳐야 작은 타일 여러 개로 나뉜 스캔도 하나의 큰 영역이 되어 면적 기준을 통과한다.
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
        // text layer 없이 벡터 도형만 있는 페이지(도형으로 그린 글자): 페이지 전체를 OCR한다.
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

/// 렌더러가 그리는 페이지 영역: MediaBox ∩ CropBox, 둘이 겹치지 않으면 MediaBox.
/// pdf_oxide 0.3.78의 `rendering::page_render_box`(crate 내부 전용)를 그대로 옮긴 것이라,
/// pdf_oxide를 올릴 때 같은지 다시 확인해야 한다. PDF 좌표의 `(x0, y0, x1, y1)`을 돌려준다.
fn render_box(info: &PageInfo) -> Px {
    let m = &info.media_box;
    let media = (m.x, m.y, m.x + m.width, m.y + m.height);
    let Some(c) = &info.crop_box else { return media };
    let b = (c.x.max(media.0), c.y.max(media.1), (c.x + c.width).min(media.2), (c.y + c.height).min(media.3));
    if b.2 <= b.0 || b.3 <= b.1 { media } else { b }
}

/// PDF 좌표의 사각형 `(x, y, w, h)`(왼쪽 아래 원점)를 렌더링 결과의 픽셀 좌표(왼쪽 위 원점)로 바꾼다.
/// `page`는 `render_box`가 돌려준 렌더링 영역이다. 렌더러가 `/Rotate`를 시계 방향으로 적용하므로
/// 여기서도 똑같이 돌린다. 렌더러가 메모리 한도 때문에 DPI를 낮출 수 있어서, 배율은 렌더링 결과 크기에서 계산한다.
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

/// 겹치거나 맞닿은 영역을 하나로 합친다(띠나 타일로 나뉜 스캔이 한 영역이 됨).
/// 합친 영역마다 들어간 이미지 수를 더해 둔다.
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

/// 모델 디렉터리에서 OCR 엔진을 만든다. 파일이 없으면 다운로드하지 않고 `MissingModels`로 실패한다.
fn load_engine(dir: &Path) -> Result<OcrEngine, Error> {
    let missing: Vec<_> = MODEL_FILES.into_iter().filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        return Err(Error::MissingModels { dir: dir.to_path_buf(), missing });
    }
    // 기본 설정이 고른 모델에 맞다: detection은 긴 변 960, recognition 입력 높이 48(add-pdf2md-skill design D8).
    OcrEngine::new(dir.join("det.onnx"), dir.join("rec.onnx"), dir.join("dict.txt"), OcrConfig::default())
        .map_err(|e| Error::Ocr(format!("cannot load OCR models from {}: {e}", dir.display())))
}

/// 표준 출력용 OCR 페이지 보고문(spec: OCR 페이지 보고). 사람이 바로 읽는 텍스트다.
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
        // 100x200pt 페이지를 2 px/pt로 렌더링. 왼쪽 아래 모서리의 10x20pt 사각형.
        let media = (0.0, 0.0, 100.0, 200.0);
        let rect = (0.0, 0.0, 10.0, 20.0);
        assert_eq!(to_raster(rect, media, 0, (200, 400)), (0.0, 360.0, 20.0, 400.0));
        // 시계 방향 90도: 페이지의 왼쪽 아래가 렌더링 결과의 왼쪽 위로 간다.
        assert_eq!(to_raster(rect, media, 90, (400, 200)), (0.0, 0.0, 40.0, 20.0));
        assert_eq!(to_raster(rect, media, 180, (200, 400)), (180.0, 0.0, 200.0, 40.0));
        assert_eq!(to_raster(rect, media, 270, (400, 200)), (360.0, 180.0, 400.0, 200.0));
        assert_eq!(to_raster(rect, media, -90, (400, 200)), to_raster(rect, media, 270, (400, 200)));
    }

    #[test]
    fn raster_mapping_uses_the_rendered_box_origin() {
        // CropBox (50,300)-(545,700)을 1 px/pt로 렌더링. (60,500)의 100x50 사각형은
        // 렌더링 결과 왼쪽 위에서 x = 60-50 = 10, y = 700-550 = 150에 있다.
        assert_eq!(to_raster((60.0, 500.0, 100.0, 50.0), (50.0, 300.0, 545.0, 700.0), 0, (495, 400)), (10.0, 150.0, 110.0, 200.0));
    }

    #[test]
    fn render_box_is_media_and_crop_intersection() {
        use pdf_oxide::geometry::Rect;
        let media = Rect::new(0.0, 0.0, 595.0, 842.0);
        let info = |crop| PageInfo { media_box: media, crop_box: crop, rotation: 0 };
        assert_eq!(render_box(&info(None)), (0.0, 0.0, 595.0, 842.0));
        assert_eq!(render_box(&info(Some(Rect::new(50.0, 300.0, 495.0, 400.0)))), (50.0, 300.0, 545.0, 700.0));
        // 겹치지 않으면 렌더러처럼 MediaBox를 쓴다.
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
