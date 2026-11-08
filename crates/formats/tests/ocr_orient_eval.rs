//! Orientation check on real models (`plaindoc_ocr` orient.rs): turned fixture pages read as upright, sparse
//! pages stay, `/Rotate` and the check do not turn a page twice. The turned synthetic scan set is
//! `OCR_EVAL_TURN` in `ocr_eval.rs`.
//!
//! ```sh
//! cargo test --release -p plaindoc-formats --features ocr-paddle --test ocr_orient_eval -- --ignored
//! ```

mod pdf_common;

use std::sync::{Arc, OnceLock};

use plaindoc_core::{Format, Input, OcrMode, ParseOptions, Parser};
use plaindoc_formats::Registry;
use plaindoc_formats::ocr::{ImageParser, OcrConfig, paddle_engine};
use plaindoc_ocr::{OcrEngine, OcrParams, ocr_image};

fn engine() -> Arc<dyn OcrEngine> {
    static E: OnceLock<Arc<dyn OcrEngine>> = OnceLock::new();
    E.get_or_init(|| paddle_engine(None, None)).clone()
}

/// Levenshtein over chars of whitespace-normalised text / length of `gt`.
fn cer(gt: &str, ocr: &str) -> f64 {
    let n = |s: &str| -> Vec<char> {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .collect()
    };
    let (a, b) = (n(gt), n(ocr));
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()] as f64 / a.len().max(1) as f64
}

fn parse_text_bytes(bytes: Vec<u8>, f: Format) -> String {
    let d = ImageParser::new(OcrConfig::default_engine())
        .parse(Input::Bytes(bytes), f, &ParseOptions::default())
        .unwrap();
    pdf_common::doc_text(&d)
}

fn render_png(pdf: Vec<u8>, page: usize, dpi: u32) -> Vec<u8> {
    use pdf_oxide::PdfDocument;
    use pdf_oxide::rendering::{PageRenderer, RenderOptions};
    let doc = PdfDocument::from_bytes(pdf).unwrap();
    PageRenderer::new(RenderOptions::with_dpi(dpi))
        .render_page(&doc, page)
        .unwrap()
        .data
}

/// `img` turned clockwise by `turn` degrees.
fn turned(img: &image::RgbImage, turn: u32) -> image::RgbImage {
    match turn {
        90 => image::imageops::rotate90(img),
        180 => image::imageops::rotate180(img),
        270 => image::imageops::rotate270(img),
        _ => img.clone(),
    }
}

#[test]
#[ignore = "needs OCR models"]
fn turned_fixture_pages_read_as_upright() {
    // The in-repo scan and the builder page rendered at 200 dpi, turned clockwise by 90/180/270 before the
    // ImageParser: the orientation check must undo the turn (CER within 0.01 of the upright page).
    let scan = std::fs::read(pdf_common::fixture("scan.pdf")).unwrap();
    for (name, pdf) in [
        ("scan.pdf", scan),
        ("builder", pdf_common::builder_pdf_bytes(1)),
    ] {
        let png = render_png(pdf, 0, 200);
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        let text = |turn: u32| {
            let mut out = std::io::Cursor::new(Vec::new());
            turned(&img, turn)
                .write_to(&mut out, image::ImageFormat::Png)
                .unwrap();
            parse_text_bytes(out.into_inner(), Format::Png)
        };
        let upright = text(0);
        for turn in [90, 180, 270] {
            let t = text(turn);
            let c = cer(&upright, &t);
            eprintln!("{name} turned {turn}: CER vs upright {c:.4}");
            assert!(
                c <= 0.01,
                "{name} turned {turn}: CER {c:.4}\n{upright}\n---\n{t}"
            );
        }
    }
}

#[test]
#[ignore = "needs OCR models"]
fn sparse_pages_are_not_turned() {
    // A barcode (vertical bars) and a single heading line turned by 90°: too little evidence, no turn;
    // the vertical line is still read (vertical boxes are read both ways).
    let mut bars = image::RgbImage::from_pixel(1200, 800, image::Rgb([255; 3]));
    let mut x = 200u32;
    for i in 0..60u32 {
        let w = 2 + (i * 7 + 3) % 5;
        for y in 300..500 {
            for dx in 0..w {
                bars.put_pixel(x + dx, y, image::Rgb([0; 3]));
            }
        }
        x += w + 2 + (i * 5) % 4;
    }
    let png = render_png(pdf_common::builder_pdf_bytes(1), 0, 200);
    let page = image::load_from_memory(&png).unwrap().to_rgb8();
    let heading = image::imageops::crop_imm(&page, 0, 150, page.width(), 110).to_image();
    for (name, img) in [("barcode", bars), ("one line", turned(&heading, 90))] {
        let p = ocr_image(&*engine(), &img, &OcrParams::default()).unwrap();
        eprintln!("{name}: rotation {} text {:?}", p.rotation, p.text());
        assert_eq!(p.rotation, 0, "{name}");
    }
}

