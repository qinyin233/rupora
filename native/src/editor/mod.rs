//! Stateful editor surface. Owns input, selection, IME and per-document view state.
//! External effects are returned to the shell after document edits have committed.
use crate::{
    document::{Document, EditKind},
    editing::{self, MarkdownCommand, char_to_byte},
    editor_buffer::{TrackingTextBuffer, set_accessible_label},
    markdown::{self, BlockId},
    presentation::*,
    rendering::*,
    wysiwyg::{
        VisualProjection, complete_bare_fenced_code_after_typing, complete_fenced_code_on_enter,
        complete_indented_code_on_enter, complete_setext_heading_on_enter, complete_visual_enter,
        consume_paired_fenced_code_closer, fenced_code_content, fenced_code_language,
        move_across_hidden_inline_code_boundary, paragraph_after_fenced_code,
    },
};
use eframe::egui::{
    self, Align, FontId, Key, Layout, Margin, ScrollArea, Stroke, TextEdit, Ui, Vec2,
    text::{CCursor, CCursorRange},
};
use std::{collections::HashMap, path::Path, sync::Arc};
mod hybrid;
mod input;
mod preview;
mod source;
pub(crate) use input::cursor_range_to_char_range;
use input::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ViewMode {
    Edit,
    Split,
    #[default]
    Hybrid,
    Preview,
}

pub(crate) struct EditorOptions<'a> {
    pub mode: ViewMode,
    pub dark: bool,
    pub base_path: &'a Path,
}

pub(crate) enum EditorCommand {
    PasteImage,
    EditTable,
}
enum ContextCommand {
    Undo,
    Redo,
    Format(MarkdownCommand),
    PasteImage,
    EditTable,
}

