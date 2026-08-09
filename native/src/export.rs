use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader};
use printpdf::{
    Base64OrRaw, GeneratePdfOptions, PdfDocument, PdfParseErrorSeverity, PdfParseOptions,
    PdfSaveOptions, PdfToSvgOptions, PdfWarnMsg,
};
use tempfile::{Builder as TempFileBuilder, NamedTempFile};

use crate::markdown;

const MAX_LOCAL_IMAGES: usize = 32;
const MAX_LOCAL_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LOCAL_IMAGE_TOTAL_BYTES: usize = 24 * 1024 * 1024;
const MAX_LOCAL_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_LOCAL_IMAGE_TOTAL_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_LOCAL_IMAGE_EDGE: u32 = 8_192;
const MAX_HTML_EXPORT_BYTES: usize = 128 * 1024 * 1024;
const MAX_PDF_HTML_BYTES: usize = 16 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 10_000;
const MAX_PDF_BYTES: usize = 256 * 1024 * 1024;
const MAX_PDF_SVG_TOTAL_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct LocalImage {
    bytes: Vec<u8>,
    mime: &'static str,
}

pub type LocalImages = BTreeMap<String, LocalImage>;

pub fn render_pdf(html: &str) -> Result<Vec<u8>, String> {
    render_pdf_with_images(html, &LocalImages::new())
}

pub fn render_pdf_with_images(html: &str, images: &LocalImages) -> Result<Vec<u8>, String> {
    let document = create_pdf_document(html, images)?;
    let mut save_warnings = Vec::new();
    let bytes = document.save(&PdfSaveOptions::default(), &mut save_warnings);
    reject_pdf_errors("PDF 保存", &save_warnings)?;
    ensure_pdf_output_size(bytes.len())?;
    if bytes.len() < 5 || !bytes.starts_with(b"%PDF-") {
        return Err("PDF 生成器没有返回有效文档".to_owned());
    }
    Ok(bytes)
}

pub fn render_pdf_svg_pages(html: &str) -> Result<Vec<String>, String> {
    // Render the serialized PDF rather than the in-memory document. printpdf's
    // in-memory SVG path embeds each complete source font, which can turn a
    // small CJK page into hundreds of MiB when the platform font is a large TTC.
    // The PDF serializer subsets those fonts first, so parsing the bounded PDF
    // back also makes this visual-regression helper exercise the real artifact.
    let pdf = render_pdf(html)?;
    let mut parse_warnings = Vec::new();
    let mut document = PdfDocument::parse(
        &pdf,
        &PdfParseOptions {
            fail_on_error: true,
        },
        &mut parse_warnings,
    )
    .map_err(|error| format!("无法重新读取生成的 PDF：{error}"))?;
    reject_pdf_errors("PDF 重新读取", &parse_warnings)?;
    if document.pages.is_empty() {
        return Err("重新读取的 PDF 没有任何页面".to_owned());
    }
    ensure_pdf_page_count(document.pages.len())?;
    // Parsed PDF font programs are already subsets. printpdf currently marks
    // them as requiring another subset pass; that second pass fails for valid
    // PDF subsets that omit optional sfnt tables such as `post`. Treating the
    // parsed programs as final also keeps the SVG font payload bounded.
    for font in document.resources.fonts.map.values_mut() {
        font.meta.requires_subsetting = false;
    }

    let mut warnings = Vec::new();
    let mut pages = Vec::with_capacity(document.pages.len());
    let mut total_svg_bytes = 0usize;
    for page in 1..=document.pages.len() {
        let svg = document
            .page_to_svg(page, &PdfToSvgOptions::default(), &mut warnings)
            .ok_or_else(|| format!("无法渲染 PDF 第 {page} 页"))?;
        total_svg_bytes = add_pdf_svg_bytes(total_svg_bytes, svg.len())?;
        pages.push(svg);
    }
    reject_pdf_errors("PDF 页面渲染", &warnings)?;
    Ok(pages)
}

