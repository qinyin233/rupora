//! Native document rendering, source-coordinate hit testing and render caches.
//!
//! This module receives source and view inputs; it never reaches into application
//! state. Cache handles can be cloned into UI closures without exposing their
//! storage or holding an application borrow while a document is painted.

use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::Path,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use eframe::egui::{
    self, Button, FontFamily, FontId, Margin, RichText, Stroke, Ui, Vec2,
    text::{CCursor, CCursorRange, LayoutJob},
};

use crate::{
    editing::char_to_byte,
    markdown::{self, BlockId},
    native_preview::{
        LocalImageStore, NativeImage, ResolvedLocalImage, render_math_widget,
        render_mermaid_widget, standalone_display_math, standalone_image_with_references,
        standalone_mermaid,
    },
    presentation::{
        AppPalette, WYSIWYG_BODY_LINE_HEIGHT, WYSIWYG_INLINE_CODE_VERTICAL_PADDING, app_palette,
        code_block_frame, show_code_header, visual_text_format,
    },
    table,
    wysiwyg::{VisualProjection, fenced_code_language},
};

mod prepared;

const MAX_HYBRID_BLOCK_HEIGHT_CACHE: usize = 65_536;

/// Owns the caches shared by reading and editing views. Clones share the same
/// cache state; no caller needs to manage SVG storage or cache eviction.
#[derive(Clone, Default)]
pub(crate) struct RenderCache {
    state: Rc<RefCell<RenderCacheState>>,
}

#[derive(Default)]
struct RenderCacheState {
    resources: RenderResources,
    block_heights: HashMap<HybridBlockLayoutKey, CachedBlockHeight>,
    documents: HashMap<u64, prepared::PreparedDocumentState>,
}

struct CachedBlockHeight {
    dependencies: prepared::RenderingDependencies,
    height: f32,
}

#[derive(Default)]
struct RenderResources {
    generated_svg: HashMap<String, Arc<[u8]>>,
    local_images: LocalImageStore,
}

struct PreparedImage {
    image: NativeImage,
    resource: ResolvedLocalImage,
}

struct BlockPreviewContext<'a> {
    owner: (u64, BlockId),
    base_directory: &'a Path,
    dark: bool,
    palette: AppPalette,
    references: Arc<markdown::ReferenceDefinitions>,
    image: Option<&'a PreparedImage>,
}

