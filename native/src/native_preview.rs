use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

use eframe::egui::{self, Context, Response, Ui};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::markdown;

pub(crate) const MAX_GENERATED_SVG_CACHE_ENTRIES: usize = 128;
pub(crate) const MAX_GENERATED_SVG_CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeImage {
    pub range: std::ops::Range<usize>,
    pub destination: String,
    pub alt: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeMath {
    pub range: std::ops::Range<usize>,
    pub source: String,
}

pub(crate) fn standalone_image(source: &str) -> Option<NativeImage> {
    let mut images = Vec::new();
    let mut active = None;
    for (event, range) in Parser::new_ext(source, markdown::parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                active = Some(NativeImage {
                    range,
                    destination: dest_url.into_string(),
                    alt: String::new(),
                });
            }
            Event::Text(text) | Event::Code(text) if active.is_some() => {
                if let Some(image) = active.as_mut() {
                    image.alt.push_str(&text);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some(image) = active.take() {
                    images.push(image);
                }
            }
            _ => {}
        }
    }
    let image = images.pop()?;
    images.is_empty().then_some(())?;
    source[..image.range.start]
        .trim()
        .is_empty()
        .then_some(())?;
    source[image.range.end..].trim().is_empty().then_some(image)
}

pub(crate) fn standalone_display_math(source: &str) -> Option<NativeMath> {
    let mut math = None;
    for (event, range) in Parser::new_ext(source, markdown::parser_options()).into_offset_iter() {
        match event {
            Event::DisplayMath(value) if math.is_none() => {
                math = Some(NativeMath {
                    range,
                    source: value.into_string(),
                });
            }
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
            Event::Text(value) if value.trim().is_empty() => {}
            Event::SoftBreak | Event::HardBreak => {}
            Event::DisplayMath(_) => return None,
            _ => return None,
        }
    }
    math
}

pub(crate) fn standalone_mermaid(source: &str) -> Option<markdown::MermaidBlock> {
    let mut blocks = markdown::mermaid_blocks(source);
    let block = blocks.pop()?;
    blocks.is_empty().then_some(())?;
    source[..block.range.start]
        .trim()
        .is_empty()
        .then_some(())?;
    source[block.range.end..].trim().is_empty().then_some(block)
}

pub(crate) fn document_image_uri(
    base_directory: &Path,
    destination: &str,
) -> Result<String, String> {
    let path_part = destination.split(['?', '#']).next().unwrap_or_default();
    if path_part.is_empty() {
        return Err("图片路径为空".to_owned());
    }
    if has_uri_scheme(path_part) && !Path::new(path_part).is_absolute() {
        return Err("远程图片不会自动联网加载".to_owned());
    }
    let decoded =
        decode_local_resource_path(destination).ok_or_else(|| "图片路径编码无效".to_owned())?;
    let path = PathBuf::from(decoded);
    let resolved = if path.is_absolute() {
        path
    } else {
        base_directory.join(path)
    };
    if !resolved.is_file() {
        return Err(format!("找不到图片：{}", resolved.display()));
    }
    let absolute = resolved.canonicalize().unwrap_or(resolved);
    // egui_extras strips its file:// prefix but does not percent-decode the
    // remaining path. Preserve the native path, including Windows verbatim
    // prefixes and literal %, #, spaces and Unicode characters.
    let path = absolute
        .to_str()
        .ok_or_else(|| "图片路径不能表示为 Unicode".to_owned())?;
    Ok(if cfg!(windows) {
        format!("file:///{path}")
    } else {
        format!("file://{path}")
    })
}

pub(crate) fn decode_local_resource_path(destination: &str) -> Option<String> {
    let path_part = destination.split(['?', '#']).next().unwrap_or_default();
    if path_part.is_empty() || (has_uri_scheme(path_part) && !Path::new(path_part).is_absolute()) {
        return None;
    }
    percent_decode_path(path_part)
}

fn has_uri_scheme(value: &str) -> bool {
    let before_slash = value
        .find(['/', '\\'])
        .map_or(value, |index| &value[..index]);
    before_slash.contains(':')
}