fn create_pdf_document(html: &str, images: &LocalImages) -> Result<PdfDocument, String> {
    ensure_pdf_html_size(html.len())?;
    let mut fonts = BTreeMap::new();
    if let Some(bytes) = system_cjk_font() {
        fonts.insert("RUPORA CJK".to_owned(), Base64OrRaw::Raw(bytes));
    }
    let options = GeneratePdfOptions {
        margin_top: Some(14.0),
        margin_right: Some(16.0),
        margin_bottom: Some(16.0),
        margin_left: Some(16.0),
        show_page_numbers: Some(true),
        ..GeneratePdfOptions::default()
    };
    let (html, images) = unique_pdf_image_resources(html, images);
    let mut warnings = Vec::new();
    let document = PdfDocument::from_html(&html, &images, &fonts, &options, &mut warnings)
        .map_err(|error| format!("HTML 转 PDF 失败：{error}"))?;
    reject_pdf_errors("HTML 转 PDF", &warnings)?;
    if document.pages.is_empty() {
        return Err("HTML 转 PDF 没有生成任何页面".to_owned());
    }
    ensure_pdf_page_count(document.pages.len())?;
    Ok(document)
}

fn unique_pdf_image_resources(
    html: &str,
    images: &LocalImages,
) -> (String, BTreeMap<String, Base64OrRaw>) {
    let mut rewritten = html.to_owned();
    let mut resources = BTreeMap::new();
    for (index, (source, image)) in images.iter().enumerate() {
        let source = escape_html_attribute(source);
        let unique_key = format!("ruporaimage{index}");
        rewritten = rewritten.replace(
            &format!("src=\"{source}\""),
            &format!("src=\"{unique_key}\""),
        );
        resources.insert(unique_key, Base64OrRaw::Raw(image.bytes.clone()));
    }
    (rewritten, resources)
}

