use resvg::{tiny_skia, usvg};
use rupora::{export::render_pdf_svg_pages, markdown::render_html_document};

const REFERENCE_MARKDOWN: &str = include_str!("fixtures/export_reference.md");

#[derive(Debug)]
struct PixelBounds {
    painted: usize,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
}

#[test]
fn html_export_matches_the_reference_document_semantics() {
    let html = render_html_document(REFERENCE_MARKDOWN, "Export Reference", false);

    for marker in [
        "<!doctype html>",
        "<h1",
        "<blockquote>",
        "<table>",
        "<pre><code",
        "math-inline",
        "math-display",
    ] {
        assert!(html.contains(marker), "missing export marker: {marker}");
    }
    assert!(!html.contains("<script"));
    assert!(!html.contains("alert("));
}

#[test]
fn pdf_export_rasterizes_to_a_nonblank_unclipped_reference_page() {
    let html = render_html_document(REFERENCE_MARKDOWN, "Export Reference", false);
    let pages = render_pdf_svg_pages(&html).expect("reference PDF should render");
    assert_eq!(pages.len(), 1, "reference document should fit one page");

    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&pages[0], &options).expect("PDF page SVG should parse");
    let size = tree.size().to_int_size();
    assert!((500..=1_500).contains(&size.width()));
    assert!((700..=2_000).contains(&size.height()));

    let mut pixmap =
        tiny_skia::Pixmap::new(size.width(), size.height()).expect("page pixmap should allocate");
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    let bounds = painted_bounds(&pixmap);

    assert!(
        bounds.painted > 8_000,
        "exported page is unexpectedly blank: {bounds:?}"
    );
    assert!(
        bounds.min_x > 10,
        "content touches the left edge: {bounds:?}"
    );
    assert!(
        bounds.min_y > 10,
        "content touches the top edge: {bounds:?}"
    );
    assert!(
        bounds.max_x + 10 < size.width(),
        "content is clipped on the right: {bounds:?}"
    );
    assert!(
        bounds.max_y + 10 < size.height(),
        "content is clipped at the bottom: {bounds:?}"
    );
    assert!(
        bounds.max_y - bounds.min_y > size.height() / 3,
        "reference sections collapsed vertically: {bounds:?}"
    );
}

fn painted_bounds(pixmap: &tiny_skia::Pixmap) -> PixelBounds {
    let width = pixmap.width();
    let mut bounds = PixelBounds {
        painted: 0,
        min_x: width,
        min_y: pixmap.height(),
        max_x: 0,
        max_y: 0,
    };
    for (index, pixel) in pixmap.data().chunks_exact(4).enumerate() {
        let visibly_inked = pixel[3] > 16 && (pixel[0] < 245 || pixel[1] < 245 || pixel[2] < 245);
        if !visibly_inked {
            continue;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        bounds.painted += 1;
        bounds.min_x = bounds.min_x.min(x);
        bounds.min_y = bounds.min_y.min(y);
        bounds.max_x = bounds.max_x.max(x);
        bounds.max_y = bounds.max_y.max(y);
    }
    bounds
}