/// One-page PDF of `content` in Helvetica `/F1`, MediaBox `[0 0 w h]`, `/Rotate rotate`.
fn one_page_pdf([w, h]: [u32; 2], rotate: u32, content: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Rotate {rotate} \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Upright evaluation matrix (as the review generator `gen.py`): `cols` ruled columns whose long headers
/// are turned by 90° (`up`: read bottom-to-top, else top-to-bottom), `rows` rows labelled `P1..` with
/// one-glyph cells, an optional title. The header area is most of the text box area.
fn matrix_pdf(
    landscape: bool,
    cols: usize,
    rows: usize,
    up: bool,
    title: bool,
    cell: &[&str],
) -> Vec<u8> {
    const HEAD: [&str; 12] = [
        "Certificate of conformity",
        "Delivery dates are met",
        "Packing and labelling quality",
        "Complete set of documents",
        "Warranty obligations given",
        "Meets the technical task",
        "Timely payment under contract",
        "No customer complaints filed",
        "Licence for the activity",
        "Staff qualification level",
        "Own warehouse premises",
        "Experience with similar work",
    ];
    let (w, h) = if landscape { (842, 595) } else { (595, 842) };
    let (top, hh, cw, rh, x0) = (h as f32 - 70.0, 190.0, 34.0, 20.0, 150.0);
    let mut c = String::new();
    if title {
        c.push_str(&format!(
            "BT /F1 14 Tf 1 0 0 1 50 {} Tm (Supplier evaluation) Tj ET\n",
            h - 40
        ));
    }
    for i in 0..cols {
        let x = x0 + cw * i as f32;
        c.push_str(&format!("{x} {} {cw} {hh} re S\n", top - hh));
        let (tm, tx, ty) = if up {
            ("0 1 -1 0", x + 22.0, top - hh + 8.0)
        } else {
            ("0 -1 1 0", x + 12.0, top - 8.0)
        };
        c.push_str(&format!(
            "BT /F1 11 Tf {tm} {tx} {ty} Tm ({}) Tj ET\n",
            HEAD[i % 12]
        ));
    }
    for r in 0..rows {
        let y = top - hh - rh * (r + 1) as f32;
        c.push_str(&format!("50 {y} {} {rh} re S\n", x0 - 50.0));
        c.push_str(&format!(
            "BT /F1 11 Tf 1 0 0 1 56 {} Tm (P{}) Tj ET\n",
            y + 6.0,
            r + 1
        ));
        for i in 0..cols {
            let x = x0 + cw * i as f32;
            c.push_str(&format!("{x} {y} {cw} {rh} re S\n"));
            let v = cell[(r * 7 + i * 3) % cell.len()];
            c.push_str(&format!(
                "BT /F1 11 Tf 1 0 0 1 {} {} Tm ({v}) Tj ET\n",
                x + 13.0,
                y + 6.0
            ));
        }
    }
    one_page_pdf([w, h], 0, &c)
}

#[test]
#[ignore = "needs OCR models"]
fn upright_matrices_with_turned_headers_are_not_turned() {
    // Review K1: most of the box area is in vertical headers read confidently one way, yet the page is
    // upright; the horizontal minority (title, row labels, cells) reads well at 0°, so no turn.
    let digits = ["1", "2", "3", "4", "5"];
    let plus = ["+", "-", "+"];
    for (name, pdf) in [
        (
            "landscape, up, plus",
            matrix_pdf(true, 12, 4, true, true, &plus),
        ),
        (
            "landscape, up, digits",
            matrix_pdf(true, 12, 6, true, true, &digits),
        ),
        (
            "landscape, down, plus",
            matrix_pdf(true, 12, 3, false, false, &plus),
        ),
        (
            "portrait, up, plus",
            matrix_pdf(false, 9, 8, true, true, &plus),
        ),
    ] {
        let img = image::load_from_memory(&render_png(pdf, 0, 200))
            .unwrap()
            .to_rgb8();
        let p = ocr_image(&*engine(), &img, &OcrParams::default()).unwrap();
        eprintln!("{name}: rotation {} words {}", p.rotation, p.words.len());
        assert_eq!(p.rotation, 0, "{name}");
    }
}

/// One page, MediaBox 612 x 792, `/Rotate rotate`, six Helvetica 14 lines drawn with the text matrix `tm`
/// from `(x, y)`, each next line at `step` (pt).
fn rotate_pdf(rotate: u32, tm: &str, (x, y): (f32, f32), step: (f32, f32)) -> Vec<u8> {
    let lines = [
        "Payment order number forty two for the delivery of goods",
        "Recipient bank account and the purpose of the payment",
        "Total amount due including value added tax and fees",
        "The invoice is valid for twenty four hours after issue",
        "Signature of the person responsible for the supplier",
        "Contract terms and conditions of the public offer apply",
    ];
    let mut content = String::new();
    for (i, l) in lines.iter().enumerate() {
        let (lx, ly) = (x + step.0 * i as f32, y + step.1 * i as f32);
        content.push_str(&format!("BT /F1 14 Tf {tm} {lx} {ly} Tm ({l}) Tj ET\n"));
    }
    one_page_pdf([612, 792], rotate, &content)
}

#[test]
#[ignore = "needs OCR models"]
fn rotate_entry_and_orientation_check_do_not_turn_twice() {
    // `--ocr always` over a text layer, so `ocr_mismatch` compares the OCR words (mapped back through the
    // orientation turn and `/Rotate`) with the layer at its place.
    let mut o = ParseOptions::default();
    o.ocr = OcrMode::Always;
    // (/Rotate 90 + text turned with it: shown upright, no turn) and (/Rotate 90 + upright text in user
    // space: shown sideways, running top-to-bottom, undone by 270°).
    for (pdf, turn) in [
        (
            rotate_pdf(90, "0 1 -1 0", (100.0, 100.0), (20.0, 0.0)),
            None,
        ),
        (
            rotate_pdf(90, "1 0 0 1", (72.0, 700.0), (0.0, -20.0)),
            Some("turned by 270°"),
        ),
    ] {
        let d = Registry::with_defaults()
            .parse(Input::Bytes(pdf), &o)
            .unwrap();
        let ws: Vec<&str> = d
            .sections
            .iter()
            .flat_map(|s| &s.warnings)
            .chain(&d.warnings)
            .map(|w| w.message.as_str())
            .collect();
        eprintln!("{turn:?}: {ws:?}");
        assert!(ws.iter().any(|w| w.starts_with("OCR (")), "OCR ran: {ws:?}");
        match turn {
            Some(t) => assert!(ws.iter().any(|w| w.contains(t)), "{ws:?}"),
            None => assert!(!ws.iter().any(|w| w.contains("turned by")), "{ws:?}"),
        }
        assert!(
            !d.sections
                .iter()
                .flat_map(|s| &s.warnings)
                .any(|w| w.code == Some("ocr_mismatch")),
            "OCR words land on the layer words: {ws:?}"
        );
    }
}

/// Resident set size of this process, KiB (`ps`).
fn rss_kib() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

#[test]
#[ignore = "needs ORIENT_MEM_SET (a directory of page images) and OCR models"]
fn service_mode_memory_reaches_a_plateau() {
    // One engine for the whole run, as a long-lived service: portrait, landscape and turned pages mixed
    // (the directory order). RSS after each page; the peak must stop growing with new raster shapes.
    let dir = std::path::PathBuf::from(std::env::var_os("ORIENT_MEM_SET").expect("ORIENT_MEM_SET"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    let e = engine();
    let mut peak = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let img = image::ImageReader::open(f)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .to_rgb8();
        let p = ocr_image(&*e, &img, &OcrParams::default()).unwrap();
        let rss = rss_kib() / 1024;
        peak.push(rss);
        eprintln!(
            "{i:3} {:?} {}x{} rotation {} rss {rss} MB",
            f.file_name().unwrap(),
            img.width(),
            img.height(),
            p.rotation
        );
    }
    let half = peak.len() / 2;
    let (a, b) = (
        peak[..half].iter().max().unwrap(),
        peak[half..].iter().max().unwrap(),
    );
    eprintln!("peak first half {a} MB, second half {b} MB");
    assert!(
        *b as f64 <= *a as f64 * 1.1,
        "RSS keeps growing: {a} -> {b} MB"
    );
}