fn block_source_hash(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

impl RenderCache {
    /// Start a hybrid layout pass. Preserve the existing frame-level eviction
    /// policy so a long pass does not discard heights partway through layout.
    pub(crate) fn begin_frame(&self) {
        let mut state = self.state.borrow_mut();
        if state.block_heights.len() > MAX_HYBRID_BLOCK_HEIGHT_CACHE {
            state.block_heights.clear();
        }
    }

    /// Discard layout measurements for a closed or reloaded document. Generated
    /// SVGs are keyed by content and remain shared across documents and views.
    pub(crate) fn forget_document(&self, document_id: u64) {
        let mut state = self.state.borrow_mut();
        state.documents.remove(&document_id);
        state.resources.local_images.forget_document(document_id);
        state
            .block_heights
            .retain(|key, _| key.document_id != document_id);
    }

    /// Reuse a measurement only for unchanged source. A block keeps its identity
    /// across edits, so changed source must fall back to an estimate until it is
    /// measured again. Production callers use the complete prepared dependency set.
    #[cfg(test)]
    fn block_height(
        &self,
        key: HybridBlockLayoutKey,
        source: &str,
        width: f32,
        fenced_code: bool,
    ) -> f32 {
        self.state
            .borrow()
            .block_heights
            .get(&key)
            .filter(|cached| {
                cached.dependencies == prepared::RenderingDependencies::source_only(source)
            })
            .map(|cached| cached.height)
            .unwrap_or_else(|| estimated_hybrid_block_height(source, width, fenced_code))
    }

    #[cfg(test)]
    fn remember_height(&self, key: HybridBlockLayoutKey, source: &str, height: f32) {
        self.state.borrow_mut().block_heights.insert(
            key,
            CachedBlockHeight {
                dependencies: prepared::RenderingDependencies::source_only(source),
                height,
            },
        );
    }
}

pub(crate) fn is_fenced_code_block(source: &str) -> bool {
    let first_line = source.lines().next().unwrap_or_default();
    let indentation = first_line.bytes().take_while(|byte| *byte == b' ').count();
    if indentation > 3 || first_line.as_bytes().get(indentation) == Some(&b'\t') {
        return false;
    }
    let marker = &first_line[indentation..];
    if marker.starts_with("~~~") {
        return true;
    }
    let backticks = marker.bytes().take_while(|byte| *byte == b'`').count();
    // CommonMark forbids backticks in a backtick fence's info string.
    // A same-line triple-backtick code span therefore remains a paragraph.
    backticks >= 3 && !marker[backticks..].contains('`')
}

pub(crate) fn wysiwyg_layout(
    ui: &Ui,
    text: &str,
    projection: &VisualProjection,
    wrap_width: f32,
    palette: AppPalette,
    inline_code_chips: bool,
) -> Arc<egui::Galley> {
    let runs = projection.runs_for(text);
    if runs.is_empty() {
        let mut job = LayoutJob::simple(
            text.to_owned(),
            FontId::new(18.0, FontFamily::Proportional),
            palette.text,
            wrap_width,
        );
        job.sections[0].format.line_height = Some(WYSIWYG_BODY_LINE_HEIGHT);
        job.first_row_min_height = WYSIWYG_BODY_LINE_HEIGHT;
        job.keep_trailing_whitespace = true;
        return ui.fonts_mut(|fonts| fonts.layout_job(job));
    }

    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.keep_trailing_whitespace = true;
    let mut previous_was_inline_code = false;
    // Visual runs are ordered, disjoint character ranges. Keep one advancing
    // UTF-8 cursor instead of rescanning the prefix for every styled fragment.
    let mut characters = text.chars();
    let mut character_index = 0usize;
    let mut byte_index = 0usize;
    let mut byte_at = |target: usize| {
        while character_index < target {
            let Some(character) = characters.next() else {
                return text.len();
            };
            byte_index += character.len_utf8();
            character_index += 1;
        }
        byte_index
    };
    for run in runs {
        let start = byte_at(run.range.start);
        let end = byte_at(run.range.end);
        let format = visual_text_format(run.style, palette);
        let is_inline_code = inline_code_chips && run.style.code && !run.style.marker;
        let leading_space = if is_inline_code != previous_was_inline_code {
            3.0
        } else {
            0.0
        };
        job.append(&text[start..end], leading_space, format);
        previous_was_inline_code = is_inline_code;
    }
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

pub(crate) fn syntax_highlighted_code_layout(
    ui: &Ui,
    text: &str,
    wrap_width: f32,
    palette: AppPalette,
    language: Option<&str>,
) -> Arc<egui::Galley> {
    crate::code_highlight::layout(
        ui,
        text,
        wrap_width,
        crate::code_highlight::CodePalette {
            plain: palette.text,
            keyword: palette.code_keyword,
            string: palette.code_string,
            comment: palette.code_comment,
            number: palette.code_number,
        },
        language,
    )
}

pub(crate) fn rounded_inline_code_backgrounds(
    galley: &egui::Galley,
    galley_pos: egui::Pos2,
    runs: &[crate::wysiwyg::VisualRun],
    palette: AppPalette,
) -> Vec<egui::Shape> {
    let mut shapes = Vec::new();
    let mut char_index = 0usize;
    let mut run_index = 0usize;

    for placed_row in &galley.rows {
        let row_offset = galley_pos.to_vec2() + placed_row.pos.to_vec2();
        let mut chip_rect = None;
        for glyph in &placed_row.glyphs {
            while runs
                .get(run_index)
                .is_some_and(|run| run.range.end <= char_index)
            {
                run_index += 1;
            }
            let is_inline_code = runs.get(run_index).is_some_and(|run| {
                run.range.contains(&char_index) && run.style.code && !run.style.marker
            });
            if is_inline_code {
                let rect = glyph.logical_rect().translate(row_offset);
                chip_rect = Some(chip_rect.map_or(rect, |current: egui::Rect| current.union(rect)));
            } else {
                push_inline_code_background(&mut shapes, chip_rect.take(), palette);
            }
            char_index += 1;
        }
        push_inline_code_background(&mut shapes, chip_rect.take(), palette);
        if placed_row.ends_with_newline {
            char_index += 1;
        }
    }
    shapes
}

#[derive(Clone)]
pub(crate) struct TextProjectionPreview {
    pub(crate) rect: egui::Rect,
    pub(crate) galley: Arc<egui::Galley>,
    pub(crate) galley_pos: egui::Pos2,
    pub(crate) projection: VisualProjection,
}

impl TextProjectionPreview {
    fn source_byte_at_position(&self, source: &str, position: egui::Pos2) -> usize {
        let cursor = self
            .galley
            .cursor_from_pos(position - self.galley_pos + egui::vec2(self.galley.rect.left(), 0.0));
        let visual_index = cursor.index.0;
        let source_index = self
            .projection
            .source_char_range(source, visual_index..visual_index)
            .start;
        char_to_byte(source, source_index)
    }
}

#[derive(Clone)]
pub(crate) enum NativePointerMapping {
    Text(TextProjectionPreview),
    Atomic {
        source_range: std::ops::Range<usize>,
    },
}

impl NativePointerMapping {
    pub(crate) fn source_byte_at_position(
        &self,
        source: &str,
        rect: egui::Rect,
        position: egui::Pos2,
    ) -> usize {
        match self {
            Self::Text(preview) => preview.source_byte_at_position(source, position),
            Self::Atomic { source_range } => {
                if position.y < rect.center().y
                    || (position.y == rect.center().y && position.x < rect.center().x)
                {
                    source_range.start
                } else {
                    source_range.end
                }
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct NativeBlockPreview {
    pub(crate) rect: egui::Rect,
    pub(crate) mapping: NativePointerMapping,
    pub(crate) atomic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct HybridBlockLayoutKey {
    document_id: u64,
    block_id: BlockId,
    width: u32,
    pixels_per_point: u32,
    dark: bool,
}

impl HybridBlockLayoutKey {
    fn new(document_id: u64, block_id: BlockId, width: f32, dark: bool) -> Self {
        Self {
            document_id,
            block_id,
            width: width.to_bits(),
            pixels_per_point: 1.0_f32.to_bits(),
            dark,
        }
    }
}

#[derive(Clone)]
pub(crate) struct HybridPointerRegion {
    pub(crate) block_id: BlockId,
    pub(crate) source_range: std::ops::Range<usize>,
    pub(crate) rect: egui::Rect,
    pub(crate) atomic_range: Option<std::ops::Range<usize>>,
    pub(crate) mapping: NativePointerMapping,
}

impl HybridPointerRegion {
    fn source_char_at_position(&self, source: &str, position: egui::Pos2) -> Option<usize> {
        let block_source = source.get(self.source_range.clone())?;
        let local_byte = self
            .mapping
            .source_byte_at_position(block_source, self.rect, position);
        let byte = (self.source_range.start + local_byte).min(source.len());
        Some(source[..byte].chars().count())
    }
}

pub(crate) fn hybrid_pointer_hit(
    source: &str,
    regions: &[HybridPointerRegion],
    position: egui::Pos2,
) -> Option<(BlockId, usize)> {
    regions
        .iter()
        .filter(|region| region.rect.expand(3.0).contains(position))
        .min_by(|left, right| {
            pointer_rect_distance(left.rect, position)
                .total_cmp(&pointer_rect_distance(right.rect, position))
                .then_with(|| {
                    left.rect
                        .center()
                        .distance_sq(position)
                        .total_cmp(&right.rect.center().distance_sq(position))
                })
        })
        .and_then(|region| {
            region
                .source_char_at_position(source, position)
                .map(|cursor| (region.block_id, cursor))
        })
}

fn pointer_rect_distance(rect: egui::Rect, position: egui::Pos2) -> f32 {
    let horizontal = if position.x < rect.left() {
        rect.left() - position.x
    } else if position.x > rect.right() {
        position.x - rect.right()
    } else {
        0.0
    };
    let vertical = if position.y < rect.top() {
        rect.top() - position.y
    } else if position.y > rect.bottom() {
        position.y - rect.bottom()
    } else {
        0.0
    };
    horizontal.mul_add(horizontal, vertical * vertical)
}

fn estimated_hybrid_block_height(source: &str, width: f32, fenced_code: bool) -> f32 {
    let characters_per_row = (width / if fenced_code { 8.4 } else { 7.6 })
        .floor()
        .max(12.0) as usize;
    let visual_rows = source
        .lines()
        .map(|line| line.chars().count().max(1).div_ceil(characters_per_row))
        .sum::<usize>()
        .max(1);
    let trimmed = source.trim_start();
    if trimmed.starts_with("![") {
        360.0
    } else if fenced_code {
        visual_rows as f32 * 19.0 + 72.0
    } else if trimmed.starts_with('|') && source.lines().count() >= 2 {
        visual_rows as f32 * 30.0 + 24.0
    } else if trimmed.starts_with('#') {
        visual_rows as f32 * 38.0 + 18.0
    } else {
        visual_rows as f32 * 22.0 + 20.0
    }
}

pub(crate) fn paint_hybrid_cross_selection(
    ui: &Ui,
    source: &str,
    regions: &[HybridPointerRegion],
    cursor: CCursorRange,
) {
    let [start, end] = cursor.sorted_cursors();
    let selection = start.index.0..end.index.0;
    if selection.is_empty() {
        return;
    }
    for region in regions {
        let Some(local_source) = hybrid_region_selection(source, &region.source_range, &selection)
        else {
            continue;
        };
        let block_source = &source[region.source_range.clone()];
        let fully_selects_atomic_block = region.atomic_range.as_ref().is_some_and(|atomic| {
            let start = source[..atomic.start].chars().count();
            let end = source[..atomic.end].chars().count();
            selection.start <= start && selection.end >= end
        });
        if fully_selects_atomic_block {
            ui.painter().rect_filled(
                region.rect,
                8.0,
                ui.visuals().selection.bg_fill.gamma_multiply(0.32),
            );
            continue;
        }
        let NativePointerMapping::Text(preview) = &region.mapping else {
            continue;
        };
        let visual = preview
            .projection
            .visual_char_range(block_source, local_source);
        if visual.is_empty() {
            continue;
        }
        let mut galley = Arc::clone(&preview.galley);
        egui::text_selection::visuals::paint_text_selection(
            &mut galley,
            ui.visuals(),
            &CCursorRange::two(CCursor::new(visual.start), CCursor::new(visual.end)),
            None,
        );
        ui.painter()
            .galley(preview.galley_pos, galley, ui.visuals().text_color());
    }
}

fn hybrid_region_selection(
    source: &str,
    region: &std::ops::Range<usize>,
    selection: &std::ops::Range<usize>,
) -> Option<std::ops::Range<usize>> {
    let region_start = source.get(..region.start)?.chars().count();
    let region_end = source.get(..region.end)?.chars().count();
    let start = selection.start.max(region_start);
    let end = selection.end.min(region_end);
    (start < end).then(|| start - region_start..end - region_start)
}

pub(crate) fn show_code_copy_button(
    ui: &mut Ui,
    id_source: (u64, BlockId),
    code_rect: egui::Rect,
    content: &str,
    palette: AppPalette,
) -> bool {
    let id = ui.make_persistent_id(("copy-wysiwyg-code", id_source));
    let now = ui.input(|input| input.time);
    let copied_at = ui.data(|data| data.get_temp::<f64>(id));
    let recently_copied = copied_at.is_some_and(|copied_at| now - copied_at < 1.6);
    if recently_copied {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    let size = Vec2::new(if recently_copied { 76.0 } else { 54.0 }, 25.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(code_rect.right() - size.x - 8.0, code_rect.top() + 8.0),
        size,
    );
    let label = if recently_copied {
        "✓ 已复制"
    } else {
        "复制"
    };
    // This is an overlay on an already allocated block. Advancing the parent
    // cursor here would place the next paragraph below the button, inside code.
    let response = ui.place(
        rect,
        Button::new(RichText::new(label).size(11.0).color(palette.text))
            .fill(palette.surface.gamma_multiply(0.94))
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(5),
    );
    if !response.clicked() {
        return false;
    }

    ui.ctx().copy_text(content.to_owned());
    ui.data_mut(|data| data.insert_temp(id, now));
    ui.ctx().request_repaint();
    true
}

#[cfg(test)]
fn show_text_projection_preview(
    ui: &mut Ui,
    source: &str,
    palette: AppPalette,
) -> TextProjectionPreview {
    show_text_projection_preview_with_context(ui, source, palette, Default::default())
}

fn show_text_projection_preview_with_context(
    ui: &mut Ui,
    source: &str,
    palette: AppPalette,
    references: Arc<markdown::ReferenceDefinitions>,
) -> TextProjectionPreview {
    let projection = VisualProjection::from_markdown_with_context(source, None, references);
    let runs = projection.runs_for(projection.text());
    let galley = wysiwyg_layout(
        ui,
        projection.text(),
        &projection,
        ui.available_width(),
        palette,
        true,
    );
    let (galley_pos, galley, response) =
        egui::Label::new(galley).selectable(false).layout_in_ui(ui);
    ui.painter().extend(rounded_inline_code_backgrounds(
        &galley, galley_pos, &runs, palette,
    ));
    ui.painter()
        .galley(galley_pos, Arc::clone(&galley), palette.text);
    let rect = if projection.text().is_empty() {
        egui::Rect::from_min_size(
            response.rect.min,
            Vec2::new(ui.available_width(), WYSIWYG_BODY_LINE_HEIGHT),
        )
    } else {
        response.rect
    };
    TextProjectionPreview {
        rect,
        galley,
        galley_pos,
        projection,
    }
}

fn show_code_projection_preview(
    ui: &mut Ui,
    source: &str,
    palette: AppPalette,
    language: Option<&str>,
) -> TextProjectionPreview {
    let projection = VisualProjection::from_markdown(source);
    let galley = syntax_highlighted_code_layout(
        ui,
        projection.text(),
        ui.available_width(),
        palette,
        language,
    );
    let (galley_pos, galley, response) =
        egui::Label::new(galley).selectable(false).layout_in_ui(ui);
    ui.painter()
        .galley(galley_pos, Arc::clone(&galley), palette.text);
    TextProjectionPreview {
        rect: response.rect,
        galley,
        galley_pos,
        projection,
    }
}

#[cfg(test)]
fn show_native_block_preview(
    ui: &mut Ui,
    source: &str,
    base_directory: &Path,
    dark: bool,
    svg_cache: &mut HashMap<String, Arc<[u8]>>,
    palette: AppPalette,
) -> NativeBlockPreview {
    let mut resources = RenderResources {
        generated_svg: std::mem::take(svg_cache),
        ..RenderResources::default()
    };
    let preview = show_native_block_preview_with_context(
        ui,
        source,
        &mut resources,
        BlockPreviewContext {
            owner: (0, markdown::blocks(source)[0].id),
            base_directory,
            dark,
            palette,
            references: Default::default(),
            image: None,
        },
    );
    *svg_cache = resources.generated_svg;
    preview
}

fn show_native_block_preview_with_context(
    ui: &mut Ui,
    source: &str,
    resources: &mut RenderResources,
    context: BlockPreviewContext<'_>,
) -> NativeBlockPreview {
    let BlockPreviewContext {
        owner,
        base_directory,
        dark,
        palette,
        references,
        image: prepared_image,
    } = context;
    if let Some(diagram) = standalone_mermaid(source) {
        let response = egui::Frame::new()
            .fill(palette.code_bg)
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("MERMAID")
                        .monospace()
                        .size(11.0)
                        .color(palette.secondary),
                );
                ui.add_space(8.0);
                render_mermaid_widget(ui, &mut resources.generated_svg, &diagram.source, dark);
            })
            .response;
        return NativeBlockPreview {
            rect: response.rect,
            mapping: NativePointerMapping::Atomic {
                source_range: diagram.range,
            },
            atomic: true,
        };
    }

    if let Some(math) = standalone_display_math(source) {
        let response = egui::Frame::new()
            .fill(palette.code_bg.gamma_multiply(0.45))
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    render_math_widget(ui, &mut resources.generated_svg, &math.source, false, dark);
                });
            })
            .response;
        return NativeBlockPreview {
            rect: response.rect,
            mapping: NativePointerMapping::Atomic {
                source_range: math.range,
            },
            atomic: true,
        };
    }

    if let Some(image) = prepared_image
        .map(|prepared| prepared.image.clone())
        .or_else(|| standalone_image_with_references(source, &references))
    {
        let resource = prepared_image.map_or_else(
            || {
                resources
                    .local_images
                    .resolve(ui.ctx(), owner, base_directory, &image.destination)
            },
            |prepared| prepared.resource.clone(),
        );
        let response = egui::Frame::new()
            .fill(palette.code_bg.gamma_multiply(0.35))
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(12, 12))
            .show(ui, |ui| match resource.uri {
                Ok(uri) => {
                    ui.vertical_centered(|ui| {
                        ui.add(
                            egui::Image::from_uri(uri)
                                .alt_text(if image.alt.trim().is_empty() {
                                    "文档图片"
                                } else {
                                    &image.alt
                                })
                                .fit_to_original_size(1.0)
                                .max_width(ui.available_width())
                                .max_height(520.0),
                        );
                        if !image.alt.trim().is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(&image.alt)
                                    .size(12.0)
                                    .color(palette.secondary),
                            );
                        }
                    });
                }
                Err(error) => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("▧").size(20.0).color(palette.secondary));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(if image.alt.trim().is_empty() {
                                    "图片不可用"
                                } else {
                                    &image.alt
                                })
                                .strong()
                                .color(palette.text),
                            );
                            ui.label(RichText::new(error).size(12.0).color(palette.secondary));
                        });
                    });
                }
            })
            .response;
        return NativeBlockPreview {
            rect: response.rect,
            mapping: NativePointerMapping::Atomic {
                source_range: image.range,
            },
            atomic: true,
        };
    }

    if is_fenced_code_block(source) {
        let response = code_block_frame(palette).show(ui, |ui| {
            show_code_header(ui, fenced_code_language(source), palette);
            show_code_projection_preview(ui, source, palette, fenced_code_language(source))
        });
        return NativeBlockPreview {
            rect: response.response.rect,
            mapping: NativePointerMapping::Text(response.inner),
            atomic: true,
        };
    }

    let styled_container = is_native_table_block(source) || is_native_quote_block(source);
    let shown = styled_container.then(|| {
        egui::Frame::new()
            .fill(if is_native_quote_block(source) {
                palette.accent_soft.gamma_multiply(0.42)
            } else {
                palette.code_bg.gamma_multiply(0.38)
            })
            .stroke(Stroke::new(1.0, palette.border))
            .corner_radius(if is_native_quote_block(source) { 6 } else { 8 })
            .inner_margin(Margin::symmetric(14, 9))
            .show(ui, |ui| {
                show_text_projection_preview_with_context(ui, source, palette, references.clone())
            })
    });
    let (rect, preview) = if let Some(shown) = shown {
        (shown.response.rect, shown.inner)
    } else {
        let preview = show_text_projection_preview_with_context(ui, source, palette, references);
        (preview.rect, preview)
    };
    NativeBlockPreview {
        rect,
        mapping: NativePointerMapping::Text(preview),
        atomic: false,
    }
}

