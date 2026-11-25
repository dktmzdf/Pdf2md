//! End-to-end tests of the `pdf2md` binary. Fixture PDFs are built at test time with
//! pdf_oxide's writer. OCR tests need the model files: set `PDF2MD_MODEL_DIR` (see README).

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

enum Page<'a> {
    Text(&'a str),
    Table,
    TextAndFigure(&'a str),
    Scan(&'a [u8]),
    /// One scanned page stored as two stacked image strips.
    SplitScan,
    /// A large scan with a header and a page number in the text layer outside the image.
    ScanWithHeader,
    /// A text line plus two equally sized, different figures, placed at `size` points wide.
    TwoFigures(f32),
}

fn build(pages: &[Page]) -> Vec<u8> {
    let mut b = DocumentBuilder::new();
    for page in pages {
        let p = b.page(PageSize::A4);
        match page {
            Page::Text(t) => p.at(72.0, 760.0).text(t).done(),
            Page::Table => {
                let row = |c: [&str; 3]| c.map(TableCell::text).to_vec();
                let t = Table::new(vec![row(["Name", "Qty", "Price"]), row(["Apple", "3", "1.20"]), row(["Pear", "5", "0.80"])]);
                p.at(72.0, 760.0).table(t.with_header_row()).done()
            },
            Page::TextAndFigure(t) => {
                p.at(72.0, 760.0).text(t).image_from_bytes(KO_SCAN, Rect::new(72.0, 300.0, 120.0, 30.0)).unwrap().done()
            },
            Page::Scan(png) => p.image_from_bytes(png, Rect::new(0.0, 400.0, 595.0, 200.0)).unwrap().done(),
            Page::SplitScan => p
                .image_from_bytes(SCAN_TOP, Rect::new(0.0, 571.0, 595.0, 149.0))
                .unwrap()
                .image_from_bytes(SCAN_BOTTOM, Rect::new(0.0, 422.0, 595.0, 149.0))
                .unwrap()
                .done(),
            Page::ScanWithHeader => p
                .at(72.0, 800.0)
                .text("ACME REPORT")
                .image_from_bytes(SCAN_BODY, Rect::new(20.0, 120.0, 555.0, 640.0))
                .unwrap()
                .at(280.0, 60.0)
                .text("Page 12")
                .done(),
            Page::TwoFigures(w) => {
                let h = w * 0.3;
                p.at(72.0, 780.0)
                    .text("Two figures follow.")
                    .image_from_bytes(FIG_A, Rect::new(72.0, 700.0 - h, *w, h))
                    .unwrap()
                    .image_from_bytes(FIG_B, Rect::new(72.0, 650.0 - 2.0 * h, *w, h))
                    .unwrap()
                    .done()
            },
        };
    }
    b.build().unwrap()
}

/// A fresh directory per test so parallel tests never share files.
fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pdf2md-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_pdf(dir: &Path, pages: &[Page]) -> PathBuf {
    let pdf = dir.join("doc.pdf");
    std::fs::write(&pdf, build(pages)).unwrap();
    pdf
}

fn models() -> PathBuf {
    let d = std::env::var_os("PDF2MD_MODEL_DIR")
        .map(PathBuf::from)
        .expect("OCR tests need PDF2MD_MODEL_DIR pointing at det.onnx/rec.onnx/dict.txt (see README)");
    assert!(d.join("rec.onnx").is_file(), "no models in {}", d.display());
    d
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pdf2md")).args(args).output().unwrap()
}

fn run_ok(args: &[&str]) -> (String, String) {
    let o = run(args);
    let (out, err) = (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned());
    assert!(o.status.success(), "exit {:?}\nstdout: {out}\nstderr: {err}", o.status.code());
    (out, err)
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn writes_md_next_to_pdf_with_page_markers_in_order() {
    let d = workdir("markers");
    let pdf = write_pdf(&d, &[Page::Text("alpha one"), Page::Text("beta two"), Page::Text("gamma three")]);
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    let pos: Vec<usize> = (1..=3).map(|n| md.find(&format!("<!-- p.{n} -->")).expect("marker")).collect();
    assert!(pos[0] < pos[1] && pos[1] < pos[2], "{md}");
    assert!(md.find("beta").unwrap() > pos[1] && md.find("beta").unwrap() < pos[2], "{md}");
}

#[test]
fn overwrites_existing_md() {
    let d = workdir("overwrite");
    let pdf = write_pdf(&d, &[Page::Text("fresh content")]);
    std::fs::write(d.join("doc.md"), "hand written notes").unwrap();
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("fresh content") && !md.contains("hand written"), "{md}");
}

#[test]
fn figures_become_placeholders_without_image_files() {
    let d = workdir("figure");
    let pdf = write_pdf(&d, &[Page::Text("intro"), Page::TextAndFigure("see figure below")]);
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("[image p.2]"), "{md}");
    assert!(!md.contains("[image p.1]"), "{md}");
    let files: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(files.len(), 2, "only doc.pdf and doc.md expected: {files:?}");
}

#[test]
fn native_table_is_gfm() {
    let d = workdir("table");
    let pdf = write_pdf(&d, &[Page::Table]);
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.lines().any(|l| l.starts_with('|') && l.contains("Apple")), "{md}");
    assert!(!md.contains("<table"), "{md}");
}

