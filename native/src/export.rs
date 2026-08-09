use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader};
use printpdf::{Base64OrRaw, GeneratePdfOptions, PdfDocument, PdfSaveOptions, PdfToSvgOptions};

use crate::markdown;

const MAX_LOCAL_IMAGES: usize = 32;
const MAX_LOCAL_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LOCAL_IMAGE_TOTAL_BYTES: usize = 24 * 1024 * 1024;
const MAX_LOCAL_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;

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
    if bytes.len() < 5 || !bytes.starts_with(b"%PDF-") {
        return Err("PDF 生成器没有返回有效文档".to_owned());
    }
    Ok(bytes)
}

pub fn render_pdf_svg_pages(html: &str) -> Result<Vec<String>, String> {
    let document = create_pdf_document(html, &LocalImages::new())?;
    let mut warnings = Vec::new();
    (1..=document.pages.len())
        .map(|page| {
            document
                .page_to_svg(page, &PdfToSvgOptions::default(), &mut warnings)
                .ok_or_else(|| format!("无法渲染 PDF 第 {page} 页"))
        })
        .collect()
}

fn create_pdf_document(html: &str, images: &LocalImages) -> Result<PdfDocument, String> {
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
    let images = images
        .iter()
        .map(|(source, image)| (source.clone(), Base64OrRaw::Raw(image.bytes.clone())))
        .collect::<BTreeMap<_, _>>();
    let mut warnings = Vec::new();
    PdfDocument::from_html(html, &images, &fonts, &options, &mut warnings)
        .map_err(|error| format!("HTML 转 PDF 失败：{error}"))
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
    for destination in destinations {
        let resource_path = local_resource_path(&canonical_base, &destination)?;
        let metadata = fs::metadata(&resource_path)
            .map_err(|error| format!("无法读取图片 {}：{error}", resource_path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_LOCAL_IMAGE_BYTES {
            return Err(format!(
                "图片 {} 超过 {} MiB 导出上限或不是普通文件",
                resource_path.display(),
                MAX_LOCAL_IMAGE_BYTES / 1024 / 1024
            ));
        }
        let bytes = fs::read(&resource_path)
            .map_err(|error| format!("无法读取图片 {}：{error}", resource_path.display()))?;
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
        if u64::from(width).saturating_mul(u64::from(height)) > MAX_LOCAL_IMAGE_PIXELS {
            return Err(format!(
                "图片 {} 的 {}×{} 像素超过导出上限",
                resource_path.display(),
                width,
                height
            ));
        }
        images.insert(destination, LocalImage { bytes, mime });
    }
    Ok(images)
}

pub fn embed_local_images(html: &str, images: &LocalImages) -> String {
    let mut output = html.to_owned();
    for (source, image) in images {
        let source = escape_html_attribute(source);
        let needle = format!("src=\"{source}\"");
        let encoded = STANDARD.encode(&image.bytes);
        let replacement = format!("src=\"data:{};base64,{encoded}\"", image.mime);
        output = output.replace(&needle, &replacement);
    }
    output
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
    fs::write(path, bytes).map_err(|error| format!("无法写入 {}：{error}", path.display()))
}

pub fn print_html(html: &str, images: &LocalImages) -> Result<PathBuf, String> {
    let directory = eframe::storage_dir("RUPORA")
        .unwrap_or_else(std::env::temp_dir)
        .join("print-jobs");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("无法创建打印目录 {}：{error}", directory.display()))?;
    let path = directory.join("rupora-print.pdf");
    write_pdf(&path, html, images)?;
    send_pdf_to_printer(&path)?;
    Ok(path)
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
        assert!(!pages[0].contains("NaN"));
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
        let embedded = embed_local_images(&html, &images);
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
}