fn reject_pdf_errors(stage: &str, warnings: &[PdfWarnMsg]) -> Result<(), String> {
    let messages = warnings
        .iter()
        .filter(|warning| warning.severity == PdfParseErrorSeverity::Error)
        .take(3)
        .map(|warning| {
            warning
                .msg
                .chars()
                .filter(|character| !character.is_control())
                .take(240)
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        Ok(())
    } else {
        Err(format!("{stage} 失败：{}", messages.join("；")))
    }
}

fn ensure_pdf_html_size(bytes: usize) -> Result<(), String> {
    if bytes > MAX_PDF_HTML_BYTES {
        Err(format!(
            "PDF 的 HTML 输入超过 {} MiB 安全上限",
            MAX_PDF_HTML_BYTES / 1024 / 1024
        ))
    } else {
        Ok(())
    }
}

fn ensure_pdf_page_count(pages: usize) -> Result<(), String> {
    if pages > MAX_PDF_PAGES {
        Err(format!("PDF 超过 {MAX_PDF_PAGES} 页安全上限"))
    } else {
        Ok(())
    }
}

fn ensure_pdf_output_size(bytes: usize) -> Result<(), String> {
    if bytes > MAX_PDF_BYTES {
        Err(format!(
            "PDF 输出超过 {} MiB 安全上限",
            MAX_PDF_BYTES / 1024 / 1024
        ))
    } else {
        Ok(())
    }
}

fn add_pdf_svg_bytes(total: usize, page_bytes: usize) -> Result<usize, String> {
    total
        .checked_add(page_bytes)
        .filter(|bytes| *bytes <= MAX_PDF_SVG_TOTAL_BYTES)
        .ok_or_else(|| {
            format!(
                "PDF 页面 SVG 总输出超过 {} MiB 安全上限",
                MAX_PDF_SVG_TOTAL_BYTES / 1024 / 1024
            )
        })
}

pub fn load_local_images(
    source: &str,
    document_path: Option<&Path>,
) -> Result<LocalImages, String> {
    let Some(base_directory) = document_path.and_then(Path::parent) else {
        return Ok(LocalImages::new());
    };
    let canonical_base = base_directory
        .canonicalize()
        .map_err(|error| format!("无法解析文档资源目录 {}：{error}", base_directory.display()))?;
    let destinations = markdown::local_image_destinations(source);
    if destinations.len() > MAX_LOCAL_IMAGES {
        return Err(format!("本地图片超过 {MAX_LOCAL_IMAGES} 个导出上限"));
    }

    let mut images = LocalImages::new();
    let mut total_bytes = 0usize;
    let mut total_pixels = 0u64;
    for destination in destinations {
        let resource_path = local_resource_path(&canonical_base, &destination)?;
        let bytes = read_local_image(&resource_path)?;
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= MAX_LOCAL_IMAGE_TOTAL_BYTES)
            .ok_or_else(|| {
                format!(
                    "本地图片总大小超过 {} MiB 导出上限",
                    MAX_LOCAL_IMAGE_TOTAL_BYTES / 1024 / 1024
                )
            })?;
        let reader = ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|error| format!("无法识别图片 {}：{error}", resource_path.display()))?;
        let format = reader
            .format()
            .ok_or_else(|| format!("无法识别图片格式：{}", resource_path.display()))?;
        let mime = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            _ => {
                return Err(format!(
                    "图片 {} 不是受支持的 PNG 或 JPEG",
                    resource_path.display()
                ));
            }
        };
        let (width, height) = reader
            .into_dimensions()
            .map_err(|error| format!("无法读取图片尺寸 {}：{error}", resource_path.display()))?;
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        if width == 0
            || height == 0
            || width > MAX_LOCAL_IMAGE_EDGE
            || height > MAX_LOCAL_IMAGE_EDGE
            || pixels > MAX_LOCAL_IMAGE_PIXELS
        {
            return Err(format!(
                "图片 {} 的 {}×{} 像素超过导出上限",
                resource_path.display(),
                width,
                height
            ));
        }
        total_pixels = total_pixels
            .checked_add(pixels)
            .filter(|pixels| *pixels <= MAX_LOCAL_IMAGE_TOTAL_PIXELS)
            .ok_or_else(|| {
                format!(
                    "本地图片总像素超过 {} 百万像素导出上限",
                    MAX_LOCAL_IMAGE_TOTAL_PIXELS / 1_000_000
                )
            })?;
        image::load_from_memory_with_format(&bytes, format)
            .map_err(|error| format!("图片数据不完整 {}：{error}", resource_path.display()))?;
        images.insert(destination, LocalImage { bytes, mime });
    }
    Ok(images)
}