#[test]
fn repeated_headers_are_kept() {
    let d = workdir("headers");
    let pdf = write_pdf(&d, &[Page::Text("ACME REPORT page body one"), Page::Text("ACME REPORT page body two")]);
    run_ok(&[s(&pdf)]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert_eq!(md.matches("ACME REPORT").count(), 2, "{md}");
}

#[test]
fn page_range_selects_and_validates() {
    let d = workdir("range");
    let pages: Vec<String> = (1..=10).map(|n| format!("page body {n}")).collect();
    let pages: Vec<Page> = pages.iter().map(|t| Page::Text(t)).collect();
    let pdf = write_pdf(&d, &pages);
    let md = || std::fs::read_to_string(d.join("doc.md")).unwrap();

    run_ok(&[s(&pdf), "--pages", "3-5"]);
    let m = md();
    assert_eq!(m.matches("<!-- p.").count(), 3, "{m}");
    assert!(m.contains("<!-- p.3 -->") && m.contains("<!-- p.5 -->") && !m.contains("<!-- p.6 -->"));

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
    assert_eq!(o.status.code(), Some(3), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!d.join("corrupt.md").exists());

    let plain = write_pdf(&d, &[Page::Text("secret")]);
    let locked = d.join("locked.pdf");
    Pdf::open(&plain).unwrap().save_encrypted(&locked, "userpw", "ownerpw").unwrap();
    let o = run(&[s(&locked)]);
    assert_eq!(o.status.code(), Some(3), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!d.join("locked.md").exists());
}

#[test]
fn mixed_digital_and_scanned_pages() {
    let d = workdir("mixed");
    let pdf = write_pdf(&d, &[Page::Text("digital page text"), Page::Scan(KO_SCAN)]);
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("digital page text"), "{md}");
    assert!(md.contains("객체지향") && md.contains("책임"), "{md}");
    assert!(out.contains("OCR pages: 1") && out.contains("- p.2 confidence"), "{out}");
}

#[test]
fn scanned_table_is_plain_text() {
    let d = workdir("scantable");
    let pdf = write_pdf(&d, &[Page::Scan(KO_TABLE_SCAN)]);
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    for cell in ["외환보유액", "4,431.6", "유가증권", "3,824.3"] {
        assert!(md.contains(cell), "missing {cell}: {md}");
    }
    assert!(!md.lines().any(|l| l.starts_with('|')) && !md.contains("<table"), "{md}");
}

#[test]
fn digital_pdf_reports_no_ocr_pages() {
    let d = workdir("noocr");
    let pdf = write_pdf(&d, &[Page::Text("just text")]);
    let (out, _) = run_ok(&[s(&pdf), "--models", "/nonexistent"]);
    assert!(out.contains("OCR pages: none"), "{out}");
}

#[test]
fn missing_models_fail_only_when_ocr_needed() {
    let d = workdir("nomodels");
    let pdf = write_pdf(&d, &[Page::Scan(KO_SCAN)]);
    let o = run(&[s(&pdf), "--models", "/nonexistent"]);
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("det.onnx") && err.contains("README"), "{err}");
    assert!(!d.join("doc.md").exists());
}

/// Runs inside a fresh network namespace (no interfaces but loopback down), so any
/// network attempt would fail the conversion.
#[cfg(target_os = "linux")]
#[test]
fn converts_scan_with_network_disabled() {
    let d = workdir("offline");
    let pdf = write_pdf(&d, &[Page::Scan(KO_SCAN)]);
    let o = Command::new("unshare")
        .args(["-rn", env!("CARGO_BIN_EXE_pdf2md"), s(&pdf), "--models", s(&models())])
        .output()
        .expect("unshare (util-linux) is required for this test");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(std::fs::read_to_string(d.join("doc.md")).unwrap().contains("객체지향"));
}

#[test]
fn scan_split_into_two_images_is_read_completely() {
    let d = workdir("splitscan");
    let pdf = write_pdf(&d, &[Page::SplitScan]);
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(md.contains("위쪽 조각의 첫 번째") && md.contains("아래쪽 조각의 두 번째"), "{md}");
    assert!(!md.contains("[image p.1]"), "scan strips are not figures: {md}");
    assert!(out.contains("- p.1 confidence") && !out.contains("partial"), "{out}");
}

#[test]
fn scan_keeps_text_layer_header_and_page_number() {
    let d = workdir("headerscan");
    let pdf = write_pdf(&d, &[Page::ScanWithHeader]);
    run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    for want in ["ACME REPORT", "Page 12", "스캔 본문은 이미지 안에"] {
        assert!(md.contains(want), "missing {want}: {md}");
    }
}

#[test]
fn hybrid_page_ocrs_every_image_region() {
    let d = workdir("hybrid");
    let pdf = write_pdf(&d, &[Page::TwoFigures(400.0)]);
    let (out, _) = run_ok(&[s(&pdf), "--models", s(&models())]);
    let md = std::fs::read_to_string(d.join("doc.md")).unwrap();
    assert!(out.contains("OCR pages: 1"), "page should be OCR'd as hybrid: {out}\n{md}");
    for want in ["Two figures follow.", "첫째 그림", "둘째 그림"] {
        assert!(md.contains(want), "missing {want}: {md}");
    }
}

#[test]
fn same_size_figures_get_one_placeholder_each() {
    let d = workdir("twofigs");
    let pdf = write_pdf(&d, &[Page::TwoFigures(200.0)]);
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
    std::fs::write(&pdf, build(&[Page::Text("bytes in the name")])).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_pdf2md")).arg(&pdf).output().unwrap();
    assert!(o.status.success(), "exit {:?}: {}", o.status.code(), String::from_utf8_lossy(&o.stderr));
    let md = std::fs::read_to_string(d.join(OsStr::from_bytes(b"r\xffport.md"))).unwrap();
    assert!(md.contains("bytes in the name"));
}

#[test]
fn write_failure_has_its_own_exit_code() {
    let d = workdir("writefail");
    let pdf = write_pdf(&d, &[Page::Text("fine pdf")]);
    std::fs::create_dir(d.join("doc.md")).unwrap();
    let o = run(&[s(&pdf)]);
    assert_eq!(o.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot write"));
}