pub(crate) fn is_native_table_block(source: &str) -> bool {
    table::find_table(source, 0).is_some()
}

pub(crate) fn is_native_quote_block(source: &str) -> bool {
    source.trim_start().starts_with('>')
}

fn accessible_markdown_block_text(source: &str) -> String {
    let projection = VisualProjection::from_markdown(source);
    let text = projection.text().trim_end_matches(['\r', '\n']);
    if text.trim().is_empty() {
        source.to_owned()
    } else {
        text.to_owned()
    }
}

pub(crate) fn set_wysiwyg_document_accessibility(ui: &Ui, title: &str) {
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
        node.set_role(egui::accesskit::Role::Document);
        node.set_label(format!("{title} 所见即所得文档"));
    });
}

pub(crate) fn accessible_document_remainder(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    active_id: BlockId,
) -> String {
    const MAX_ACCESSIBLE_REMAINDER_CHARS: usize = 1_048_576;

    let mut output = String::new();
    let mut remaining = MAX_ACCESSIBLE_REMAINDER_CHARS;
    for block in blocks.iter().filter(|block| block.id != active_id) {
        if remaining == 0 {
            break;
        }
        let text = accessible_markdown_block_text(&source[block.range.clone()]);
        if text.is_empty() {
            continue;
        }
        if remaining > 0 {
            output.push('\n');
            remaining = remaining.saturating_sub(1);
        }
        let mut appended = 0usize;
        for character in text.chars().take(remaining) {
            output.push(character);
            appended += 1;
        }
        remaining = remaining.saturating_sub(appended);
    }
    output
}