fn read_local_image(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = open_local_image(path)
        .map_err(|error| format!("无法打开图片 {}：{error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法检查图片 {}：{error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_LOCAL_IMAGE_BYTES {
        return Err(format!(
            "图片 {} 超过 {} MiB 导出上限或不是普通文件",
            path.display(),
            MAX_LOCAL_IMAGE_BYTES / 1024 / 1024
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_LOCAL_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("无法读取图片 {}：{error}", path.display()))?;
    if bytes.len() as u64 > MAX_LOCAL_IMAGE_BYTES {
        return Err(format!(
            "图片 {} 在读取期间超过 {} MiB 导出上限",
            path.display(),
            MAX_LOCAL_IMAGE_BYTES / 1024 / 1024
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn open_local_image(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_local_image(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

pub fn embed_local_images(html: &str, images: &LocalImages) -> Result<String, String> {
    if html.len() > MAX_HTML_EXPORT_BYTES {
        return Err(format!(
            "HTML 导出超过 {} MiB 安全上限",
            MAX_HTML_EXPORT_BYTES / 1024 / 1024
        ));
    }

    let mut projected_bytes = html.len();
    for (source, image) in images {
        let source = escape_html_attribute(source);
        let needle = format!("src=\"{source}\"");
        let occurrences = html.match_indices(&needle).count();
        if occurrences == 0 {
            continue;
        }
        let encoded_bytes = image
            .bytes
            .len()
            .checked_add(2)
            .and_then(|bytes| bytes.checked_div(3))
            .and_then(|groups| groups.checked_mul(4))
            .ok_or_else(|| "无法计算嵌入图片的 HTML 大小".to_owned())?;
        let replacement_bytes = "src=\"data:".len()
            + image.mime.len()
            + ";base64,".len()
            + encoded_bytes
            + '"'.len_utf8();
        let extra_per_reference = replacement_bytes.saturating_sub(needle.len());
        projected_bytes = occurrences
            .checked_mul(extra_per_reference)
            .and_then(|extra| projected_bytes.checked_add(extra))
            .filter(|bytes| *bytes <= MAX_HTML_EXPORT_BYTES)
            .ok_or_else(|| {
                format!(
                    "嵌入本地图片后的 HTML 超过 {} MiB 安全上限",
                    MAX_HTML_EXPORT_BYTES / 1024 / 1024
                )
            })?;
    }

    let mut output = html.to_owned();
    for (source, image) in images {
        let source = escape_html_attribute(source);
        let needle = format!("src=\"{source}\"");
        let encoded = STANDARD.encode(&image.bytes);
        let replacement = format!("src=\"data:{};base64,{encoded}\"", image.mime);
        output = output.replace(&needle, &replacement);
    }
    debug_assert_eq!(output.len(), projected_bytes);
    Ok(output)
}

fn local_resource_path(base_directory: &Path, destination: &str) -> Result<PathBuf, String> {
    let path_part = destination.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode_path(path_part)?;
    let relative = Path::new(&decoded);
    if relative.is_absolute() {
        return Err(format!("图片路径必须位于文档目录内：{destination}"));
    }
    let canonical = base_directory
        .join(relative)
        .canonicalize()
        .map_err(|error| format!("无法解析图片 {destination}：{error}"))?;
    if !canonical.starts_with(base_directory) {
        return Err(format!("图片路径越过文档目录：{destination}"));
    }
    Ok(canonical)
}

fn percent_decode_path(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(format!("图片路径包含无效转义：{value}"));
            }
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            let Some(decoded) = high.zip(low).map(|(high, low)| (high << 4) | low) else {
                return Err(format!("图片路径包含无效转义：{value}"));
            };
            output.push(decoded);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| format!("图片路径不是有效 UTF-8：{value}"))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn escape_html_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn write_pdf(path: &Path, html: &str, images: &LocalImages) -> Result<(), String> {
    let bytes = render_pdf_with_images(html, images)?;
    write_bytes_atomically(path, &bytes)
}

pub fn write_html(path: &Path, html: &str) -> Result<(), String> {
    if html.len() > MAX_HTML_EXPORT_BYTES {
        return Err(format!(
            "HTML 导出超过 {} MiB 安全上限",
            MAX_HTML_EXPORT_BYTES / 1024 / 1024
        ));
    }
    write_bytes_atomically(path, html.as_bytes())
}

pub fn print_html(html: &str, images: &LocalImages) -> Result<PathBuf, String> {
    let directory = eframe::storage_dir("RUPORA")
        .unwrap_or_else(std::env::temp_dir)
        .join("print-jobs");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("无法创建打印目录 {}：{error}", directory.display()))?;
    let bytes = render_pdf_with_images(html, images)?;
    let mut temporary = TempFileBuilder::new()
        .prefix("rupora-print-")
        .suffix(".pdf")
        .tempfile_in(&directory)
        .map_err(|error| format!("无法创建唯一打印文件：{error}"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| format!("无法写入打印文件：{error}"))?;
    let (_, path) = temporary
        .keep()
        .map_err(|error| format!("无法保留打印文件：{}", error.error))?;
    sync_parent_directory(&directory)?;
    send_pdf_to_printer(&path)?;
    Ok(path)
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => path
            .canonicalize()
            .map_err(|error| format!("无法解析导出符号链接 {}：{error}", path.display()))?,
        Ok(_) => path.to_path_buf(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(format!("无法检查导出目标 {}：{error}", path.display())),
    };
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("无法在 {} 创建导出临时文件：{error}", parent.display()))?;
    if let Ok(metadata) = fs::metadata(&target) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| format!("无法继承 {} 的权限：{error}", target.display()))?;
    }
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| format!("无法写入导出目标 {}：{error}", target.display()))?;
    temporary
        .persist(&target)
        .map_err(|error| format!("无法提交导出目标 {}：{}", target.display(), error.error))?;
    sync_parent_directory(parent)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), String> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("无法同步导出目录 {}：{error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn send_pdf_to_printer(path: &Path) -> Result<(), String> {
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Process -FilePath $args[0] -Verb Print",
        ])
        .arg(path)
        .status()
        .map_err(|error| format!("无法调用系统打印服务：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("系统打印命令失败，退出码：{status}"))
    }
}

#[cfg(not(target_os = "windows"))]
fn send_pdf_to_printer(path: &Path) -> Result<(), String> {
    let status = Command::new("lp")
        .arg(path)
        .status()
        .map_err(|error| format!("无法调用 lp 打印服务：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("系统打印命令失败，退出码：{status}"))
    }
}

fn system_cjk_font() -> Option<Vec<u8>> {
    cjk_font_candidates()
        .into_iter()
        .find_map(|path| fs::read(path).ok())
}

pub fn cjk_font_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        vec![
            PathBuf::from(r"C:\Windows\Fonts\msyh.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\msyh.ttf"),
            PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
            PathBuf::from("/System/Library/Fonts/STHeiti Light.ttc"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_a_valid_pdf_header() {
        let bytes =
            render_pdf("<!doctype html><html><body><h1>RUPORA</h1><p>Native PDF</p></body></html>")
                .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(bytes.len() > 500);
    }

    #[test]
    fn renders_pdf_pages_back_to_svg_for_visual_regression() {
        let pages = render_pdf_svg_pages(
            "<!doctype html><html><body><h1>RUPORA</h1><p>Visual page</p></body></html>",
        )
        .unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].starts_with("<svg"));
        assert!(pages[0].contains("<path") || pages[0].contains("<text"));
        assert!(pages[0].len() > 1_000);

        // Do not search the raw SVG for strings such as `NaN`: printpdf embeds
        // complete font files as base64 data URLs, where any such character
        // sequence is valid opaque payload. Parsing the SVG verifies the actual
        // markup and numeric geometry without misclassifying embedded bytes.
        let tree = usvg::Tree::from_str(&pages[0], &usvg::Options::default()).unwrap();
        assert!(tree.size().width().is_finite());
        assert!(tree.size().height().is_finite());
        assert!(tree.size().width() > 0.0);
        assert!(tree.size().height() > 0.0);
    }

    #[test]
    fn embeds_bounded_local_images_in_html_and_pdf() {
        let directory = tempfile::tempdir().unwrap();
        let document_path = directory.path().join("document.md");
        let image_path = directory.path().join("small icon.png");
        fs::write(&document_path, "![icon](small%20icon.png)").unwrap();
        fs::write(&image_path, include_bytes!("../../assets/icons/32x32.png")).unwrap();
        let source = fs::read_to_string(&document_path).unwrap();
        let images = load_local_images(&source, Some(&document_path)).unwrap();
        assert_eq!(images.len(), 1);

        let html = markdown::render_html_document(&source, "images", false);
        let embedded = embed_local_images(&html, &images).unwrap();
        assert!(embedded.contains("src=\"data:image/png;base64,"));
        assert!(!embedded.contains("src=\"small%20icon.png\""));

        let pdf = render_pdf_with_images(&html, &images).unwrap();
        assert!(
            pdf.windows(14).any(|bytes| bytes == b"/Subtype/Image")
                || pdf.windows(15).any(|bytes| bytes == b"/Subtype /Image")
        );
    }

    #[test]
    fn refuses_images_outside_the_document_directory() {
        let directory = tempfile::tempdir().unwrap();
        let document_directory = directory.path().join("docs");
        fs::create_dir(&document_directory).unwrap();
        let document_path = document_directory.join("document.md");
        fs::write(&document_path, "![outside](../outside.png)").unwrap();
        fs::write(
            directory.path().join("outside.png"),
            include_bytes!("../../assets/icons/32x32.png"),
        )
        .unwrap();

        let error = load_local_images(
            &fs::read_to_string(&document_path).unwrap(),
            Some(&document_path),
        )
        .unwrap_err();
        assert!(error.contains("越过文档目录"));
    }

    #[test]
    fn rejects_pdf_parser_errors_without_overwriting_an_existing_export() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("existing.pdf");
        fs::write(&target, b"previous export").unwrap();

        let error = write_pdf(&target, "<", &LocalImages::new()).unwrap_err();
        assert!(error.contains("PDF") || error.contains("HTML"));
        assert_eq!(fs::read(target).unwrap(), b"previous export");
    }

    #[test]
    fn rejects_repeated_embedded_image_amplification_before_encoding() {
        let mut images = LocalImages::new();
        images.insert(
            "large.png".to_owned(),
            LocalImage {
                bytes: vec![0; 1024 * 1024],
                mime: "image/png",
            },
        );
        let html = r#"<img src="large.png">"#.repeat(96);

        let error = embed_local_images(&html, &images).unwrap_err();

        assert!(error.contains("HTML"));
        assert!(error.contains("安全上限"));
    }

    #[test]
    fn rejects_oversized_pdf_html_without_overwriting_an_existing_export() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("existing.pdf");
        fs::write(&target, b"previous export").unwrap();
        let oversized = "x".repeat(MAX_PDF_HTML_BYTES + 1);

        let error = write_pdf(&target, &oversized, &LocalImages::new()).unwrap_err();

        assert!(error.contains("HTML"));
        assert_eq!(fs::read(target).unwrap(), b"previous export");
    }

    #[test]
    fn enforces_pdf_page_output_and_svg_budgets_at_the_boundary() {
        assert!(ensure_pdf_page_count(MAX_PDF_PAGES).is_ok());
        assert!(ensure_pdf_page_count(MAX_PDF_PAGES + 1).is_err());
        assert!(ensure_pdf_output_size(MAX_PDF_BYTES).is_ok());
        assert!(ensure_pdf_output_size(MAX_PDF_BYTES + 1).is_err());
        assert_eq!(
            add_pdf_svg_bytes(MAX_PDF_SVG_TOTAL_BYTES - 1, 1).unwrap(),
            MAX_PDF_SVG_TOTAL_BYTES
        );
        assert!(add_pdf_svg_bytes(MAX_PDF_SVG_TOTAL_BYTES, 1).is_err());
    }

    #[test]
    fn rejects_truncated_local_image_data() {
        let directory = tempfile::tempdir().unwrap();
        let document_path = directory.path().join("document.md");
        fs::write(&document_path, "![broken](broken.png)").unwrap();
        let png = include_bytes!("../../assets/icons/32x32.png");
        fs::write(directory.path().join("broken.png"), &png[..32]).unwrap();

        assert!(
            load_local_images(
                &fs::read_to_string(&document_path).unwrap(),
                Some(&document_path)
            )
            .is_err()
        );
    }

    #[test]
    fn assigns_unique_pdf_resource_ids_to_colliding_source_names() {
        let bytes = include_bytes!("../../assets/icons/32x32.png").to_vec();
        let mut images = LocalImages::new();
        images.insert(
            "a-b.png".to_owned(),
            LocalImage {
                bytes: bytes.clone(),
                mime: "image/png",
            },
        );
        images.insert(
            "a_b.png".to_owned(),
            LocalImage {
                bytes,
                mime: "image/png",
            },
        );

        let document = create_pdf_document(
            r#"<html><body><img src="a-b.png"><img src="a_b.png"></body></html>"#,
            &images,
        )
        .unwrap();
        assert_eq!(document.resources.xobjects.map.len(), 2);
    }
}
