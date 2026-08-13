use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

use eframe::egui::{self, Context, Ui};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::markdown;

pub(crate) const MAX_GENERATED_SVG_CACHE_ENTRIES: usize = 128;
pub(crate) const MAX_GENERATED_SVG_CACHE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn prepare_native_preview(
    ctx: &Context,
    source: &str,
    base_directory: &Path,
    dark: bool,
    cache: &mut HashMap<String, Arc<[u8]>>,
) -> String {
    let mut output = markdown::prepare_preview_markdown(source);
    for block in markdown::mermaid_blocks(&output).into_iter().rev() {
        let key = generated_svg_key("mermaid", &block.source, dark);
        let rendered = if let Some(bytes) = cache.get(&key) {
            Ok(bytes.clone())
        } else {
            markdown::render_mermaid_svg(&block.source, dark).and_then(|svg| {
                let bytes = Arc::<[u8]>::from(svg.into_bytes());
                cache_generated_svg(ctx, cache, key.clone(), bytes.clone())
                    .then_some(bytes)
                    .ok_or_else(|| "图表超过预览缓存资源预算".to_owned())
            })
        };
        let replacement = match rendered {
            Ok(bytes) => {
                let uri = format!("bytes://rupora/{key}.svg");
                ctx.include_bytes(uri.clone(), egui::load::Bytes::Shared(bytes));
                format!("\n\n![Mermaid diagram]({uri})\n\n")
            }
            Err(error) => format!(
                "\n\n> **Mermaid 图表错误：** {}\n\n",
                error.replace('\n', " ")
            ),
        };
        output.replace_range(block.range, &replacement);
    }
    replace_missing_local_images(&output, base_directory)
}

#[derive(Debug)]
struct PreviewImage {
    range: std::ops::Range<usize>,
    destination: String,
    alt: String,
}

fn replace_missing_local_images(source: &str, base_directory: &Path) -> String {
    let mut images = Vec::new();
    let mut active = None;
    for (event, range) in Parser::new_ext(source, markdown::parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                active = Some(PreviewImage {
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
                if let Some(image) = active.take()
                    && local_image_is_missing(base_directory, &image.destination)
                {
                    images.push(image);
                }
            }
            _ => {}
        }
    }
    if images.is_empty() {
        return source.to_owned();
    }

    let mut output = source.to_owned();
    for image in images.into_iter().rev() {
        let alt = single_line_markdown_text(&image.alt, "未命名图片");
        let destination = single_line_markdown_text(&image.destination, "未知路径");
        let replacement = format!("\n\n> **图片不可用：{alt}**  \n> 路径：`{destination}`\n\n");
        output.replace_range(image.range, &replacement);
    }
    output
}

fn local_image_is_missing(base_directory: &Path, destination: &str) -> bool {
    let path_part = destination.split(['?', '#']).next().unwrap_or_default();
    if path_part.is_empty() || has_uri_scheme(path_part) {
        return false;
    }
    let Some(decoded) = percent_decode_path(path_part) else {
        return true;
    };
    let path = PathBuf::from(decoded);
    let resolved = if path.is_absolute() {
        path
    } else {
        base_directory.join(path)
    };
    !resolved.is_file()
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

fn single_line_markdown_text(value: &str, fallback: &str) -> String {
    let value = value.trim().replace(['\r', '\n'], " ");
    let value = value.replace('`', "'").replace('*', "\\*");
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}

pub(crate) fn render_math_widget(
    ui: &mut Ui,
    cache: &mut HashMap<String, Arc<[u8]>>,
    math: &str,
    inline: bool,
    dark: bool,
) {
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
    match rendered {
        Ok(bytes) => {
            let uri = format!("bytes://rupora/{key}.svg");
            ui.add(
                egui::Image::new(egui::ImageSource::Bytes {
                    uri: uri.into(),
                    bytes: egui::load::Bytes::Shared(bytes),
                })
                .fit_to_original_size(1.0)
                .max_width(ui.available_width()),
            );
        }
        Err(error) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("公式错误：{math}（{error}）"),
            );
        }
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
    _ctx: &Context,
    cache: &mut HashMap<String, Arc<[u8]>>,
    key: String,
    bytes: Arc<[u8]>,
) -> bool {
    if bytes.len() > MAX_GENERATED_SVG_CACHE_BYTES {
        return false;
    }
    let existing_bytes = cache.get(&key).map_or(0, |existing| existing.len());
    let cached_bytes = cache.values().map(|value| value.len()).sum::<usize>();
    let projected_bytes = cached_bytes
        .saturating_sub(existing_bytes)
        .saturating_add(bytes.len());
    if (cache.len() >= MAX_GENERATED_SVG_CACHE_ENTRIES && !cache.contains_key(&key))
        || projected_bytes > MAX_GENERATED_SVG_CACHE_BYTES
    {
        return false;
    }
    cache.insert(key, bytes);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_local_images_become_readable_placeholders() {
        let directory = tempfile::tempdir().unwrap();
        let source = "before\n\n![审计截图](missing%20image.png)\n\nafter";
        let preview = replace_missing_local_images(source, directory.path());

        assert!(preview.contains("图片不可用：审计截图"));
        assert!(preview.contains("missing%20image.png"));
        assert!(!preview.contains("![审计截图]"));
        assert!(preview.contains("before"));
        assert!(preview.contains("after"));
    }

    #[test]
    fn existing_and_remote_images_remain_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("present image.png"), b"image").unwrap();
        for source in [
            "![存在](present%20image.png)",
            "![远程](https://example.invalid/image.png)",
            "![内存](bytes://rupora/generated.svg)",
        ] {
            assert_eq!(
                replace_missing_local_images(source, directory.path()),
                source
            );
        }
    }

    #[test]
    fn missing_image_placeholder_escapes_active_markdown() {
        let directory = tempfile::tempdir().unwrap();
        let preview =
            replace_missing_local_images("![**危险**](missing`name.png)", directory.path());

        assert!(preview.contains("图片不可用：危险"));
        assert!(!preview.contains("**危险**"));
        assert!(preview.contains("missing'name.png"));
    }
}