pub(crate) fn append_accessible_text_runs(
    ui: &mut Ui,
    parent_id: egui::Id,
    salt: (u64, BlockId),
    text: &str,
    bounds: egui::Rect,
) {
    const MAX_TEXT_RUN_CHARS: usize = 255;

    let mut chunk = String::new();
    let mut chunk_chars = 0usize;
    let mut chunk_index = 0usize;
    for character in text.chars() {
        chunk.push(character);
        chunk_chars += 1;
        if chunk_chars == MAX_TEXT_RUN_CHARS {
            append_accessible_text_run(ui, parent_id, salt, chunk_index, &chunk, bounds);
            chunk.clear();
            chunk_chars = 0;
            chunk_index += 1;
        }
    }
    if !chunk.is_empty() {
        append_accessible_text_run(ui, parent_id, salt, chunk_index, &chunk, bounds);
    }
}

fn append_accessible_text_run(
    ui: &mut Ui,
    parent_id: egui::Id,
    salt: (u64, BlockId),
    chunk_index: usize,
    text: &str,
    bounds: egui::Rect,
) {
    let child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("wysiwyg-accessible-remainder", salt, chunk_index))
            .max_rect(bounds)
            .accessibility_parent(parent_id),
    );
    child
        .ctx()
        .accesskit_node_builder(child.unique_id(), |node| {
            node.set_role(egui::accesskit::Role::TextRun);
            node.set_value(text);
            node.set_character_lengths(
                text.chars()
                    .map(|character| character.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            node.set_text_direction(egui::accesskit::TextDirection::LeftToRight);
            node.set_bounds(egui::accesskit::Rect {
                x0: bounds.min.x.into(),
                y0: bounds.min.y.into(),
                x1: bounds.max.x.into(),
                y1: bounds.max.y.into(),
            });
        });
}

pub(crate) fn set_markdown_preview_accessibility(response: &egui::Response, source: &str) {
    let accessible_text = accessible_markdown_block_text(source);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &accessible_text));
}

pub(crate) fn paint_inline_code_delimiters(
    ui: &Ui,
    galley: &egui::Galley,
    galley_pos: egui::Pos2,
    runs: &[crate::wysiwyg::VisualRun],
    palette: AppPalette,
) -> usize {
    let mut painted = 0usize;
    let mut char_index = 0usize;
    let mut run_index = 0usize;
    for placed_row in &galley.rows {
        let row_offset = galley_pos.to_vec2() + placed_row.pos.to_vec2();
        for glyph in &placed_row.glyphs {
            while runs
                .get(run_index)
                .is_some_and(|run| run.range.end <= char_index)
            {
                run_index += 1;
            }
            let is_delimiter = glyph.chr == '`'
                && runs.get(run_index).is_some_and(|run| {
                    run.range.contains(&char_index) && run.style.code && run.style.marker
                });
            if is_delimiter {
                let rect = glyph.logical_rect().translate(row_offset);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "`",
                    FontId::new(17.0, FontFamily::Monospace),
                    palette.accent,
                );
                painted += 1;
            }
            char_index += 1;
        }
        if placed_row.ends_with_newline {
            char_index += 1;
        }
    }
    painted
}