fn percent_decode_path(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' {
            let high = bytes.get(cursor + 1).and_then(|value| hex_value(*value))?;
            let low = bytes.get(cursor + 2).and_then(|value| hex_value(*value))?;
            decoded.push((high << 4) | low);
            cursor += 3;
        } else {
            decoded.push(bytes[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn render_math_widget(
    ui: &mut Ui,
    cache: &mut HashMap<String, Arc<[u8]>>,
    math: &str,
    inline: bool,
    dark: bool,
) -> Response {
    let kind = if inline {
        "math-inline"
    } else {
        "math-display"
    };
    let key = generated_svg_key(kind, math, dark);
    let rendered = if let Some(bytes) = cache.get(&key) {
        Ok(bytes.clone())
    } else {
        markdown::render_math_svg(math, inline).and_then(|mut svg| {
            if dark {
                svg = svg
                    .replace("rgba(0,0,0,1)", "rgba(232,234,240,1)")
                    .replace("rgba(0, 0, 0, 1)", "rgba(232, 234, 240, 1)")
                    .replace("rgb(0,0,0)", "rgb(232,234,240)");
            }
            let bytes = Arc::<[u8]>::from(svg.into_bytes());
            cache_generated_svg(ui.ctx(), cache, key.clone(), bytes.clone())
                .then_some(bytes)
                .ok_or_else(|| "公式超过预览缓存资源预算".to_owned())
        })
    };
    show_generated_svg(ui, key, rendered, format!("公式错误：{math}"))
}

pub(crate) fn render_mermaid_widget(
    ui: &mut Ui,
    cache: &mut HashMap<String, Arc<[u8]>>,
    source: &str,
    dark: bool,
) -> Response {
    let key = generated_svg_key("mermaid", source, dark);
    let rendered = if let Some(bytes) = cache.get(&key) {
        Ok(bytes.clone())
    } else {
        markdown::render_mermaid_svg(source, dark).and_then(|svg| {
            let bytes = Arc::<[u8]>::from(svg.into_bytes());
            cache_generated_svg(ui.ctx(), cache, key.clone(), bytes.clone())
                .then_some(bytes)
                .ok_or_else(|| "图表超过预览缓存资源预算".to_owned())
        })
    };
    show_generated_svg(ui, key, rendered, "Mermaid 图表错误".to_owned())
}

fn show_generated_svg(
    ui: &mut Ui,
    key: String,
    rendered: Result<Arc<[u8]>, String>,
    error_prefix: String,
) -> Response {
    match rendered {
        Ok(bytes) => ui.add(
            egui::Image::new(egui::ImageSource::Bytes {
                uri: format!("bytes://rupora/{key}.svg").into(),
                bytes: egui::load::Bytes::Shared(bytes),
            })
            .fit_to_original_size(1.0)
            .max_width(ui.available_width()),
        ),
        Err(error) => ui.colored_label(
            ui.visuals().error_fg_color,
            format!("{error_prefix}（{}）", error.replace('\n', " ")),
        ),
    }
}

fn generated_svg_key(kind: &str, source: &str, dark: bool) -> String {
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    format!("{kind}-{:016x}", hasher.finish())
}

pub(crate) fn cache_generated_svg(
    ctx: &Context,
    cache: &mut HashMap<String, Arc<[u8]>>,
    key: String,
    bytes: Arc<[u8]>,
) -> bool {
    if bytes.len() > MAX_GENERATED_SVG_CACHE_BYTES {
        return false;
    }
    let replacing = cache.contains_key(&key);
    while (!replacing && cache.len() >= MAX_GENERATED_SVG_CACHE_ENTRIES)
        || cache
            .values()
            .map(|value| value.len())
            .sum::<usize>()
            .saturating_sub(cache.get(&key).map_or(0, |existing| existing.len()))
            .saturating_add(bytes.len())
            > MAX_GENERATED_SVG_CACHE_BYTES
    {
        let Some(victim) = cache.keys().find(|candidate| *candidate != &key).cloned() else {
            return false;
        };
        cache.remove(&victim);
        ctx.forget_image(&format!("bytes://rupora/{victim}.svg"));
    }
    cache.insert(key, bytes);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_local_image_uri_is_loadable_by_the_installed_egui_loader() {
        use egui::load::BytesPoll;
        use std::time::{Duration, Instant};

        let directory = tempfile::tempdir().unwrap();
        let filename = "图片 space #%🙂.png";
        let bytes = b"actual image bytes";
        std::fs::write(directory.path().join(filename), bytes).unwrap();
        let uri = document_image_uri(directory.path(), "图片%20space%20%23%25🙂.png").unwrap();
        let context = Context::default();
        egui_extras::install_image_loaders(&context);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match context.try_load_bytes(&uri).unwrap() {
                BytesPoll::Ready { bytes: loaded, .. } => {
                    assert_eq!(loaded.as_ref(), bytes);
                    break;
                }
                BytesPoll::Pending { .. } => {
                    assert!(Instant::now() < deadline, "image loader timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn recognizes_only_standalone_markdown_images() {
        let image = standalone_image("![审计截图](missing%20image.png)").unwrap();
        assert_eq!(image.alt, "审计截图");
        assert_eq!(image.destination, "missing%20image.png");
        assert!(standalone_image("before ![inline](image.png) after").is_none());
        assert!(standalone_image("![one](1.png) ![two](2.png)").is_none());
    }

    #[test]
    fn resolves_existing_local_images_and_reports_missing_or_remote_ones() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("present image.png"), b"image").unwrap();
        let uri = document_image_uri(directory.path(), "present%20image.png").unwrap();
        assert!(uri.starts_with("file://"));
        assert!(uri.ends_with("present image.png"));
        assert!(document_image_uri(directory.path(), "missing.png").is_err());
        assert!(document_image_uri(directory.path(), "https://example.invalid/image.png").is_err());
    }

    #[test]
    fn recognizes_standalone_display_math_and_mermaid() {
        let math = standalone_display_math("$$x^2 + y^2$$").unwrap();
        assert_eq!(math.source, "x^2 + y^2");
        assert!(standalone_display_math("before $x$ after").is_none());

        let diagram = standalone_mermaid("```mermaid\nflowchart LR\nA --> B\n```").unwrap();
        assert_eq!(diagram.source, "flowchart LR\nA --> B\n");
        assert!(standalone_mermaid("```rust\nlet x = 1;\n```").is_none());
    }

    #[test]
    fn rejects_invalid_percent_encoded_image_paths() {
        let directory = tempfile::tempdir().unwrap();
        assert!(document_image_uri(directory.path(), "bad%ZZ.png").is_err());
        assert_eq!(
            decode_local_resource_path("notes/My%20Note.md#part").as_deref(),
            Some("notes/My Note.md")
        );
        assert!(decode_local_resource_path("https://example.com/note.md").is_none());
    }
}
