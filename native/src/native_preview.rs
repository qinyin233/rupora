use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use eframe::egui::{self, Context, Ui};

use crate::markdown;

pub(crate) const MAX_GENERATED_SVG_CACHE_ENTRIES: usize = 128;

pub(crate) fn prepare_native_preview(
    ctx: &Context,
    source: &str,
    dark: bool,
    cache: &mut HashMap<String, Arc<[u8]>>,
) -> String {
    let mut output = markdown::prepare_preview_markdown(source);
    for block in markdown::mermaid_blocks(&output).into_iter().rev() {
        let key = generated_svg_key("mermaid", &block.source, dark);
        let rendered = if let Some(bytes) = cache.get(&key) {
            Ok(bytes.clone())
        } else {
            markdown::render_mermaid_svg(&block.source, dark).map(|svg| {
                let bytes = Arc::<[u8]>::from(svg.into_bytes());
                cache_generated_svg(cache, key.clone(), bytes.clone());
                bytes
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
    output
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
        markdown::render_math_svg(math, inline).map(|mut svg| {
            if dark {
                svg = svg
                    .replace("rgba(0,0,0,1)", "rgba(232,234,240,1)")
                    .replace("rgba(0, 0, 0, 1)", "rgba(232, 234, 240, 1)")
                    .replace("rgb(0,0,0)", "rgb(232,234,240)");
            }
            let bytes = Arc::<[u8]>::from(svg.into_bytes());
            cache_generated_svg(cache, key.clone(), bytes.clone());
            bytes
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
    cache: &mut HashMap<String, Arc<[u8]>>,
    key: String,
    bytes: Arc<[u8]>,
) {
    if cache.len() >= MAX_GENERATED_SVG_CACHE_ENTRIES && !cache.contains_key(&key) {
        cache.clear();
    }
    cache.insert(key, bytes);
}