#[derive(Default)]
pub(crate) struct EditorOutput {
    pub notice: String,
    pub destination: Option<String>,
    pub commands: Vec<EditorCommand>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct EditorBookmark {
    pub cursor: Option<CCursorRange>,
    pub scroll_ratio: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SplitScrollDriver {
    #[default]
    Editor,
    Preview,
}

#[derive(Clone, Copy, Debug, Default)]
struct PaneScroll {
    offset: f32,
    maximum: f32,
    hovered: bool,
}

#[derive(Clone, Copy)]
struct DocumentViewState {
    cursor: Option<CCursorRange>,
    scroll_ratio: f32,
}

#[derive(Clone, Debug)]
struct HybridImeSession {
    document_id: u64,
    block_id: BlockId,
    base_source: String,
    visual_content: String,
}

#[derive(Clone, Copy, Debug)]
struct HybridPointerAnchor {
    document_id: u64,
    block_id: BlockId,
    source_char: usize,
}

#[derive(Clone, Copy, Debug)]
struct HybridCrossSelection {
    document_id: u64,
    cursor: CCursorRange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ImeFrameAction {
    #[default]
    None,
    Preedit,
    Commit,
    Cancel,
}

#[derive(Default)]
pub(crate) struct EditorSurface {
    editor_cursor: Option<CCursorRange>,
    pending_editor_cursor: Option<CCursorRange>,
    editor_widget_id: Option<egui::Id>,
    deferred_history_action: Option<bool>,
    document_views: HashMap<u64, DocumentViewState>,
    hybrid_active: Option<(u64, BlockId)>,
    hybrid_ime_session: Option<HybridImeSession>,
    hybrid_pointer_anchor: Option<HybridPointerAnchor>,
    hybrid_cross_selection: Option<HybridCrossSelection>,
    split_scroll_ratio: f32,
    split_scroll_driver: SplitScrollDriver,
    split_editor_maximum: f32,
    split_preview_maximum: f32,
    render_cache: RenderCache,
}

impl EditorSurface {
    pub(crate) fn cancel_focus_request(&mut self) {
        self.pending_editor_cursor = None;
    }
    pub(crate) fn clear_selection(&mut self) {
        self.editor_cursor = None;
        self.pending_editor_cursor = None;
    }
    pub(crate) fn cursor(&self) -> Option<CCursorRange> {
        self.editor_cursor
    }
    pub(crate) fn widget_id(&self) -> Option<egui::Id> {
        self.editor_widget_id
    }
    pub(crate) fn selection(&self, document: &Document) -> std::ops::Range<usize> {
        self.editor_cursor
            .map(cursor_range_to_char_range)
            .map(|range| clamp_char_range(&document.content, range))
            .unwrap_or_else(|| {
                let end = document.content.chars().count();
                end..end
            })
    }
    pub(crate) fn queue_editor_selection(&mut self, range: std::ops::Range<usize>) {
        self.select_cursor(CCursorRange::two(
            CCursor::new(range.start),
            CCursor::new(range.end),
        ));
    }
    pub(crate) fn select_cursor(&mut self, cursor: CCursorRange) {
        self.editor_cursor = Some(cursor);
        self.pending_editor_cursor = Some(cursor);
    }
    pub(crate) fn defer_history(&mut self, redo: bool) {
        self.deferred_history_action = Some(redo);
    }
    pub(crate) fn change_mode(&mut self, mode: ViewMode) {
        self.pending_editor_cursor = self.editor_cursor;
        if mode != ViewMode::Hybrid {
            self.hybrid_ime_session = None;
        }
    }
    pub(crate) fn invalidate_content(&mut self) {
        self.hybrid_active = None;
        self.hybrid_ime_session = None;
        self.hybrid_pointer_anchor = None;
        self.hybrid_cross_selection = None;
    }
    /// Reconciles input state after a command or deferred result changes a document.
    pub(crate) fn document_edited(
        &mut self,
        document: &Document,
        selection: Option<std::ops::Range<usize>>,
    ) {
        self.invalidate_content();
        let selection = selection.unwrap_or_else(|| self.selection(document));
        self.queue_editor_selection(clamp_char_range(&document.content, selection));
    }
    pub(crate) fn bookmark(&self) -> EditorBookmark {
        EditorBookmark {
            cursor: self.editor_cursor,
            scroll_ratio: self.split_scroll_ratio,
        }
    }
    pub(crate) fn remember(&mut self, document_id: u64) {
        self.document_views.insert(
            document_id,
            DocumentViewState {
                cursor: self.editor_cursor,
                scroll_ratio: self.split_scroll_ratio,
            },
        );
    }
    pub(crate) fn forget(&mut self, document_id: u64) {
        self.document_views.remove(&document_id);
        self.render_cache.forget_document(document_id);
    }
    pub(crate) fn bind_document(&mut self, document: Option<&Document>, fallback: EditorBookmark) {
        self.invalidate_content();
        self.editor_widget_id = None;
        self.deferred_history_action = None;
        self.editor_cursor = None;
        self.pending_editor_cursor = None;
        self.split_scroll_ratio = 0.0;
        self.split_editor_maximum = 0.0;
        self.split_preview_maximum = 0.0;
        self.split_scroll_driver = SplitScrollDriver::Editor;
        let Some(document) = document else {
            return;
        };
        let saved = self
            .document_views
            .get(&document.id())
            .map(|v| EditorBookmark {
                cursor: v.cursor,
                scroll_ratio: v.scroll_ratio,
            })
            .unwrap_or(fallback);
        if let Some(mut cursor) = saved.cursor {
            let len = document.content.chars().count();
            cursor.primary.index.0 = cursor.primary.index.0.min(len);
            cursor.secondary.index.0 = cursor.secondary.index.0.min(len);
            self.select_cursor(cursor);
        }
        self.split_scroll_ratio = saved.scroll_ratio.clamp(0.0, 1.0);
    }
    pub(crate) fn apply_format(
        &mut self,
        document: &mut Document,
        command: MarkdownCommand,
    ) -> &'static str {
        let selection = self.selection(document);
        let mut next = selection.clone();
        document.edit(EditKind::Format, Some(selection.clone()), |text| {
            next = editing::apply_markdown_command(text, selection, command);
            Some(next.clone())
        });
        self.document_edited(document, Some(next));
        "已应用 Markdown 格式"
    }
    fn apply_context_command(
        &mut self,
        document: &mut Document,
        command: ContextCommand,
        effects: &mut EditorOutput,
    ) {
        match command {
            ContextCommand::Undo => effects.notice = self.apply_history(document, false).to_owned(),
            ContextCommand::Redo => effects.notice = self.apply_history(document, true).to_owned(),
            ContextCommand::Format(command) => {
                effects.notice = self.apply_format(document, command).to_owned()
            }
            ContextCommand::PasteImage => effects.commands.push(EditorCommand::PasteImage),
            ContextCommand::EditTable => effects.commands.push(EditorCommand::EditTable),
        }
    }
    pub(crate) fn apply_history(&mut self, document: &mut Document, redo: bool) -> &'static str {
        let outcome = if redo {
            document.redo()
        } else {
            document.undo()
        };
        let Some(outcome) = outcome else {
            return if redo {
                "没有可重做的操作"
            } else {
                "没有可撤销的操作"
            };
        };
        self.invalidate_content();
        if let Some(selection) = outcome.selection {
            self.queue_editor_selection(selection);
        } else {
            self.editor_cursor = None;
            self.pending_editor_cursor = None;
        }
        if redo { "已重做" } else { "已撤销" }
    }
    fn finish_history_action(&mut self, document: &mut Document, output: &mut EditorOutput) {
        if let Some(redo) = self.deferred_history_action.take() {
            output.notice = self.apply_history(document, redo).to_owned();
        }
    }
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        options: EditorOptions<'_>,
    ) -> EditorOutput {
        let mut output = EditorOutput::default();
        match options.mode {
            ViewMode::Edit => {
                self.edit_pane(ui, document, &options, &mut output, None);
            }
            ViewMode::Preview => {
                self.preview_pane(ui, document, &options, &mut output, None);
            }
            ViewMode::Hybrid => self.hybrid_pane(ui, document, &options, &mut output),
            ViewMode::Split => {
                let editor_target = (self.split_scroll_driver == SplitScrollDriver::Preview)
                    .then_some(self.split_scroll_ratio * self.split_editor_maximum);
                let preview_target = (self.split_scroll_driver == SplitScrollDriver::Editor)
                    .then_some(self.split_scroll_ratio * self.split_preview_maximum);
                let mut editor_scroll = PaneScroll::default();
                let mut preview_scroll = PaneScroll::default();
                let pane_bounds = ui.available_rect_before_wrap();
                let mut divider_x = pane_bounds.center().x;
                ui.columns(2, |columns| {
                    divider_x =
                        (columns[0].max_rect().right() + columns[1].max_rect().left()) * 0.5;
                    columns[0].push_id("source-pane", |ui| {
                        editor_scroll =
                            self.edit_pane(ui, document, &options, &mut output, editor_target);
                    });
                    columns[1].push_id("preview-pane", |ui| {
                        preview_scroll =
                            self.preview_pane(ui, document, &options, &mut output, preview_target);
                    });
                });
                ui.painter().vline(
                    divider_x,
                    pane_bounds.y_range(),
                    Stroke::new(1.0, app_palette(options.dark).border),
                );
                self.split_editor_maximum = editor_scroll.maximum;
                self.split_preview_maximum = preview_scroll.maximum;
                if editor_scroll.hovered {
                    self.split_scroll_driver = SplitScrollDriver::Editor;
                    self.split_scroll_ratio = scroll_ratio(editor_scroll);
                } else if preview_scroll.hovered {
                    self.split_scroll_driver = SplitScrollDriver::Preview;
                    self.split_scroll_ratio = scroll_ratio(preview_scroll);
                }
            }
        }
        output
    }
}

#[cfg(test)]
#[path = "../paragraph_layout_tests.rs"]
mod paragraph_layout_tests;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod session_tests;
