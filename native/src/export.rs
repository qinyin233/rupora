use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use printpdf::{Base64OrRaw, GeneratePdfOptions, PdfDocument, PdfSaveOptions};

pub fn render_pdf(html: &str) -> Result<Vec<u8>, String> {
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
    let mut warnings = Vec::new();
    let document = PdfDocument::from_html(html, &BTreeMap::new(), &fonts, &options, &mut warnings)
        .map_err(|error| format!("HTML 转 PDF 失败：{error}"))?;
    let mut save_warnings = Vec::new();
    let bytes = document.save(&PdfSaveOptions::default(), &mut save_warnings);
    if bytes.len() < 5 || !bytes.starts_with(b"%PDF-") {
        return Err("PDF 生成器没有返回有效文档".to_owned());
    }
    Ok(bytes)
}

pub fn write_pdf(path: &Path, html: &str) -> Result<(), String> {
    let bytes = render_pdf(html)?;
    fs::write(path, bytes).map_err(|error| format!("无法写入 {}：{error}", path.display()))
}

pub fn print_html(html: &str) -> Result<PathBuf, String> {
    let directory = eframe::storage_dir("RUPORA")
        .unwrap_or_else(std::env::temp_dir)
        .join("print-jobs");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("无法创建打印目录 {}：{error}", directory.display()))?;
    let path = directory.join("rupora-print.pdf");
    write_pdf(&path, html)?;
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
}