fn push_inline_code_background(
    shapes: &mut Vec<egui::Shape>,
    rect: Option<egui::Rect>,
    palette: AppPalette,
) {
    let Some(rect) = rect else {
        return;
    };
    let rect = rect.expand2(Vec2::new(3.0, WYSIWYG_INLINE_CODE_VERTICAL_PADDING));
    shapes.push(egui::Shape::rect_filled(rect, 4, palette.code_bg));
    shapes.push(egui::Shape::rect_stroke(
        rect,
        4,
        Stroke::new(0.75, palette.border),
        egui::StrokeKind::Inside,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Instant;

    use eframe::egui::{Context, TextEdit};

    use crate::{
        native_preview::{
            MAX_GENERATED_SVG_CACHE_BYTES, MAX_GENERATED_SVG_CACHE_ENTRIES, cache_generated_svg,
        },
        presentation::{WYSIWYG_INLINE_CODE_LINE_HEIGHT, apply_theme, install_fonts},
    };

    #[test]
    fn full_audit_inline_triple_backticks_are_not_a_fenced_block() {
        for source in ["```inline```", "```a`b\nplain text"] {
            assert!(
                !markdown::events_with_references(source, &Default::default()).any(|(event, _)| {
                    matches!(
                        event,
                        pulldown_cmark::Event::Start(pulldown_cmark::Tag::CodeBlock(
                            pulldown_cmark::CodeBlockKind::Fenced(_)
                        ))
                    )
                }),
                "fixture is a CommonMark paragraph, not fenced code"
            );
            assert!(
                !is_fenced_code_block(source),
                "inline code/invalid fence must retain paragraph editing behavior: {source}"
            );
        }
    }

    #[test]
    fn full_audit_display_math_height_is_independent_of_unused_parent_height() {
        let context = Context::default();
        egui_extras::install_image_loaders(&context);
        let cache = RenderCache::default();
        let source = "$$x$$";
        let index = markdown::BlockIndex::new(source);
        let mut heights = Vec::new();
        for height in [240.0, 800.0] {
            let mut measured = 0.0;
            for _ in 0..3 {
                let _ = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(420.0, height),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(400.0);
                        let document =
                            cache.prepare_document(1, source, index.blocks(), index.references());
                        let block = document.block(
                            ui.ctx(),
                            &index.blocks()[0],
                            ui.available_width(),
                            false,
                            Path::new("."),
                        );
                        measured = block.show(ui).rect.height();
                    },
                );
            }
            heights.push(measured);
        }
        assert!(
            (heights[0] - heights[1]).abs() < 1.0,
            "the same one-symbol formula consumes different heights: {heights:?}"
        );
    }

    #[test]
    #[ignore = "manual single-paragraph rich-text layout scaling measurement"]
    fn full_audit_measures_single_rich_paragraph() {
        use std::hint::black_box;
        for spans in [2_000usize, 4_000, 8_000, 16_000] {
            let source = "`中` ".repeat(spans);
            let projection = VisualProjection::from_markdown(&source);
            let visual = projection.text();
            let runs = projection.runs_for(visual).len();
            let context = Context::default();
            let mut samples = Vec::new();
            for pass in 0..4 {
                let _ = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 400.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let started = Instant::now();
                        let galley = wysiwyg_layout(
                            ui,
                            visual,
                            &projection,
                            560.0,
                            app_palette(false),
                            true,
                        );
                        let elapsed = started.elapsed();
                        assert_eq!(galley.text(), visual);
                        black_box(galley);
                        if pass > 0 {
                            samples.push(elapsed);
                        }
                    },
                );
            }
            samples.sort();
            eprintln!(
                "spans={spans}, source_bytes={}, visual_chars={}, runs={runs}, median_layout_ms={:.3}, samples={samples:?}",
                source.len(),
                visual.chars().count(),
                samples[1].as_secs_f64() * 1000.0
            );
        }
    }

    #[test]
    fn rich_layout_retains_unicode_text_and_style_sections() {
        let context = Context::default();
        install_fonts(&context);
        let source = "**你🙂** plain `é中` tail **Z**";
        let projection = VisualProjection::from_markdown(source);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            let galley = wysiwyg_layout(
                ui,
                projection.text(),
                &projection,
                560.0,
                app_palette(false),
                true,
            );
            assert_eq!(galley.text(), "你🙂 plain é中 tail Z");
            let sections = &galley.job.sections;
            let parts = sections
                .iter()
                .map(|section| {
                    &galley.job.text[section.byte_range.start.0..section.byte_range.end.0]
                })
                .collect::<Vec<_>>();
            assert_eq!(parts, ["你🙂", " plain ", "é中", " tail ", "Z"]);
            assert_eq!(
                sections[0].format.font_id.family,
                FontFamily::Name(crate::presentation::WYSIWYG_STRONG_FAMILY.into())
            );
            assert_eq!(sections[1].format.font_id.family, FontFamily::Proportional);
            assert_eq!(sections[2].format.font_id.family, FontFamily::Monospace);
            assert_eq!(sections[3].format.font_id.family, FontFamily::Proportional);
            assert_eq!(
                sections[4].format.font_id.family,
                FontFamily::Name(crate::presentation::WYSIWYG_STRONG_FAMILY.into())
            );
        });
    }

    #[test]
    fn cloned_cache_handles_share_heights_without_mixing_layout_inputs() {
        let source = "A rendered paragraph";
        let block_id = markdown::blocks(source)[0].id;
        let cache = RenderCache::default();
        let view_cache = cache.clone();
        let key = HybridBlockLayoutKey::new(1, block_id, 400.0, false);
        cache.remember_height(key, source, 73.0);

        assert_eq!(view_cache.block_height(key, source, 400.0, false), 73.0);
        for other_key in [
            HybridBlockLayoutKey::new(2, block_id, 400.0, false),
            HybridBlockLayoutKey::new(1, block_id, 401.0, false),
            HybridBlockLayoutKey::new(1, block_id, 400.0, true),
        ] {
            assert_eq!(
                view_cache.block_height(other_key, source, 400.0, false),
                42.0,
            );
        }
    }

    #[test]
    fn source_changes_invalidate_measurements_even_when_block_identity_and_length_are_unchanged() {
        let original = "abcdefghijklmnopq";
        let same_length_change = "qrstuvwxyzabcdefg";
        let multiline_change = "abcdefgh\nijklmnop";
        let mut index = markdown::BlockIndex::new(original);
        let block_id = index.blocks()[0].id;
        let key = HybridBlockLayoutKey::new(1, block_id, 400.0, false);
        let cache = RenderCache::default();
        cache.remember_height(key, original, 73.0);
        assert_eq!(cache.block_height(key, original, 400.0, false), 73.0);

        for (source, estimated_height, measured_height) in [
            (same_length_change, 42.0, 91.0),
            (multiline_change, 64.0, 115.0),
        ] {
            assert_eq!(source.len(), original.len());
            index.update(source);
            assert_eq!(index.blocks().len(), 1);
            assert_eq!(index.blocks()[0].id, block_id);
            assert_eq!(
                cache.block_height(key, source, 400.0, false),
                estimated_height,
            );

            cache.remember_height(key, source, measured_height);
            assert_eq!(
                cache.block_height(key, source, 400.0, false),
                measured_height,
            );
        }

        // Updating one layout replaces its measurement instead of retaining a
        // separate cached height for every historical source string.
        assert_eq!(cache.block_height(key, original, 400.0, false), 42.0);
        assert_eq!(
            cache.block_height(key, same_length_change, 400.0, false),
            42.0,
        );
    }

    #[test]
    fn toc_height_is_invalidated_when_other_heading_blocks_change() {
        let original = "[TOC]\n\n# First";
        let updated = "[TOC]\n\n# First\n\n# Second\n\n# Third";
        let mut index = markdown::BlockIndex::new(original);
        let toc_id = index.blocks()[0].id;
        let cache = RenderCache::default();
        let context = Context::default();
        let prepared = cache.prepare_document(1, original, index.blocks(), index.references());
        let before = prepared.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        before.remember_height(73.0);
        assert_eq!(before.estimated_height(), 73.0);

        index.update(updated);
        assert_eq!(index.blocks()[0].id, toc_id);
        assert_ne!(
            markdown::toc_preview_markdown(original),
            markdown::toc_preview_markdown(updated),
        );
        let prepared = cache.prepare_document(1, updated, index.blocks(), index.references());
        let after = prepared.block(&context, &index.blocks()[0], 400.0, false, Path::new("."));
        assert!(after.is_generated());
        assert_ne!(before.preview_source(), after.preview_source());
        assert_ne!(after.estimated_height(), 73.0);
        after.remember_height(147.0);
        assert_eq!(after.estimated_height(), 147.0);
    }

    #[test]
    fn forgetting_a_document_discards_all_its_layouts_and_preserves_other_documents() {
        let source = "A rendered paragraph";
        let block_id = markdown::blocks(source)[0].id;
        let cache = RenderCache::default();
        let view_cache = cache.clone();
        let layouts = [
            (HybridBlockLayoutKey::new(1, block_id, 400.0, false), 400.0),
            (HybridBlockLayoutKey::new(1, block_id, 401.0, false), 401.0),
            (HybridBlockLayoutKey::new(1, block_id, 400.0, true), 400.0),
        ];
        for (key, _) in layouts {
            cache.remember_height(key, source, 73.0);
        }
        let other_document = HybridBlockLayoutKey::new(2, block_id, 400.0, false);
        cache.remember_height(other_document, source, 91.0);

        view_cache.forget_document(1);
        for (key, width) in layouts {
            assert_eq!(cache.block_height(key, source, width, false), 42.0);
        }
        assert_eq!(
            cache.block_height(other_document, source, 400.0, false),
            91.0,
        );

        cache.forget_document(1);
        cache.forget_document(3);
        assert_eq!(
            view_cache.block_height(other_document, source, 400.0, false),
            91.0,
        );
    }

    #[test]
    fn height_eviction_happens_at_the_next_frame_after_exceeding_the_budget() {
        let source = "Paragraph";
        let block_id = markdown::blocks(source)[0].id;
        let cache = RenderCache::default();
        let first_key = HybridBlockLayoutKey::new(0, block_id, 400.0, false);
        for document_id in 0..MAX_HYBRID_BLOCK_HEIGHT_CACHE as u64 {
            cache.remember_height(
                HybridBlockLayoutKey::new(document_id, block_id, 400.0, false),
                source,
                73.0,
            );
        }
        cache.begin_frame();
        assert_eq!(cache.block_height(first_key, source, 400.0, false), 73.0);

        cache.remember_height(
            HybridBlockLayoutKey::new(MAX_HYBRID_BLOCK_HEIGHT_CACHE as u64, block_id, 400.0, false),
            source,
            73.0,
        );
        assert_eq!(cache.block_height(first_key, source, 400.0, false), 73.0);
        cache.begin_frame();
        assert_eq!(cache.block_height(first_key, source, 400.0, false), 42.0);
    }

    #[test]
    fn fenced_code_header_keeps_copy_button_clear_of_the_first_line() {
        let context = Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(400.0);
            let preview = show_native_block_preview(
                ui,
                "```\nfirst line\n```",
                Path::new("."),
                false,
                &mut HashMap::new(),
                app_palette(false),
            );
            let NativePointerMapping::Text(text) = preview.mapping else {
                panic!("code must use the actual text galley");
            };
            assert!(
                text.galley_pos.y >= preview.rect.top() + 33.0,
                "the copy control overlaps the first code line"
            );
        });
    }

    #[test]
    fn recognizes_only_commonmark_indented_fences() {
        assert!(is_fenced_code_block("```rust\nfn main() {}\n```"));
        assert!(is_fenced_code_block("   ~~~\ncode\n   ~~~"));
        assert!(!is_fenced_code_block("    ```\nnot a fence"));
        assert!(!is_fenced_code_block("\t```\nnot a fence"));
    }

    #[test]
    fn bounds_the_generated_svg_cache() {
        let context = Context::default();
        let mut cache = HashMap::new();
        for index in 0..=MAX_GENERATED_SVG_CACHE_ENTRIES {
            cache_generated_svg(
                &context,
                &mut cache,
                format!("key-{index}"),
                Arc::from([index as u8]),
            );
        }
        assert_eq!(cache.len(), MAX_GENERATED_SVG_CACHE_ENTRIES);
        assert!(cache.contains_key(&format!("key-{MAX_GENERATED_SVG_CACHE_ENTRIES}")));
    }

    #[test]
    fn bounds_the_generated_svg_cache_by_bytes() {
        let context = Context::default();
        let mut cache = HashMap::new();
        let shared = Arc::<[u8]>::from(vec![0; MAX_GENERATED_SVG_CACHE_BYTES / 4]);
        for index in 0..4 {
            cache_generated_svg(
                &context,
                &mut cache,
                format!("large-{index}"),
                shared.clone(),
            );
        }
        assert_eq!(cache.len(), 4);

        assert!(cache_generated_svg(
            &context,
            &mut cache,
            "replacement".to_owned(),
            Arc::from([1]),
        ));
        assert!(cache.len() <= 4);
        assert!(cache.contains_key("replacement"));
    }

    #[test]
    fn wysiwyg_editor_matches_editing_typography_to_markdown_blocks() {
        let heading = VisualProjection::from_markdown("# Title");
        assert_eq!(heading.text(), "Title");
        assert!(heading.runs_for(heading.text())[0].style.heading == 1);

        let code = VisualProjection::from_markdown("```rust\nfn main() {}\n```");
        assert_eq!(code.text(), "fn main() {}");
        assert!(code.runs_for(code.text())[0].style.code);
    }

    #[test]
    fn native_projection_covers_every_markdown_block_kind() {
        for source in [
            "plain text",
            "before `code` after",
            "# heading with `code`",
            "**bold `code`** and *emphasis*",
            "脚注引用[^1]",
            "soft\nline",
            "hard  \nline",
            "before <span>重点</span> after",
            "<div data-value=\"a > b\">文字</div>",
            "- list with `code`",
            "[link](note.md) and `code`",
            "![image](image.png) and `code`",
            "```rust\ncode\n```",
            "<img alt=\"diagram\" src=\"image.png\">",
        ] {
            let projection = VisualProjection::from_markdown(source);
            assert!(!projection.text().is_empty(), "{source}");
            assert_eq!(
                projection.visual_char_range(source, 0..source.chars().count()),
                0..projection.text().chars().count(),
                "{source}"
            );
        }
    }

    #[test]
    fn rendered_media_hit_testing_exposes_only_exact_atomic_boundaries() {
        let mapping = NativePointerMapping::Atomic {
            source_range: 3..17,
        };
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 120.0));
        assert_eq!(
            mapping.source_byte_at_position("0123456789abcdefghijkl", rect, egui::pos2(20.0, 30.0)),
            3
        );
        assert_eq!(
            mapping.source_byte_at_position(
                "0123456789abcdefghijkl",
                rect,
                egui::pos2(200.0, 110.0)
            ),
            17
        );
    }

    #[test]
    fn overlapping_pointer_slop_prefers_the_region_actually_under_the_pointer() {
        let source = "aaa\n\nbbb";
        let blocks = markdown::blocks(source);
        let regions = [
            HybridPointerRegion {
                block_id: blocks[0].id,
                source_range: blocks[0].range.clone(),
                rect: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 10.0)),
                atomic_range: None,
                mapping: NativePointerMapping::Atomic { source_range: 0..3 },
            },
            HybridPointerRegion {
                block_id: blocks[1].id,
                source_range: blocks[1].range.clone(),
                rect: egui::Rect::from_min_max(egui::pos2(0.0, 11.0), egui::pos2(100.0, 21.0)),
                atomic_range: None,
                mapping: NativePointerMapping::Atomic { source_range: 0..3 },
            },
        ];

        assert_eq!(
            hybrid_pointer_hit(source, &regions, egui::pos2(50.0, 9.5)).map(|(block, _)| block),
            Some(blocks[0].id)
        );
        assert_eq!(
            hybrid_pointer_hit(source, &regions, egui::pos2(50.0, 11.5)).map(|(block, _)| block),
            Some(blocks[1].id)
        );
    }

    #[test]
    #[ignore = "manual large-document WYSIWYG frame measurement"]
    fn measures_large_native_wysiwyg_layout_cost() {
        for block_count in [1_000usize, 5_000, 20_000] {
            let source = (0..block_count)
                .map(|index| format!("Paragraph {index} with **bold** and 中文。"))
                .collect::<Vec<_>>()
                .join("\n\n");
            let blocks = markdown::blocks(&source);
            let context = Context::default();
            install_fonts(&context);
            apply_theme(&context, false);
            let started = Instant::now();
            let mut rendered = 0usize;
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1_280.0, 800.0),
                    )),
                    ..egui::RawInput::default()
                },
                |ui| {
                    ui.set_width(1_280.0);
                    let mut svg_cache = HashMap::new();
                    for block in &blocks {
                        let block_source = &source[block.range.clone()];
                        let estimated = estimated_hybrid_block_height(
                            block_source,
                            ui.available_width(),
                            false,
                        );
                        let predicted = egui::Rect::from_min_size(
                            ui.next_widget_position(),
                            egui::vec2(ui.available_width(), estimated),
                        );
                        if ui
                            .clip_rect()
                            .expand2(egui::vec2(0.0, 800.0))
                            .intersects(predicted)
                        {
                            rendered += 1;
                            let _ = show_native_block_preview(
                                ui,
                                block_source,
                                Path::new("."),
                                false,
                                &mut svg_cache,
                                app_palette(false),
                            );
                        } else {
                            ui.add_space(estimated);
                        }
                    }
                },
            );
            eprintln!(
                "virtualized native WYSIWYG layout: {block_count} blocks ({rendered} rendered) in {:?}",
                started.elapsed()
            );
        }
    }

    #[test]
    fn inline_code_preview_click_maps_to_the_actual_glyph() {
        use egui::RawInput;

        let context = Context::default();
        let source = "PRE `ab` POST";
        let mut source_byte = None;
        let _ = context.run_ui(RawInput::default(), |ui| {
            ui.set_width(720.0);
            let preview = show_text_projection_preview(ui, source, app_palette(false));
            let visual_index = preview
                .projection
                .text()
                .find("ab")
                .expect("code should be visible")
                + 1;
            let position = preview.galley_pos
                + preview
                    .galley
                    .pos_from_cursor(CCursor::new(visual_index))
                    .center()
                    .to_vec2();
            source_byte = Some(preview.source_byte_at_position(source, position));
        });

        let source_byte = source_byte.expect("preview should be measured");
        let source_cursor = source[..source_byte].chars().count();
        assert_eq!(source_cursor, 6);
        assert_eq!(
            VisualProjection::from_markdown_with_selection(
                source,
                Some(source_cursor..source_cursor),
            )
            .text(),
            source
        );
    }

    #[test]
    fn native_task_checkbox_hit_maps_to_the_source_marker() {
        use egui::RawInput;

        let context = Context::default();
        let source = "- [ ] 待办事项";
        let mut updated = None;
        let _ = context.run_ui(RawInput::default(), |ui| {
            ui.set_width(720.0);
            let preview = show_text_projection_preview(ui, source, app_palette(false));
            let position = preview.galley_pos
                + preview
                    .galley
                    .pos_from_cursor(CCursor::new(0))
                    .center()
                    .to_vec2();
            let source_byte = preview.source_byte_at_position(source, position);
            updated = markdown::toggle_task_marker_at(source, source_byte);
        });
        assert_eq!(updated.as_deref(), Some("- [x] 待办事项"));
    }

    #[test]
    fn code_copy_control_emits_the_complete_code_body() {
        use egui::{Event, Modifiers, OutputCommand, PointerButton, RawInput, Rect, pos2, vec2};

        let context = Context::default();
        let code_rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(300.0, 90.0));
        let click = pos2(274.0, 30.0);
        let block_id = markdown::blocks("code")[0].id;
        let _ = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 140.0))),
                ..RawInput::default()
            },
            |ui| {
                ui.set_width(340.0);
                assert!(!show_code_copy_button(
                    ui,
                    (7, block_id),
                    code_rect,
                    "fn main() {}\n中文",
                    app_palette(false),
                ));
            },
        );
        let output = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(360.0, 140.0))),
                events: vec![
                    Event::PointerMoved(click),
                    Event::PointerButton {
                        pos: click,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    Event::PointerButton {
                        pos: click,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
                ..RawInput::default()
            },
            |ui| {
                ui.set_width(340.0);
                assert!(show_code_copy_button(
                    ui,
                    (7, block_id),
                    code_rect,
                    "fn main() {}\n中文",
                    app_palette(false),
                ));
            },
        );
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, OutputCommand::CopyText(text) if text == "fn main() {}\n中文")
        }));
    }

    #[test]
    fn accessible_preview_text_covers_non_active_markdown_blocks() {
        assert_eq!(accessible_markdown_block_text("- 项目"), "• 项目");
        let footnote = accessible_markdown_block_text("脚注引用[^1]");
        assert!(footnote.contains('1'));
        assert!(!footnote.contains("[^1]"));
        assert!(accessible_markdown_block_text("```rust\nfn main() {}\n```").contains("fn main"));
    }

    #[test]
    fn inactive_markdown_preview_registers_its_accessible_text() {
        use egui::{RawInput, Sense, vec2};

        let context = Context::default();
        context.enable_accesskit();
        let output = context.run_ui(RawInput::default(), |ui| {
            ui.push_id("accessible-wysiwyg-document", |ui| {
                set_wysiwyg_document_accessibility(ui, "测试.md");
                ui.label("文档开头");
                let response = ui.allocate_response(vec2(240.0, 40.0), Sense::click());
                set_markdown_preview_accessibility(
                    &response,
                    "后续列表与 `code` RETEST-END-20260813",
                );
            });
        });
        let update = output
            .platform_output
            .accesskit_update
            .expect("AccessKit should receive a tree update");

        assert!(
            update
                .nodes
                .iter()
                .any(|(_, node)| node.role() == egui::accesskit::Role::Document)
        );
        assert!(update.nodes.iter().any(|(_, node)| {
            node.value()
                .is_some_and(|value| value.contains("RETEST-END-20260813"))
        }));
    }

    #[test]
    fn focused_wysiwyg_editor_exposes_the_remaining_document_as_text_runs() {
        use egui::{Id, RawInput};

        let context = Context::default();
        context.enable_accesskit();
        let editor_id = Id::new("focused-wysiwyg-accessibility-regression");
        let source = "ACCESS-FINAL\n\nACCESS-FINAL-END-20260813";
        let blocks = markdown::blocks(source);
        let remainder = accessible_document_remainder(source, &blocks, blocks[0].id);
        assert!(remainder.contains("ACCESS-FINAL-END-20260813"));
        assert!(!remainder.contains("ACCESS-FINAL\n"));

        let output = context.run_ui(RawInput::default(), |ui| {
            ui.memory_mut(|memory| memory.request_focus(editor_id));
            let mut active = "ACCESS-FINAL".to_owned();
            let output = TextEdit::multiline(&mut active).id(editor_id).show(ui);
            append_accessible_text_runs(
                ui,
                editor_id,
                (7, blocks[0].id),
                &remainder,
                output.response.rect,
            );
        });
        let update = output
            .platform_output
            .accesskit_update
            .expect("AccessKit should receive a tree update");
        assert_eq!(update.focus, editor_id.accesskit_id());

        let editor = update
            .nodes
            .iter()
            .find(|(id, _)| *id == editor_id.accesskit_id())
            .map(|(_, node)| node)
            .expect("focused editor node should exist");
        let remainder_node = update
            .nodes
            .iter()
            .find(|(_, node)| {
                node.value()
                    .is_some_and(|value| value.contains("ACCESS-FINAL-END-20260813"))
            })
            .expect("remainder text run should exist");
        assert!(editor.children().contains(&remainder_node.0));
    }

    #[test]
    fn active_inline_code_paints_both_visible_delimiters() {
        use egui::RawInput;

        let context = Context::default();
        let mut painted = None;
        let _ = context.run_ui(RawInput::default(), |ui| {
            let source = "PRE `ab` POST";
            let content_start = source.find("ab").unwrap();
            let source_cursor = source[..content_start + 1].chars().count();
            let projection = VisualProjection::from_markdown_with_selection(
                source,
                Some(source_cursor..source_cursor),
            );
            let runs = projection.runs_for(projection.text());
            let galley = wysiwyg_layout(
                ui,
                projection.text(),
                &projection,
                480.0,
                app_palette(false),
                true,
            );
            painted = Some(paint_inline_code_delimiters(
                ui,
                &galley,
                egui::Pos2::ZERO,
                &runs,
                app_palette(false),
            ));
        });

        assert_eq!(painted, Some(2));
    }

    #[test]
    fn inline_code_text_and_chip_are_centered_on_the_body_line() {
        use egui::RawInput;

        let context = Context::default();
        let mut measured = None;
        let _ = context.run_ui(RawInput::default(), |ui| {
            let source = "before `code` after";
            let projection = VisualProjection::from_markdown(source);
            let runs = projection.runs_for(projection.text());
            let galley = wysiwyg_layout(
                ui,
                projection.text(),
                &projection,
                480.0,
                app_palette(false),
                true,
            );
            let code_start = projection.text().find("code").unwrap();
            let code_start = projection.text()[..code_start].chars().count();
            let row = galley
                .rows
                .iter()
                .find(|row| code_start < row.glyphs.len())
                .expect("single-line inline code should have a row");
            let glyph = &row.glyphs[code_start];
            let glyph_rect = glyph.logical_rect().translate(row.pos.to_vec2());
            let shapes = rounded_inline_code_backgrounds(
                &galley,
                egui::Pos2::ZERO,
                &runs,
                app_palette(false),
            );
            let egui::Shape::Rect(chip) = &shapes[0] else {
                panic!("inline code background should start with a rectangle");
            };
            measured = Some((row.rect(), glyph_rect, chip.rect));
        });

        let (row, glyph, chip) = measured.expect("layout should be measured");
        assert!((glyph.center().y - row.center().y).abs() <= 0.5);
        assert!((chip.center().y - row.center().y).abs() <= 0.5);
        assert_eq!(glyph.height(), WYSIWYG_INLINE_CODE_LINE_HEIGHT);
        assert_eq!(
            chip.height(),
            WYSIWYG_INLINE_CODE_LINE_HEIGHT + 2.0 * WYSIWYG_INLINE_CODE_VERTICAL_PADDING
        );
    }

    #[test]
    fn cross_block_selection_intersects_each_unicode_block_in_source_coordinates() {
        let source = "段落 alpha\n\n```\n代码 beta\n```";
        let blocks = markdown::blocks(source);
        assert_eq!(blocks.len(), 2);
        let selection_start = source[..source.find("alpha").unwrap()].chars().count() + 2;
        let selection_end = source[..source.find("beta").unwrap()].chars().count() + 2;
        let selection = selection_start..selection_end;

        let first = hybrid_region_selection(source, &blocks[0].range, &selection).unwrap();
        let second = hybrid_region_selection(source, &blocks[1].range, &selection).unwrap();
        assert_eq!(first.start, "段落 al".chars().count());
        assert_eq!(first.end, source[blocks[0].range.clone()].chars().count());
        assert_eq!(second.start, 0);
        assert!(second.end > "```\n代码 b".chars().count());
    }
}
