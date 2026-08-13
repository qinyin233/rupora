use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    app_state::{AppCommand, KeyBindings, PersistedState, ShortcutAction, ViewMode},
    diagnostics,
    document::{Document, EditKind},
    editing::{self, MarkdownCommand},
    editor_buffer::{TrackingTextBuffer, set_accessible_label},
    export,
    extensions::{self, ExtensionInvocation, ExtensionRegistry},
    instance::InstanceCoordinator,
    markdown::{self, BlockId, Heading},
    native_preview::{prepare_native_preview, render_math_widget},
    recovery::{RecoveryEntry, RecoveryStore},
    source_map::SourceMap,
    table::{self, MarkdownTable},
    updater::{self, UpdateInfo, UpdateStatus},
    workspace::{Workspace, WorkspaceEntry},
    wysiwyg::{
        VisualProjection, VisualStyle, complete_bare_fenced_code_after_typing,
        complete_fenced_code_on_enter, complete_visual_enter, consume_paired_fenced_code_closer,
        contains_footnote_reference, fenced_code_content, fenced_code_language,
        move_across_hidden_inline_code_boundary, paragraph_after_fenced_code,
    },
};
use eframe::{
    CreationContext, Frame, Storage,
    egui::{
        self, Align, Button, CentralPanel, Color32, Context, FontData, FontDefinitions, FontFamily,
        FontId, Key, Layout, Margin, Panel, RichText, ScrollArea, Stroke, TextEdit, TextStyle, Ui,
        Vec2, ViewportCommand,
        text::{CCursor, CCursorRange, LayoutJob, TextFormat},
    },
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event as MarkdownEvent, Parser, Tag, TagEnd};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

const APP_STATE_KEY: &str = "rupora-native-state";
const UI_EXPERIENCE_KEY: &str = "rupora-native-ui-experience";
const CURRENT_UI_EXPERIENCE: u32 = 3;
const WYSIWYG_STRONG_FAMILY: &str = "rupora-wysiwyg-strong";
const WYSIWYG_BODY_LINE_HEIGHT: f32 = 27.0;
const WYSIWYG_INLINE_CODE_LINE_HEIGHT: f32 = 20.0;
const WYSIWYG_INLINE_CODE_VERTICAL_PADDING: f32 = 2.0;

#[derive(Clone, Copy)]
struct AppPalette {
    canvas: Color32,
    surface: Color32,
    toolbar: Color32,
    sidebar: Color32,
    text: Color32,
    secondary: Color32,
    border: Color32,
    accent: Color32,
    accent_soft: Color32,
    code_bg: Color32,
    hover: Color32,
}

fn app_palette(dark: bool) -> AppPalette {
    if dark {
        AppPalette {
            canvas: Color32::from_rgb(24, 25, 30),
            surface: Color32::from_rgb(30, 31, 38),
            toolbar: Color32::from_rgb(27, 28, 34),
            sidebar: Color32::from_rgb(31, 32, 39),
            text: Color32::from_rgb(232, 233, 239),
            secondary: Color32::from_rgb(146, 150, 165),
            border: Color32::from_rgb(51, 53, 64),
            accent: Color32::from_rgb(139, 148, 255),
            accent_soft: Color32::from_rgb(50, 52, 81),
            code_bg: Color32::from_rgb(38, 39, 46),
            hover: Color32::from_rgb(42, 44, 53),
        }
    } else {
        AppPalette {
            canvas: Color32::from_rgb(246, 247, 250),
            surface: Color32::WHITE,
            toolbar: Color32::from_rgb(250, 250, 252),
            sidebar: Color32::from_rgb(242, 244, 247),
            text: Color32::from_rgb(30, 32, 38),
            secondary: Color32::from_rgb(102, 108, 121),
            border: Color32::from_rgb(222, 225, 232),
            accent: Color32::from_rgb(91, 95, 235),
            accent_soft: Color32::from_rgb(235, 236, 255),
            code_bg: Color32::from_rgb(244, 245, 247),
            hover: Color32::from_rgb(234, 237, 243),
        }
    }
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

struct TableEditorState {
    document: usize,
    table: MarkdownTable,
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

struct ExtensionJobResult {
    document_id: u64,
    before: String,
    invocation: ExtensionInvocation,
}

fn extension_document_index(documents: &[Document], document_id: u64) -> Option<usize> {
    documents
        .iter()
        .position(|document| document.id() == document_id)
}

pub struct RuporaApp {
    documents: Vec<Document>,
    active: Option<usize>,
    next_untitled_id: usize,
    state: PersistedState,
    status: String,
    preview_cache: CommonMarkCache,
    allow_close: bool,
    discard_recovery_on_exit: bool,
    recovery_store: RecoveryStore,
    last_recovery_write: Instant,
    recovery_error_reported: bool,
    editor_cursor: Option<CCursorRange>,
    pending_editor_cursor: Option<CCursorRange>,
    find_open: bool,
    find_query: String,
    replace_query: String,
    find_match_case: bool,
    find_focus_requested: bool,
    workspace: Option<Workspace>,
    hybrid_active: Option<(usize, BlockId)>,
    hybrid_ime_session: Option<HybridImeSession>,
    hybrid_pointer_anchor: Option<HybridPointerAnchor>,
    hybrid_cross_selection: Option<HybridCrossSelection>,
    external_conflicts: HashSet<PathBuf>,
    last_external_check: Instant,
    external_scan_error_reported: bool,
    command_palette_open: bool,
    command_query: String,
    command_focus_requested: bool,
    split_scroll_ratio: f32,
    split_scroll_driver: SplitScrollDriver,
    split_editor_maximum: f32,
    split_preview_maximum: f32,
    split_scroll_document: Option<usize>,
    shortcut_settings_open: bool,
    external_diff_view: Option<String>,
    generated_svg_cache: Rc<RefCell<HashMap<String, Arc<[u8]>>>>,
    table_editor: Option<TableEditorState>,
    instance_coordinator: Option<InstanceCoordinator>,
    update_receiver: Option<Receiver<Result<UpdateStatus, String>>>,
    extension_registry: ExtensionRegistry,
    extension_receiver: Option<Receiver<Result<ExtensionJobResult, String>>>,
    available_update: Option<UpdateInfo>,
    about_open: bool,
}

impl RuporaApp {
    pub fn new(creation_context: &CreationContext<'_>) -> Self {
        let startup_files = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .collect();
        Self::build(creation_context, startup_files, None)
    }

    pub fn new_with_instance(
        creation_context: &CreationContext<'_>,
        startup_files: Vec<PathBuf>,
        instance_coordinator: InstanceCoordinator,
    ) -> Self {
        Self::build(creation_context, startup_files, Some(instance_coordinator))
    }

    fn build(
        creation_context: &CreationContext<'_>,
        startup_files: Vec<PathBuf>,
        instance_coordinator: Option<InstanceCoordinator>,
    ) -> Self {
        install_fonts(&creation_context.egui_ctx);
        let mut state: PersistedState = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, APP_STATE_KEY))
            .unwrap_or_default();
        let ui_experience = creation_context
            .storage
            .and_then(|storage| eframe::get_value::<u32>(storage, UI_EXPERIENCE_KEY))
            .unwrap_or_default();
        if ui_experience < CURRENT_UI_EXPERIENCE {
            state.view_mode = ViewMode::Hybrid;
            state.show_outline = false;
        }
        if matches!(state.view_mode, ViewMode::Split | ViewMode::Preview) {
            state.view_mode = ViewMode::Hybrid;
        }
        apply_theme(&creation_context.egui_ctx, state.dark);
        let recovery_store = RecoveryStore::for_app("RUPORA");
        let recovered_entries = recovery_store.load();
        let workspace = state
            .workspace_root
            .as_ref()
            .and_then(|root| Workspace::open(root.clone()).ok());
        let extension_path = eframe::storage_dir("RUPORA")
            .unwrap_or_else(std::env::temp_dir)
            .join("extensions.json");
        let (extension_registry, extension_error) =
            match ExtensionRegistry::load(extension_path.clone()) {
                Ok(registry) => (registry, None),
                Err(error) => (ExtensionRegistry::disabled(extension_path), Some(error)),
            };

        let mut app = Self {
            documents: Vec::new(),
            active: None,
            next_untitled_id: 1,
            state,
            status: "纯 Rust 原生内核已就绪".to_owned(),
            preview_cache: CommonMarkCache::default(),
            allow_close: false,
            discard_recovery_on_exit: false,
            recovery_store,
            last_recovery_write: Instant::now(),
            recovery_error_reported: false,
            editor_cursor: None,
            pending_editor_cursor: None,
            find_open: false,
            find_query: String::new(),
            replace_query: String::new(),
            find_match_case: false,
            find_focus_requested: false,
            workspace,
            hybrid_active: None,
            hybrid_ime_session: None,
            hybrid_pointer_anchor: None,
            hybrid_cross_selection: None,
            external_conflicts: HashSet::new(),
            last_external_check: Instant::now(),
            external_scan_error_reported: false,
            command_palette_open: false,
            command_query: String::new(),
            command_focus_requested: false,
            split_scroll_ratio: 0.0,
            split_scroll_driver: SplitScrollDriver::Editor,
            split_editor_maximum: 0.0,
            split_preview_maximum: 0.0,
            split_scroll_document: None,
            shortcut_settings_open: false,
            external_diff_view: None,
            generated_svg_cache: Rc::new(RefCell::new(HashMap::new())),
            table_editor: None,
            instance_coordinator,
            update_receiver: None,
            extension_registry,
            extension_receiver: None,
            available_update: None,
            about_open: false,
        };
        if let Some(error) = extension_error {
            app.status = error;
        }

        match recovered_entries {
            Ok(entries) if !entries.is_empty() => {
                let recovered_count = entries.len();
                let mut warning_count = 0usize;
                let mut conflict_count = 0usize;
                for entry in entries {
                    let RecoveryEntry {
                        path,
                        content,
                        base_content,
                        encoding,
                        line_ending,
                        ..
                    } = entry;
                    let outcome = Document::recover(
                        path,
                        content,
                        base_content,
                        encoding.as_deref(),
                        line_ending.as_deref(),
                        app.next_untitled_id,
                    );
                    conflict_count += outcome.conflicts;
                    if let Some(warning) = outcome.warning {
                        warning_count += 1;
                        diagnostics::append_event("WARN", &warning).ok();
                    }
                    app.next_untitled_id += 1;
                    app.documents.push(outcome.document);
                }
                app.active = Some(0);
                app.status = format!(
                    "已恢复 {recovered_count} 个未保存文档；{warning_count} 项需要注意；{conflict_count} 处合并冲突"
                );
            }
            Ok(_) => {}
            Err(error) => {
                app.status = error;
                app.recovery_error_reported = true;
            }
        }

        if !startup_files.is_empty() {
            app.open_paths(startup_files);
        } else if app.documents.is_empty() {
            let session_files = app
                .state
                .session_files
                .iter()
                .filter(|path| path.is_file())
                .cloned()
                .collect::<Vec<_>>();
            if !session_files.is_empty() {
                app.open_paths(session_files);
                if let Some(active_path) = app.state.active_session_file.as_ref()
                    && let Some(index) = app
                        .documents
                        .iter()
                        .position(|document| document.path.as_ref() == Some(active_path))
                {
                    app.active = Some(index);
                }
                app.status = "已恢复上次会话".to_owned();
            }
        }
        if app.documents.is_empty() {
            app.new_document();
        }
        app.restore_active_view_state();
        app
    }

    fn store_active_view_state(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let Some(path) = self.documents[index].path.clone() else {
            return;
        };
        if let Some(cursor) = self.editor_cursor {
            self.state
                .cursor_positions
                .insert(path.clone(), cursor.primary.index.0);
        }
        self.state
            .scroll_positions
            .insert(path, self.split_scroll_ratio);
    }

    fn restore_active_view_state(&mut self) {
        self.editor_cursor = None;
        self.pending_editor_cursor = None;
        self.split_scroll_ratio = 0.0;
        let Some(index) = self.active else {
            return;
        };
        let Some(path) = self.documents[index].path.clone() else {
            return;
        };
        let saved_cursor = self.state.cursor_positions.get(&path).copied();
        let saved_scroll = self.state.scroll_positions.get(&path).copied();
        if let Some(cursor) = saved_cursor {
            let cursor = cursor.min(self.documents[index].content.chars().count());
            self.queue_editor_selection(cursor..cursor);
        }
        self.split_scroll_ratio = saved_scroll.unwrap_or(0.0).clamp(0.0, 1.0);
    }

    fn activate_document(&mut self, index: usize) {
        if index >= self.documents.len() || self.active == Some(index) {
            return;
        }
        self.store_active_view_state();
        self.active = Some(index);
        self.hybrid_active = None;
        self.hybrid_ime_session = None;
        self.split_scroll_document = Some(index);
        self.restore_active_view_state();
    }

    fn new_document(&mut self) {
        self.store_active_view_state();
        let document = Document::untitled(self.next_untitled_id);
        self.next_untitled_id += 1;
        self.documents.push(document);
        self.active = Some(self.documents.len() - 1);
        self.editor_cursor = None;
        self.pending_editor_cursor = None;
        self.hybrid_active = None;
        self.hybrid_ime_session = None;
        self.status = "已新建文档".to_owned();
    }

    fn open_dialog(&mut self) {
        let paths = FileDialog::new()
            .add_filter("Markdown", &["md", "markdown", "mdown", "mkd"])
            .add_filter("Text", &["txt"])
            .pick_files();
        if let Some(paths) = paths {
            self.open_paths(paths);
        }
    }

    fn open_folder_dialog(&mut self) {
        if let Some(path) = FileDialog::new().pick_folder() {
            self.open_workspace(path);
        }
    }

    fn open_workspace(&mut self, path: PathBuf) {
        match Workspace::open(path.clone()) {
            Ok(workspace) => {
                let suffix = if workspace.truncated {
                    "（文件过多，列表已截断）"
                } else {
                    ""
                };
                self.workspace = Some(workspace);
                self.state.workspace_root = Some(path.clone());
                self.state.show_sidebar = true;
                self.status = format!("已打开工作区：{}{suffix}", path.display());
            }
            Err(error) => self.show_error("打开工作区失败", &error),
        }
    }

    fn open_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if !is_markdown_path(&path) {
                self.status = format!("已忽略非 Markdown 文件：{}", path.display());
                continue;
            }

            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if let Some(index) = self.documents.iter().position(|document| {
                document
                    .path
                    .as_ref()
                    .map(|open_path| {
                        open_path
                            .canonicalize()
                            .unwrap_or_else(|_| open_path.clone())
                            == canonical
                    })
                    .unwrap_or(false)
            }) {
                self.activate_document(index);
                continue;
            }

            match Document::open(&path) {
                Ok(document) => {
                    self.status =
                        format!("已打开：{} · {}", path.display(), document.encoding.label());
                    self.remove_initial_placeholder();
                    self.documents.push(document);
                    self.activate_document(self.documents.len() - 1);
                    self.remember_recent(path);
                }
                Err(error) => self.show_error("打开失败", &error),
            }
        }
    }

    fn remove_initial_placeholder(&mut self) {
        if self.documents.len() == 1 {
            let document = &self.documents[0];
            if document.path.is_none() && !document.dirty && document.content.is_empty() {
                self.documents.clear();
                self.active = None;
            }
        }
    }

    fn remember_recent(&mut self, path: PathBuf) {
        self.state.recent_files.retain(|existing| existing != &path);
        self.state.recent_files.insert(0, path);
        self.state.recent_files.truncate(12);
    }

    fn save_active(&mut self, force_dialog: bool) {
        let Some(index) = self.active else {
            return;
        };
        let previous_path = self.documents[index].path.clone();

        let needs_path = self.documents[index].path.is_none() || force_dialog;
        let selected_path = needs_path
            .then(|| {
                let title = self.documents[index].title();
                FileDialog::new()
                    .add_filter("Markdown", &["md", "markdown"])
                    .set_file_name(title)
                    .save_file()
            })
            .flatten();

        if needs_path && selected_path.is_none() {
            return;
        }

        let overwrite_external = if !needs_path {
            match self.documents[index].has_external_changes() {
                Ok(true) => {
                    let path = self.documents[index]
                        .path
                        .as_deref()
                        .map(Path::display)
                        .map(|display| display.to_string())
                        .unwrap_or_default();
                    MessageDialog::new()
                        .set_level(MessageLevel::Warning)
                        .set_title("检测到外部修改")
                        .set_description(format!(
                            "{path}\n\n文件已被其他程序修改。确定用 RUPORA 中的内容覆盖吗？"
                        ))
                        .set_buttons(MessageButtons::YesNo)
                        .show()
                        == MessageDialogResult::Yes
                }
                Ok(false) => false,
                Err(error) => {
                    self.show_error("保存前检查失败", &error);
                    return;
                }
            }
        } else {
            true
        };

        if !needs_path
            && self.documents[index]
                .has_external_changes()
                .unwrap_or(false)
            && !overwrite_external
        {
            return;
        }

        let result = if let Some(path) = selected_path {
            self.documents[index].save_as(path, true)
        } else {
            self.documents[index].save(overwrite_external)
        };

        match result {
            Ok(()) => {
                let document = &self.documents[index];
                self.status = format!(
                    "已保存：{} · {} · {}",
                    document
                        .path
                        .as_deref()
                        .map(Path::display)
                        .map(|display| display.to_string())
                        .unwrap_or_else(|| document.title()),
                    document.encoding.label(),
                    document.line_ending.label()
                );
                if let Some(path) = document.path.clone() {
                    self.external_conflicts.remove(&path);
                    self.remember_recent(path);
                }
                if let Some(path) = previous_path {
                    self.external_conflicts.remove(&path);
                }
            }
            Err(error) => self.show_error("保存失败", &error),
        }
    }

    fn export_html(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let document = &self.documents[index];
        let default_name = document
            .title()
            .trim_end_matches(".markdown")
            .trim_end_matches(".md")
            .to_owned()
            + ".html";
        let Some(path) = FileDialog::new()
            .add_filter("HTML", &["html", "htm"])
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };

        let output =
            markdown::render_html_document(&document.content, &document.title(), self.state.dark);
        let images = match export::load_local_images(&document.content, document.path.as_deref()) {
            Ok(images) => images,
            Err(error) => {
                self.show_error("导出失败", &error);
                return;
            }
        };
        let output = match export::embed_local_images(&output, &images) {
            Ok(output) => output,
            Err(error) => {
                self.show_error("导出失败", &error);
                return;
            }
        };
        match export::write_html(&path, &output) {
            Ok(()) => self.status = format!("已导出 HTML：{}", path.display()),
            Err(error) => self.show_error("导出失败", &error),
        }
    }

    fn export_pdf(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let document = &self.documents[index];
        let default_name = document
            .title()
            .trim_end_matches(".markdown")
            .trim_end_matches(".md")
            .to_owned()
            + ".pdf";
        let Some(path) = FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        let html =
            markdown::render_html_document(&document.content, &document.title(), self.state.dark);
        let images = match export::load_local_images(&document.content, document.path.as_deref()) {
            Ok(images) => images,
            Err(error) => {
                self.show_error("PDF 导出失败", &error);
                return;
            }
        };
        match export::write_pdf(&path, &html, &images) {
            Ok(()) => self.status = format!("已导出 PDF：{}", path.display()),
            Err(error) => self.show_error("PDF 导出失败", &error),
        }
    }

    fn print_active(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let document = &self.documents[index];
        let html =
            markdown::render_html_document(&document.content, &document.title(), self.state.dark);
        let images = match export::load_local_images(&document.content, document.path.as_deref()) {
            Ok(images) => images,
            Err(error) => {
                self.show_error("打印失败", &error);
                return;
            }
        };
        match export::print_html(&html, &images) {
            Ok(path) => self.status = format!("已提交系统打印任务：{}", path.display()),
            Err(error) => self.show_error("打印失败", &error),
        }
    }

    fn insert_text(&mut self, text: &str, kind: EditKind) {
        let Some(index) = self.active else {
            return;
        };
        let selection = self.active_selection(index);
        let before = self.documents[index].content.clone();
        let start = char_to_byte(&before, selection.start);
        let end = char_to_byte(&before, selection.end);
        self.documents[index]
            .content
            .replace_range(start..end, text);
        let cursor = selection.start + text.chars().count();
        self.documents[index].record_edit(before, Some(selection), Some(cursor..cursor), kind);
        self.queue_editor_selection(cursor..cursor);
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
    }

    fn insert_footnote(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let number = next_footnote_number(&self.documents[index].content);
        let selection = self.active_selection(index);
        let before = self.documents[index].content.clone();
        let start = char_to_byte(&before, selection.start);
        let end = char_to_byte(&before, selection.end);
        let reference = format!("[^{number}]");
        self.documents[index]
            .content
            .replace_range(start..end, &reference);
        if !self.documents[index].content.ends_with('\n') {
            self.documents[index].content.push('\n');
        }
        self.documents[index]
            .content
            .push_str(&format!("\n[^{number}]: 脚注内容\n"));
        let cursor = selection.start + reference.chars().count();
        self.documents[index].record_edit(
            before,
            Some(selection),
            Some(cursor..cursor),
            EditKind::Format,
        );
        self.queue_editor_selection(cursor..cursor);
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
        self.status = format!("已插入脚注 {number}");
    }

    fn insert_cross_reference(&mut self, label: &str, id: &str) {
        self.insert_text(&format!("[{label}](#{id})"), EditKind::Format);
        self.status = format!("已插入对“{label}”的交叉引用");
    }

    fn open_table_editor(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let cursor = self.active_selection(index).start;
        let cursor_byte = char_to_byte(&self.documents[index].content, cursor);
        let table = table::find_table(&self.documents[index].content, cursor_byte)
            .unwrap_or_else(|| table::new_table(cursor_byte));
        self.table_editor = Some(TableEditorState {
            document: index,
            table,
        });
    }

    fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        if self.documents[index].dirty {
            match prompt_to_save(&self.documents[index].title()) {
                MessageDialogResult::Yes => {
                    self.activate_document(index);
                    self.save_active(false);
                    if self.documents[index].dirty {
                        return;
                    }
                }
                MessageDialogResult::No => {}
                _ => return,
            }
        }

        if let Some(path) = self.documents[index].path.as_ref() {
            self.external_conflicts.remove(path);
        }
        self.documents.remove(index);
        self.active = match (self.active, self.documents.is_empty()) {
            (_, true) => None,
            (Some(active), false) if active > index => Some(active - 1),
            (Some(active), false) if active == index => Some(index.min(self.documents.len() - 1)),
            (active, false) => active,
        };
        self.editor_cursor = None;
        self.pending_editor_cursor = None;
        self.hybrid_active = None;
        self.hybrid_ime_session = None;
        self.restore_active_view_state();
        if self.documents.is_empty() {
            self.next_untitled_id = 1;
            self.new_document();
        }
    }

    fn show_error(&mut self, title: &str, message: &str) {
        self.status = message.to_owned();
        diagnostics::append_event("ERROR", &format!("{title}: {message}")).ok();
        MessageDialog::new()
            .set_level(MessageLevel::Error)
            .set_title(title)
            .set_description(message)
            .set_buttons(MessageButtons::Ok)
            .show();
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        let bindings = self.state.key_bindings.clone();
        let action = ctx.input_mut(|input| {
            if consume_shortcut(input, &bindings.redo) || consume_shortcut(input, "Ctrl+Y") {
                Some(ShortcutAction::Command(AppCommand::Redo))
            } else if consume_shortcut(input, &bindings.undo) {
                Some(ShortcutAction::Command(AppCommand::Undo))
            } else if consume_shortcut(input, &bindings.save_as) {
                Some(ShortcutAction::Command(AppCommand::SaveAs))
            } else if consume_shortcut(input, &bindings.save) {
                Some(ShortcutAction::Command(AppCommand::Save))
            } else if consume_shortcut(input, &bindings.open_folder) {
                Some(ShortcutAction::Command(AppCommand::OpenFolder))
            } else if consume_shortcut(input, &bindings.open_file) {
                Some(ShortcutAction::Command(AppCommand::Open))
            } else if consume_shortcut(input, &bindings.new_document) {
                Some(ShortcutAction::Command(AppCommand::New))
            } else if consume_shortcut(input, &bindings.command_palette) {
                Some(ShortcutAction::Palette)
            } else if consume_shortcut(input, &bindings.bold) {
                Some(ShortcutAction::Command(AppCommand::Format(
                    MarkdownCommand::Bold,
                )))
            } else if consume_shortcut(input, &bindings.italic) {
                Some(ShortcutAction::Command(AppCommand::Format(
                    MarkdownCommand::Italic,
                )))
            } else if consume_shortcut(input, &bindings.link) {
                Some(ShortcutAction::Command(AppCommand::Format(
                    MarkdownCommand::Link,
                )))
            } else if consume_shortcut(input, &bindings.replace) {
                Some(ShortcutAction::Replace)
            } else if consume_shortcut(input, &bindings.find) {
                Some(ShortcutAction::Find)
            } else {
                None
            }
        });

        match action {
            Some(ShortcutAction::Command(command)) => self.execute(command),
            Some(ShortcutAction::Find | ShortcutAction::Replace) => {
                self.find_open = true;
                self.find_focus_requested = true;
            }
            Some(ShortcutAction::Palette) => {
                self.command_palette_open = true;
                self.command_focus_requested = true;
            }
            None => {}
        }
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            if self.command_palette_open {
                self.command_palette_open = false;
            } else if self.find_open {
                self.find_open = false;
            }
        }
        if ctx.input(|input| input.key_pressed(Key::F3)) {
            self.find_match(!ctx.input(|input| input.modifiers.shift));
        }
    }

    fn execute(&mut self, command: AppCommand) {
        match command {
            AppCommand::New => self.new_document(),
            AppCommand::Open => self.open_dialog(),
            AppCommand::OpenFolder => self.open_folder_dialog(),
            AppCommand::ShortcutSettings => self.shortcut_settings_open = true,
            AppCommand::Save => self.save_active(false),
            AppCommand::SaveAs => self.save_active(true),
            AppCommand::Undo => self.undo_active(),
            AppCommand::Redo => self.redo_active(),
            AppCommand::ExportHtml => self.export_html(),
            AppCommand::ExportPdf => self.export_pdf(),
            AppCommand::Print => self.print_active(),
            AppCommand::EditTable => self.open_table_editor(),
            AppCommand::InsertToc => self.insert_text("[TOC]\n", EditKind::Format),
            AppCommand::InsertFootnote => self.insert_footnote(),
            AppCommand::PasteImage => self.paste_clipboard_image(),
            AppCommand::CheckUpdates => self.start_update_check(),
            AppCommand::OpenDiagnostics => self.open_diagnostics(),
            AppCommand::OpenExtensionConfig => self.open_extension_config(),
            AppCommand::ReloadExtensions => self.reload_extensions(),
            AppCommand::RunExtension(index) => self.start_extension(index),
            AppCommand::OpenReleasePage => self.open_release_page(),
            AppCommand::About => self.about_open = true,
            AppCommand::Format(command) => self.apply_format(command),
            AppCommand::SetView(mode) => {
                self.state.view_mode = mode;
                if mode != ViewMode::Hybrid {
                    self.hybrid_ime_session = None;
                }
            }
        }
    }

    fn start_update_check(&mut self) {
        if self.update_receiver.is_some() {
            self.status = "正在检查更新…".to_owned();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = updater::check_for_update(env!("CARGO_PKG_VERSION"));
            let _ = sender.send(result);
        });
        self.update_receiver = Some(receiver);
        self.status = "正在后台检查更新…".to_owned();
    }

    fn poll_update_check(&mut self) {
        let result = match self.update_receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Empty)) | None => None,
            Some(Err(TryRecvError::Disconnected)) => Some(Err("更新检查线程意外结束".to_owned())),
        };
        let Some(result) = result else {
            return;
        };
        self.update_receiver = None;
        match result {
            Ok(UpdateStatus::Current { latest }) => {
                self.available_update = None;
                self.status = format!("当前已是最新版本（{latest}）");
            }
            Ok(UpdateStatus::Available(info)) => {
                self.status = format!("发现新版本 {}，可从“帮助”菜单打开发布页", info.version);
                self.available_update = Some(info);
            }
            Err(error) => {
                diagnostics::append_event("WARN", &format!("update check failed: {error}")).ok();
                self.status = format!("检查更新失败：{error}");
            }
        }
    }

    fn open_release_page(&mut self) {
        let url = self
            .available_update
            .as_ref()
            .map(|update| update.page_url.as_str())
            .unwrap_or(updater::RELEASES_URL);
        if let Err(error) = open::that(url) {
            self.show_error("无法打开发布页", &error.to_string());
        }
    }

    fn open_diagnostics(&mut self) {
        let Some(directory) = diagnostics::log_directory() else {
            self.status = "当前平台没有可用的诊断目录".to_owned();
            return;
        };
        if let Err(error) = fs::create_dir_all(&directory)
            .and_then(|()| open::that(&directory).map_err(std::io::Error::other))
        {
            self.show_error("无法打开诊断目录", &error.to_string());
        } else {
            self.status = format!("已打开诊断目录：{}", directory.display());
        }
    }

    fn open_extension_config(&mut self) {
        if let Err(error) = self.extension_registry.ensure_template() {
            self.show_error("无法创建扩展配置", &error);
            return;
        }
        if let Err(error) = open::that(self.extension_registry.config_path()) {
            self.show_error("无法打开扩展配置", &error.to_string());
        }
    }

    fn reload_extensions(&mut self) {
        match self.extension_registry.reload() {
            Ok(()) => {
                self.status = if self.extension_registry.is_enabled() {
                    format!(
                        "已加载 {} 个扩展服务",
                        self.extension_registry.services().len()
                    )
                } else {
                    "扩展服务保持关闭".to_owned()
                };
            }
            Err(error) => self.show_error("无法加载扩展配置", &error),
        }
    }

    fn start_extension(&mut self, service_index: usize) {
        if self.extension_receiver.is_some() {
            self.status = "已有扩展服务正在运行".to_owned();
            return;
        }
        let Some(document) = self.active else {
            self.status = "没有可交给扩展的活动文档".to_owned();
            return;
        };
        let Some(service) = self
            .extension_registry
            .services()
            .get(service_index)
            .cloned()
        else {
            self.status = "扩展服务不存在或扩展功能已关闭".to_owned();
            return;
        };
        let before = self.documents[document].content.clone();
        let path = self.documents[document].path.clone();
        let document_id = self.documents[document].id();
        let name = service.name.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = extensions::invoke(&service, &before, path.as_deref()).map(|invocation| {
                ExtensionJobResult {
                    document_id,
                    before,
                    invocation,
                }
            });
            let _ = sender.send(result);
        });
        self.extension_receiver = Some(receiver);
        self.status = format!("正在运行扩展“{name}”…");
    }

    fn poll_extension(&mut self) {
        let result = match self.extension_receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Empty)) | None => None,
            Some(Err(TryRecvError::Disconnected)) => Some(Err("扩展工作线程意外结束".to_owned())),
        };
        let Some(result) = result else {
            return;
        };
        self.extension_receiver = None;
        match result {
            Err(error) => {
                diagnostics::append_event("WARN", &format!("extension failed: {error}")).ok();
                self.status = format!("扩展失败：{error}");
            }
            Ok(job) => {
                let Some(document) = extension_document_index(&self.documents, job.document_id)
                else {
                    self.status = "扩展运行期间目标文档已关闭，已丢弃过期结果".to_owned();
                    return;
                };
                if self.documents[document].content != job.before {
                    self.status = "扩展运行期间目标文档已变化，已丢弃过期结果".to_owned();
                    return;
                }
                if let Some(replacement) = job.invocation.replacement
                    && replacement != job.before
                {
                    self.documents[document].content = replacement;
                    self.documents[document].record_edit(job.before, None, None, EditKind::Other);
                    self.active = Some(document);
                    self.status = job
                        .invocation
                        .message
                        .unwrap_or_else(|| "扩展已更新活动文档".to_owned());
                } else {
                    self.status = job
                        .invocation
                        .message
                        .unwrap_or_else(|| "扩展已完成，没有文档修改".to_owned());
                }
            }
        }
    }

    fn poll_instance_requests(&mut self, ctx: &Context) {
        let result = self
            .instance_coordinator
            .as_mut()
            .map(InstanceCoordinator::poll);
        match result {
            Some(Ok(Some(request))) => {
                if !request.paths.is_empty() {
                    self.open_paths(request.paths);
                }
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            Some(Err(error)) => {
                diagnostics::append_event("WARN", &format!("instance inbox failed: {error}")).ok();
                self.status = format!("读取第二实例请求失败：{error}");
            }
            Some(Ok(None)) | None => {}
        }
    }

    fn apply_format(&mut self, command: MarkdownCommand) {
        let Some(index) = self.active else {
            return;
        };
        let selection = self.active_selection(index);
        let before = self.documents[index].content.clone();
        let next_selection = editing::apply_markdown_command(
            &mut self.documents[index].content,
            selection.clone(),
            command,
        );
        self.documents[index].record_edit(
            before,
            Some(selection),
            Some(next_selection.clone()),
            EditKind::Format,
        );
        self.queue_editor_selection(next_selection);
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
        self.status = "已应用 Markdown 格式".to_owned();
    }

    fn undo_active(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let Some(outcome) = self.documents[index].undo() else {
            self.status = "没有可撤销的操作".to_owned();
            return;
        };
        if let Some(selection) = outcome.selection {
            self.queue_editor_selection(selection);
        } else {
            self.editor_cursor = None;
            self.pending_editor_cursor = None;
        }
        self.status = "已撤销".to_owned();
    }

    fn redo_active(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let Some(outcome) = self.documents[index].redo() else {
            self.status = "没有可重做的操作".to_owned();
            return;
        };
        if let Some(selection) = outcome.selection {
            self.queue_editor_selection(selection);
        } else {
            self.editor_cursor = None;
            self.pending_editor_cursor = None;
        }
        self.status = "已重做".to_owned();
    }

    fn active_selection(&self, index: usize) -> std::ops::Range<usize> {
        self.editor_cursor
            .map(|range| {
                let [start, end] = range.sorted_cursors();
                start.index.0..end.index.0
            })
            .unwrap_or_else(|| {
                let end = self.documents[index].content.chars().count();
                end..end
            })
    }

    fn queue_editor_selection(&mut self, range: std::ops::Range<usize>) {
        let cursor_range = CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end));
        self.editor_cursor = Some(cursor_range);
        self.pending_editor_cursor = Some(cursor_range);
    }

    fn jump_to_line(&mut self, one_based_line: usize) {
        let Some(index) = self.active else {
            return;
        };
        let char_index =
            editing::char_index_for_line(&self.documents[index].content, one_based_line);
        self.queue_editor_selection(char_index..char_index);
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
        self.status = format!("已跳转到第 {one_based_line} 行");
    }

    fn find_match(&mut self, forward: bool) {
        let Some(index) = self.active else {
            return;
        };
        if self.find_query.is_empty() {
            self.status = "请输入查找内容".to_owned();
            return;
        }
        let selection = self.active_selection(index);
        let found = if forward {
            editing::find_next(
                &self.documents[index].content,
                &self.find_query,
                selection.end,
                self.find_match_case,
            )
        } else {
            editing::find_previous(
                &self.documents[index].content,
                &self.find_query,
                selection.start,
                self.find_match_case,
            )
        };

        if let Some(range) = found {
            self.queue_editor_selection(range);
            if self.state.view_mode == ViewMode::Preview {
                self.state.view_mode = ViewMode::Edit;
            }
            self.status = "已找到匹配项".to_owned();
        } else {
            self.status = format!("未找到“{}”", self.find_query);
        }
    }

    fn replace_current(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let selection = self.active_selection(index);
        if editing::selection_matches(
            &self.documents[index].content,
            selection.clone(),
            &self.find_query,
            self.find_match_case,
        ) {
            let before = self.documents[index].content.clone();
            let cursor = editing::replace_range(
                &mut self.documents[index].content,
                selection.clone(),
                &self.replace_query,
            );
            self.documents[index].record_edit(
                before,
                Some(selection),
                Some(cursor.clone()),
                EditKind::Replace,
            );
            self.queue_editor_selection(cursor);
            self.status = "已替换 1 处".to_owned();
        }
        self.find_match(true);
    }

    fn replace_all_matches(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let before = self.documents[index].content.clone();
        let selection_before = self.editor_cursor.map(cursor_range_to_char_range);
        let count = editing::replace_all(
            &mut self.documents[index].content,
            &self.find_query,
            &self.replace_query,
            self.find_match_case,
        );
        if count > 0 {
            self.documents[index].record_edit(before, selection_before, None, EditKind::Replace);
            self.editor_cursor = None;
            self.pending_editor_cursor = None;
        }
        self.status = format!("已替换 {count} 处");
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !paths.is_empty() {
            let mut documents = Vec::new();
            let mut resources = Vec::new();
            for path in paths {
                if path.is_dir() {
                    self.open_workspace(path);
                } else if is_markdown_path(&path) {
                    documents.push(path);
                } else {
                    resources.push(path);
                }
            }
            self.open_paths(documents);
            for path in resources {
                self.insert_resource(path);
            }
        }
    }

    fn insert_resource(&mut self, path: PathBuf) {
        let Some(index) = self.active else {
            return;
        };
        if !path.is_file() {
            self.status = format!("资源不存在：{}", path.display());
            return;
        }

        let base = self.documents[index]
            .path
            .as_deref()
            .and_then(Path::parent)
            .or_else(|| {
                self.workspace
                    .as_ref()
                    .map(|workspace| workspace.root.as_path())
            })
            .unwrap_or_else(|| Path::new("."));
        let destination = markdown_resource_destination(&path, base);
        let label = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "resource".to_owned());
        let image = is_image_path(&path);
        let selection = self.active_selection(index);
        let before = self.documents[index].content.clone();
        let next = editing::insert_resource_link(
            &mut self.documents[index].content,
            selection.clone(),
            &label,
            &destination,
            image,
        );
        self.documents[index].record_edit(
            before,
            Some(selection),
            Some(next.clone()),
            EditKind::Other,
        );
        self.queue_editor_selection(next);
        self.status = if image {
            format!("已插入图片：{}", path.display())
        } else {
            format!("已插入附件链接：{}", path.display())
        };
    }

    fn paste_clipboard_image(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        let image = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image())
        {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("剪贴板中没有可用图片：{error}");
                return;
            }
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let file_name = format!("image-{timestamp}.png");
        let path = if let Some(base) = self.documents[index]
            .path
            .as_deref()
            .and_then(Path::parent)
            .or_else(|| {
                self.workspace
                    .as_ref()
                    .map(|workspace| workspace.root.as_path())
            }) {
            let assets = base.join("assets");
            if let Err(error) = fs::create_dir_all(&assets) {
                self.show_error(
                    "创建资源目录失败",
                    &format!("无法创建 {}：{error}", assets.display()),
                );
                return;
            }
            assets.join(file_name)
        } else {
            let Some(path) = FileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name(file_name)
                .save_file()
            else {
                return;
            };
            path
        };

        let result = image::save_buffer_with_format(
            &path,
            image.bytes.as_ref(),
            image.width as u32,
            image.height as u32,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        );
        match result {
            Ok(()) => self.insert_resource(path),
            Err(error) => self.show_error(
                "粘贴图片失败",
                &format!("无法写入 {}：{error}", path.display()),
            ),
        }
    }

    fn save_recovery_snapshot_if_due(&mut self) {
        if self.last_recovery_write.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_recovery_write = Instant::now();
        match self.recovery_store.save(&self.documents) {
            Ok(()) => self.recovery_error_reported = false,
            Err(error) if !self.recovery_error_reported => {
                self.status = error;
                self.recovery_error_reported = true;
            }
            Err(_) => {}
        }
    }

    fn check_external_changes_if_due(&mut self) {
        if self.last_external_check.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_external_check = Instant::now();

        let open_paths = self
            .documents
            .iter()
            .filter_map(|document| document.path.clone())
            .collect::<HashSet<_>>();
        self.external_conflicts
            .retain(|path| open_paths.contains(path));

        let mut reloaded = Vec::new();
        for (index, document) in self.documents.iter_mut().enumerate() {
            let Some(path) = document.path.clone() else {
                continue;
            };
            match document.external_change_hint() {
                Ok(false) => {
                    self.external_conflicts.remove(&path);
                    self.external_scan_error_reported = false;
                }
                Ok(true) if document.dirty => {
                    self.external_conflicts.insert(path);
                }
                Ok(true) => match document.reload() {
                    Ok(()) => {
                        self.external_conflicts.remove(&path);
                        reloaded.push((index, path));
                        self.external_scan_error_reported = false;
                    }
                    Err(_) => {
                        self.external_conflicts.insert(path);
                    }
                },
                Err(error) if !self.external_scan_error_reported => {
                    self.status = error;
                    self.external_scan_error_reported = true;
                }
                Err(_) => {}
            }
        }

        if let Some((index, path)) = reloaded.last() {
            if self.active == Some(*index) {
                self.editor_cursor = None;
                self.pending_editor_cursor = None;
                self.hybrid_active = None;
                self.hybrid_ime_session = None;
            }
            self.status = format!("已自动重新加载外部修改：{}", path.display());
        }
    }

    fn external_change_bar(&mut self, root: &mut Ui) {
        let Some(index) = self.active else {
            return;
        };
        let Some(path) = self.documents[index].path.clone() else {
            return;
        };
        if !self.external_conflicts.contains(&path) {
            return;
        }

        let mut reload = false;
        let mut save_as = false;
        let mut compare = false;
        let mut merge = false;
        let mut relink = false;
        let missing = !path.exists();
        Panel::top("external-change")
            .exact_size(40.0)
            .show(root, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        if missing {
                            "磁盘文件已被移动或删除。"
                        } else {
                            "磁盘文件已发生变化，当前编辑内容尚未覆盖。"
                        },
                    );
                    if missing && ui.button("重新定位…").clicked() {
                        relink = true;
                    }
                    if ui.button("从磁盘重新加载").clicked() {
                        reload = true;
                    }
                    if ui.button("比较").clicked() {
                        compare = true;
                    }
                    if ui.button("三方合并").clicked() {
                        merge = true;
                    }
                    if ui.button("另存为…").clicked() {
                        save_as = true;
                    }
                });
            });

        if relink {
            if let Some(new_path) = FileDialog::new()
                .add_filter("Markdown", &["md", "markdown", "mdown", "mkd", "txt"])
                .pick_file()
            {
                match self.documents[index].relink_external(new_path.clone()) {
                    Ok(conflicts) => {
                        self.external_conflicts.remove(&path);
                        self.remember_recent(new_path);
                        self.editor_cursor = None;
                        self.pending_editor_cursor = None;
                        self.hybrid_active = None;
                        self.hybrid_ime_session = None;
                        self.status = if conflicts == 0 {
                            "已重新关联移动后的文件".to_owned()
                        } else {
                            format!("已重新关联，但存在 {conflicts} 处合并冲突")
                        };
                    }
                    Err(error) => self.show_error("重新关联失败", &error),
                }
            }
        } else if compare {
            match self.documents[index].external_diff() {
                Ok(diff) => self.external_diff_view = Some(diff),
                Err(error) => self.show_error("比较失败", &error),
            }
        } else if merge {
            match self.documents[index].merge_external() {
                Ok(conflicts) => {
                    self.external_conflicts.remove(&path);
                    self.editor_cursor = None;
                    self.pending_editor_cursor = None;
                    self.hybrid_active = None;
                    self.hybrid_ime_session = None;
                    self.status = if conflicts == 0 {
                        "已自动合并外部修改".to_owned()
                    } else {
                        format!("合并完成，存在 {conflicts} 处冲突；请搜索 <<<<<<< 并人工处理")
                    };
                }
                Err(error) => self.show_error("合并失败", &error),
            }
        } else if reload {
            let confirmed = !self.documents[index].dirty
                || MessageDialog::new()
                    .set_level(MessageLevel::Warning)
                    .set_title("重新加载外部版本")
                    .set_description("这会丢弃 RUPORA 中尚未保存的修改。确定继续吗？")
                    .set_buttons(MessageButtons::YesNo)
                    .show()
                    == MessageDialogResult::Yes;
            if confirmed {
                match self.documents[index].reload() {
                    Ok(()) => {
                        self.external_conflicts.remove(&path);
                        self.editor_cursor = None;
                        self.pending_editor_cursor = None;
                        self.hybrid_active = None;
                        self.hybrid_ime_session = None;
                        self.status = format!("已从磁盘重新加载：{}", path.display());
                    }
                    Err(error) => self.show_error("重新加载失败", &error),
                }
            }
        } else if save_as {
            self.save_active(true);
        }
    }

    fn external_diff_window(&mut self, root: &mut Ui) {
        let Some(diff) = self.external_diff_view.as_mut() else {
            return;
        };
        let mut open = true;
        egui::Window::new("当前编辑版本 ↔ 磁盘版本")
            .id(egui::Id::new("external-diff"))
            .default_size([760.0, 560.0])
            .open(&mut open)
            .show(root.ctx(), |ui| {
                ui.label("“-”表示当前编辑器内容，“+”表示磁盘内容。");
                ui.add(
                    TextEdit::multiline(diff)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .interactive(false),
                );
            });
        if !open {
            self.external_diff_view = None;
        }
    }

    fn table_editor_window(&mut self, root: &mut Ui) {
        let Some(state) = self.table_editor.as_mut() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("可视化表格编辑器")
            .id(egui::Id::new("table-editor"))
            .default_size([760.0, 420.0])
            .open(&mut open)
            .show(root.ctx(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("增加列").clicked() {
                        state.table.add_column();
                    }
                    if ui
                        .add_enabled(state.table.headers.len() > 1, Button::new("删除末列"))
                        .clicked()
                    {
                        state.table.remove_column();
                    }
                    if ui.button("增加行").clicked() {
                        state.table.add_row();
                    }
                    if ui
                        .add_enabled(!state.table.rows.is_empty(), Button::new("删除末行"))
                        .clicked()
                    {
                        state.table.remove_row();
                    }
                    ui.separator();
                    ui.label(format!(
                        "{} 列 × {} 行",
                        state.table.headers.len(),
                        state.table.rows.len()
                    ));
                });
                ui.separator();
                ScrollArea::both().show(ui, |ui| {
                    egui::Grid::new("table-editor-grid")
                        .striped(true)
                        .spacing([8.0, 7.0])
                        .show(ui, |ui| {
                            for column in 0..state.table.headers.len() {
                                ui.vertical(|ui| {
                                    ui.add_sized(
                                        [150.0, 24.0],
                                        TextEdit::singleline(&mut state.table.headers[column]),
                                    );
                                    egui::ComboBox::from_id_salt(("table-alignment", column))
                                        .selected_text(match state.table.alignments[column] {
                                            table::Alignment::None => "默认对齐",
                                            table::Alignment::Left => "左对齐",
                                            table::Alignment::Center => "居中",
                                            table::Alignment::Right => "右对齐",
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut state.table.alignments[column],
                                                table::Alignment::None,
                                                "默认对齐",
                                            );
                                            ui.selectable_value(
                                                &mut state.table.alignments[column],
                                                table::Alignment::Left,
                                                "左对齐",
                                            );
                                            ui.selectable_value(
                                                &mut state.table.alignments[column],
                                                table::Alignment::Center,
                                                "居中",
                                            );
                                            ui.selectable_value(
                                                &mut state.table.alignments[column],
                                                table::Alignment::Right,
                                                "右对齐",
                                            );
                                        });
                                });
                            }
                            ui.end_row();
                            for row in &mut state.table.rows {
                                for cell in row {
                                    ui.add_sized([150.0, 24.0], TextEdit::singleline(cell));
                                }
                                ui.end_row();
                            }
                        });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("应用到 Markdown").clicked() {
                        apply = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });

        if apply {
            let state = self.table_editor.take().expect("table editor state");
            if state.document >= self.documents.len() {
                return;
            }
            let document = &mut self.documents[state.document];
            let before = document.content.clone();
            if state.table.range.end > before.len()
                || !before.is_char_boundary(state.table.range.start)
                || !before.is_char_boundary(state.table.range.end)
            {
                self.show_error("表格应用失败", "文档已发生变化，请重新打开表格编辑器。");
                return;
            }
            let mut replacement = state.table.to_markdown();
            if state.table.range.is_empty() {
                if state.table.range.start > 0 && !before[..state.table.range.start].ends_with('\n')
                {
                    replacement.insert_str(0, "\n\n");
                }
                if state.table.range.start < before.len()
                    && !before[state.table.range.start..].starts_with('\n')
                {
                    replacement.push_str("\n\n");
                }
            }
            let cursor =
                before[..state.table.range.start].chars().count() + replacement.chars().count();
            document
                .content
                .replace_range(state.table.range, &replacement);
            document.record_edit(before, None, Some(cursor..cursor), EditKind::Format);
            self.queue_editor_selection(cursor..cursor);
            self.status = "已应用可视化表格修改".to_owned();
        } else if !open || cancel {
            self.table_editor = None;
        }
    }

    fn top_bar(&mut self, root: &mut Ui) {
        let extension_names = self
            .extension_registry
            .services()
            .iter()
            .map(|service| service.name.clone())
            .collect::<Vec<_>>();
        let palette = app_palette(self.state.dark);
        let toolbar_frame = egui::Frame::new()
            .fill(palette.toolbar)
            .inner_margin(Margin::symmetric(10, 6))
            .stroke(Stroke::new(1.0, palette.border));
        Panel::top("toolbar")
            .exact_size(46.0)
            .frame(toolbar_frame)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        Button::new(RichText::new("R").strong().color(palette.accent))
                            .fill(palette.accent_soft)
                            .stroke(Stroke::NONE)
                            .corner_radius(7)
                            .min_size(Vec2::splat(28.0)),
                    )
                    .on_hover_text("RUPORA · 原生 Markdown 编辑器");
                    ui.label(
                        RichText::new("RUPORA")
                            .size(12.5)
                            .strong()
                            .color(palette.text),
                    );
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    if app_icon_button(ui, AppIcon::New, false, "新建文档 · Ctrl+N", palette)
                        .clicked()
                    {
                        self.execute(AppCommand::New);
                    }
                    if app_icon_button(
                        ui,
                        AppIcon::Folder,
                        false,
                        "打开 Markdown · Ctrl+O",
                        palette,
                    )
                    .clicked()
                    {
                        self.execute(AppCommand::Open);
                    }
                    if ui
                        .add_enabled(
                            self.active.is_some(),
                            icon_button_widget(AppIcon::Save, false, palette),
                        )
                        .on_hover_text("保存当前文档 · Ctrl+S")
                        .clicked()
                    {
                        self.execute(AppCommand::Save);
                    }
                    ui.menu_button(RichText::new("···").size(17.0), |ui| {
                        ui.set_min_width(180.0);
                        ui.label(RichText::new("全部操作").small().color(palette.secondary));
                        ui.separator();
                        if ui.button("新建").on_hover_text("Ctrl+N").clicked() {
                            self.execute(AppCommand::New);
                        }
                        if ui.button("打开").on_hover_text("Ctrl+O").clicked() {
                            self.execute(AppCommand::Open);
                        }
                        if ui.button("文件夹").on_hover_text("Ctrl+Shift+O").clicked() {
                            self.execute(AppCommand::OpenFolder);
                        }
                        if ui.button("保存").on_hover_text("Ctrl+S").clicked() {
                            self.execute(AppCommand::Save);
                        }
                        if ui.button("另存为").on_hover_text("Ctrl+Shift+S").clicked() {
                            self.execute(AppCommand::SaveAs);
                        }
                        let can_undo = self
                            .active
                            .and_then(|index| self.documents.get(index))
                            .is_some_and(Document::can_undo);
                        if ui
                            .add_enabled(can_undo, Button::new("撤销"))
                            .on_hover_text("Ctrl+Z")
                            .clicked()
                        {
                            self.execute(AppCommand::Undo);
                        }
                        let can_redo = self
                            .active
                            .and_then(|index| self.documents.get(index))
                            .is_some_and(Document::can_redo);
                        if ui
                            .add_enabled(can_redo, Button::new("重做"))
                            .on_hover_text("Ctrl+Shift+Z / Ctrl+Y")
                            .clicked()
                        {
                            self.execute(AppCommand::Redo);
                        }
                        ui.menu_button("导出", |ui| {
                            if ui.button("HTML…").clicked() {
                                self.execute(AppCommand::ExportHtml);
                                ui.close();
                            }
                            if ui.button("PDF…").clicked() {
                                self.execute(AppCommand::ExportPdf);
                                ui.close();
                            }
                            if ui.button("打印…").clicked() {
                                self.execute(AppCommand::Print);
                                ui.close();
                            }
                        });
                        ui.menu_button("扩展", |ui| {
                            if !self.extension_registry.is_enabled() {
                                ui.label("扩展默认关闭");
                            } else if extension_names.is_empty() {
                                ui.label("没有已配置的扩展服务");
                            }
                            for (index, name) in extension_names.iter().enumerate() {
                                if ui
                                    .add_enabled(
                                        self.extension_receiver.is_none() && self.active.is_some(),
                                        Button::new(name),
                                    )
                                    .clicked()
                                {
                                    self.execute(AppCommand::RunExtension(index));
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("打开扩展配置").clicked() {
                                self.execute(AppCommand::OpenExtensionConfig);
                                ui.close();
                            }
                            if ui.button("重新加载扩展配置").clicked() {
                                self.execute(AppCommand::ReloadExtensions);
                                ui.close();
                            }
                        });
                        ui.menu_button("帮助", |ui| {
                            if ui
                                .add_enabled(
                                    self.update_receiver.is_none(),
                                    Button::new("检查更新…"),
                                )
                                .clicked()
                            {
                                self.execute(AppCommand::CheckUpdates);
                                ui.close();
                            }
                            if self.available_update.is_some()
                                && ui.button("打开新版本发布页").clicked()
                            {
                                self.execute(AppCommand::OpenReleasePage);
                                ui.close();
                            }
                            if ui.button("所有版本与校验信息").clicked() {
                                self.execute(AppCommand::OpenReleasePage);
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("打开诊断日志目录").clicked() {
                                self.execute(AppCommand::OpenDiagnostics);
                                ui.close();
                            }
                            if ui.button("关于 RUPORA").clicked() {
                                self.execute(AppCommand::About);
                                ui.close();
                            }
                        });

                        ui.separator();
                        ui.menu_button("格式", |ui| {
                            if ui.button("粗体    Ctrl+B").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::Bold));
                                ui.close();
                            }
                            if ui.button("斜体    Ctrl+I").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::Italic));
                                ui.close();
                            }
                            if ui.button("删除线").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::Strikethrough));
                                ui.close();
                            }
                            if ui.button("行内代码").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::InlineCode));
                                ui.close();
                            }
                            if ui.button("链接    Ctrl+K").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::Link));
                                ui.close();
                            }
                            ui.separator();
                            for level in 1..=6 {
                                if ui.button(format!("标题 {level}")).clicked() {
                                    self.execute(AppCommand::Format(MarkdownCommand::Heading(
                                        level,
                                    )));
                                    ui.close();
                                }
                            }
                            if ui.button("引用").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::Quote));
                                ui.close();
                            }
                            if ui.button("无序列表").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::BulletList));
                                ui.close();
                            }
                            if ui.button("有序列表").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::OrderedList));
                                ui.close();
                            }
                            if ui.button("代码块").clicked() {
                                self.execute(AppCommand::Format(MarkdownCommand::CodeBlock));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("目录 [TOC]").clicked() {
                                self.execute(AppCommand::InsertToc);
                                ui.close();
                            }
                            if ui.button("脚注").clicked() {
                                self.execute(AppCommand::InsertFootnote);
                                ui.close();
                            }
                            if ui.button("可视化表格…").clicked() {
                                self.execute(AppCommand::EditTable);
                                ui.close();
                            }
                            let anchors = self
                                .active
                                .map(|index| {
                                    markdown::heading_anchors(&self.documents[index].content)
                                })
                                .unwrap_or_default();
                            ui.menu_button("交叉引用", |ui| {
                                if anchors.is_empty() {
                                    ui.label("当前文档没有标题");
                                }
                                for anchor in &anchors {
                                    let label = format!(
                                        "H{}  {}",
                                        anchor.heading.level, anchor.heading.text
                                    );
                                    if ui.button(label).clicked() {
                                        self.insert_cross_reference(
                                            &anchor.heading.text,
                                            &anchor.id,
                                        );
                                        ui.close();
                                    }
                                }
                            });
                        });
                        if ui.button("查找").on_hover_text("Ctrl+F").clicked() {
                            self.find_open = true;
                            self.find_focus_requested = true;
                        }
                        if ui.button("命令").on_hover_text("Ctrl+Shift+P").clicked() {
                            self.command_palette_open = true;
                            self.command_focus_requested = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let theme_label = if self.state.dark { "浅色" } else { "深色" };
                            if ui.button(theme_label).clicked() {
                                self.state.dark = !self.state.dark;
                                apply_theme(ui.ctx(), self.state.dark);
                            }
                            ui.checkbox(&mut self.state.show_outline, "大纲");
                            ui.checkbox(&mut self.state.show_sidebar, "文档");
                        });
                    });

                    ui.add_space(10.0);
                    let command_width = (ui.available_width() - 12.0).clamp(0.0, 440.0);
                    if command_width >= 170.0
                        && ui
                            .add_sized(
                                [command_width, 29.0],
                                Button::new(
                                    RichText::new("⌕  搜索命令或打开文件…    Ctrl+Shift+P")
                                        .size(11.5)
                                        .color(palette.secondary),
                                )
                                .fill(palette.surface)
                                .stroke(Stroke::new(1.0, palette.border))
                                .corner_radius(7),
                            )
                            .on_hover_text("打开命令面板")
                            .clicked()
                    {
                        self.command_palette_open = true;
                        self.command_focus_requested = true;
                    }
                });
            });
    }

    fn find_bar(&mut self, root: &mut Ui) {
        if !self.find_open {
            return;
        }
        Panel::top("find-and-replace")
            .exact_size(76.0)
            .show(root, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("查找");
                    let response = ui.add_sized(
                        [240.0, 24.0],
                        TextEdit::singleline(&mut self.find_query).hint_text("查找内容"),
                    );
                    if self.find_focus_requested {
                        response.request_focus();
                        self.find_focus_requested = false;
                    }
                    if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        self.find_match(!ui.input(|input| input.modifiers.shift));
                    }
                    if ui.button("上一个").clicked() {
                        self.find_match(false);
                    }
                    if ui.button("下一个").clicked() {
                        self.find_match(true);
                    }
                    ui.checkbox(&mut self.find_match_case, "区分大小写");
                    if ui.button("关闭").clicked() {
                        self.find_open = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("替换");
                    ui.add_sized(
                        [240.0, 24.0],
                        TextEdit::singleline(&mut self.replace_query).hint_text("替换为"),
                    );
                    if ui.button("替换").clicked() {
                        self.replace_current();
                    }
                    if ui.button("全部替换").clicked() {
                        self.replace_all_matches();
                    }
                });
            });
    }

    fn about_window(&mut self, root: &mut Ui) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        let available_update = self.available_update.clone();
        egui::Window::new("关于 RUPORA")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(root.ctx(), |ui| {
                ui.heading("RUPORA");
                ui.label(format!("版本 {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.label("使用 Rust 与 egui 实现的原生 Markdown 编辑器。");
                ui.label("编辑、解析、数学公式、Mermaid 与 PDF 导出均不依赖浏览器内核。");
                ui.add_space(8.0);
                if let Some(update) = available_update.as_ref() {
                    ui.label(
                        RichText::new(format!("可用更新：{}", update.version))
                            .color(ui.visuals().hyperlink_color),
                    );
                    if !update.notes.trim().is_empty() {
                        ui.label(update.notes.lines().next().unwrap_or_default());
                    }
                    ui.label(format!(
                        "已验证 {} 的 {} 个发布产物",
                        update.target,
                        update.artifacts.len()
                    ));
                }
                ui.label(format!(
                    "平台：{} / {}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ));
            });
        self.about_open = open;
    }

    fn command_palette(&mut self, root: &mut Ui) {
        if !self.command_palette_open {
            return;
        }

        let commands = [
            ("新建文档", AppCommand::New),
            ("打开文件", AppCommand::Open),
            ("打开工作区", AppCommand::OpenFolder),
            ("保存", AppCommand::Save),
            ("另存为", AppCommand::SaveAs),
            ("撤销", AppCommand::Undo),
            ("重做", AppCommand::Redo),
            ("导出 HTML", AppCommand::ExportHtml),
            ("导出 PDF", AppCommand::ExportPdf),
            ("打印", AppCommand::Print),
            ("编辑表格", AppCommand::EditTable),
            ("插入目录", AppCommand::InsertToc),
            ("插入脚注", AppCommand::InsertFootnote),
            ("粘贴剪贴板图片", AppCommand::PasteImage),
            ("快捷键设置", AppCommand::ShortcutSettings),
            ("检查更新", AppCommand::CheckUpdates),
            ("打开诊断日志目录", AppCommand::OpenDiagnostics),
            ("关于 RUPORA", AppCommand::About),
            ("切换到源码模式", AppCommand::SetView(ViewMode::Edit)),
            (
                "切换到所见即所得模式",
                AppCommand::SetView(ViewMode::Hybrid),
            ),
            ("格式：粗体", AppCommand::Format(MarkdownCommand::Bold)),
            ("格式：斜体", AppCommand::Format(MarkdownCommand::Italic)),
            ("格式：链接", AppCommand::Format(MarkdownCommand::Link)),
            (
                "格式：代码块",
                AppCommand::Format(MarkdownCommand::CodeBlock),
            ),
        ];

        let mut open = self.command_palette_open;
        let mut selected = None;
        egui::Window::new("命令面板")
            .id(egui::Id::new("command-palette"))
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .default_width(460.0)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(root.ctx(), |ui| {
                let response = ui.add_sized(
                    [ui.available_width(), 28.0],
                    TextEdit::singleline(&mut self.command_query)
                        .hint_text("输入命令，例如：保存、所见即所得、粗体"),
                );
                if self.command_focus_requested {
                    response.request_focus();
                    self.command_focus_requested = false;
                }
                ui.separator();

                let query = self.command_query.trim().to_lowercase();
                let filtered = commands
                    .iter()
                    .filter(|(label, _)| query.is_empty() || label.to_lowercase().contains(&query))
                    .collect::<Vec<_>>();
                if filtered.is_empty() {
                    ui.label(RichText::new("没有匹配命令").weak());
                    return;
                }
                if response.has_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                    selected = Some(filtered[0].1);
                }
                for (label, command) in filtered.into_iter().take(12) {
                    if ui.selectable_label(false, *label).clicked() {
                        selected = Some(*command);
                    }
                }
            });
        self.command_palette_open = open;

        if let Some(command) = selected {
            self.command_palette_open = false;
            self.command_query.clear();
            self.execute(command);
        }
    }

    fn shortcut_settings(&mut self, root: &mut Ui) {
        if !self.shortcut_settings_open {
            return;
        }
        let mut open = self.shortcut_settings_open;
        egui::Window::new("快捷键设置")
            .id(egui::Id::new("shortcut-settings"))
            .default_width(420.0)
            .collapsible(false)
            .open(&mut open)
            .show(root.ctx(), |ui| {
                ui.label("使用 Ctrl、Shift、Alt 与字母组合；Ctrl 在 macOS 上对应 Command。");
                ui.add_space(6.0);
                egui::Grid::new("shortcut-grid")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        shortcut_row(ui, "新建", &mut self.state.key_bindings.new_document);
                        shortcut_row(ui, "打开", &mut self.state.key_bindings.open_file);
                        shortcut_row(ui, "打开工作区", &mut self.state.key_bindings.open_folder);
                        shortcut_row(ui, "保存", &mut self.state.key_bindings.save);
                        shortcut_row(ui, "另存为", &mut self.state.key_bindings.save_as);
                        shortcut_row(ui, "撤销", &mut self.state.key_bindings.undo);
                        shortcut_row(ui, "重做", &mut self.state.key_bindings.redo);
                        shortcut_row(ui, "查找", &mut self.state.key_bindings.find);
                        shortcut_row(ui, "替换", &mut self.state.key_bindings.replace);
                        shortcut_row(ui, "命令面板", &mut self.state.key_bindings.command_palette);
                        shortcut_row(ui, "粗体", &mut self.state.key_bindings.bold);
                        shortcut_row(ui, "斜体", &mut self.state.key_bindings.italic);
                        shortcut_row(ui, "链接", &mut self.state.key_bindings.link);
                    });
                ui.separator();
                if duplicate_shortcuts(&self.state.key_bindings) {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "存在重复快捷键，前面的命令会优先。",
                    );
                }
                if ui.button("恢复默认").clicked() {
                    self.state.key_bindings = KeyBindings::default();
                }
            });
        self.shortcut_settings_open = open;
    }

    fn sidebar(&mut self, root: &mut Ui) {
        let palette = app_palette(self.state.dark);
        Panel::left("activity-rail")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.toolbar)
                    .inner_margin(Margin::symmetric(7, 8))
                    .stroke(Stroke::new(1.0, palette.border)),
            )
            .show(root, |ui| {
                ui.vertical_centered(|ui| {
                    if app_icon_button(
                        ui,
                        AppIcon::Sidebar,
                        self.state.show_sidebar,
                        "资源管理器",
                        palette,
                    )
                    .clicked()
                    {
                        self.state.show_sidebar = !self.state.show_sidebar;
                    }
                    ui.add_space(3.0);
                    if app_icon_button(
                        ui,
                        AppIcon::Outline,
                        self.state.show_outline,
                        "文档大纲",
                        palette,
                    )
                    .clicked()
                    {
                        self.state.show_outline = !self.state.show_outline;
                    }
                });
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if app_icon_button(ui, AppIcon::Theme, false, "切换浅色 / 深色外观", palette)
                        .clicked()
                    {
                        self.state.dark = !self.state.dark;
                        apply_theme(ui.ctx(), self.state.dark);
                    }
                });
            });

        if !self.state.show_sidebar {
            return;
        }

        Panel::left("documents")
            .default_size(224.0)
            .size_range(190.0..=360.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(12, 10))
                    .stroke(Stroke::new(1.0, palette.border)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("资源管理器")
                            .size(11.5)
                            .strong()
                            .color(palette.secondary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if app_icon_button(ui, AppIcon::New, false, "新建文档", palette).clicked()
                        {
                            self.new_document();
                        }
                    });
                });
                ui.separator();
                let mut workspace_file_to_open = None;
                let mut refresh_workspace = false;
                let mut close_workspace = false;
                if let Some(workspace) = self.workspace.as_ref() {
                    ui.horizontal(|ui| {
                        let root_name = workspace
                            .root
                            .file_name()
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_else(|| workspace.root.as_os_str().to_string_lossy());
                        ui.strong(root_name);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("×").on_hover_text("关闭工作区").clicked() {
                                close_workspace = true;
                            }
                            if ui.small_button("↻").on_hover_text("刷新工作区").clicked() {
                                refresh_workspace = true;
                            }
                        });
                    });
                    let active_path = self
                        .active
                        .and_then(|index| self.documents.get(index))
                        .and_then(|document| document.path.as_deref());
                    ScrollArea::vertical()
                        .id_salt("workspace-tree")
                        .max_height(260.0)
                        .show(ui, |ui| {
                            workspace_file_to_open =
                                workspace_entries_ui(ui, &workspace.entries, active_path);
                        });
                    ui.separator();
                }

                if close_workspace {
                    self.workspace = None;
                    self.state.workspace_root = None;
                    self.status = "已关闭工作区".to_owned();
                } else if refresh_workspace && let Some(workspace) = self.workspace.as_mut() {
                    match workspace.refresh() {
                        Ok(()) => self.status = "工作区已刷新".to_owned(),
                        Err(error) => self.status = error,
                    }
                }
                if let Some(path) = workspace_file_to_open {
                    self.open_paths([path]);
                }

                if self.workspace.is_none() {
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(palette.surface)
                        .stroke(Stroke::new(1.0, palette.border))
                        .corner_radius(8)
                        .inner_margin(Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("尚未打开工作区")
                                    .size(12.5)
                                    .strong()
                                    .color(palette.text),
                            );
                            ui.label(
                                RichText::new("打开文件夹，快速浏览和管理 Markdown 文稿。")
                                    .small()
                                    .color(palette.secondary),
                            );
                            ui.add_space(7.0);
                            if ui
                                .add(
                                    Button::new("打开文件夹")
                                        .fill(palette.accent_soft)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(6),
                                )
                                .clicked()
                            {
                                self.execute(AppCommand::OpenFolder);
                            }
                        });
                    ui.add_space(10.0);
                }

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("打开的编辑器")
                            .size(11.5)
                            .strong()
                            .color(palette.secondary),
                    );
                });
                ui.add_space(3.0);

                let mut activate = None;
                let mut close = None;
                ScrollArea::vertical().show(ui, |ui| {
                    for (index, document) in self.documents.iter().enumerate() {
                        let selected = self.active == Some(index);
                        egui::Frame::new()
                            .fill(if selected {
                                palette.accent_soft
                            } else {
                                Color32::TRANSPARENT
                            })
                            .corner_radius(6)
                            .inner_margin(Margin::symmetric(5, 2))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (icon_rect, _) = ui.allocate_exact_size(
                                        Vec2::splat(16.0),
                                        egui::Sense::hover(),
                                    );
                                    paint_app_icon(
                                        ui.painter(),
                                        icon_rect.shrink(2.0),
                                        AppIcon::File,
                                        if selected {
                                            palette.accent
                                        } else {
                                            palette.secondary
                                        },
                                    );
                                    let title = if document.dirty {
                                        format!("{}  •", document.title())
                                    } else {
                                        document.title()
                                    };
                                    let title_width = (ui.available_width() - 26.0).max(40.0);
                                    let response = ui
                                        .add_sized(
                                            [title_width, 25.0],
                                            Button::new(RichText::new(title).color(if selected {
                                                palette.text
                                            } else {
                                                palette.secondary
                                            }))
                                            .frame(false)
                                            .truncate(),
                                        )
                                        .on_hover_text(
                                            document
                                                .path
                                                .as_deref()
                                                .map(Path::display)
                                                .map(|display| display.to_string())
                                                .unwrap_or_else(|| "尚未保存".to_owned()),
                                        );
                                    if response.clicked() {
                                        activate = Some(index);
                                    }
                                    if ui
                                        .add(AppIconButton {
                                            icon: AppIcon::Close,
                                            selected: false,
                                            palette,
                                            size: 22.0,
                                        })
                                        .on_hover_text("关闭")
                                        .clicked()
                                    {
                                        close = Some(index);
                                    }
                                });
                            });
                    }
                });

                if let Some(index) = activate {
                    self.activate_document(index);
                }
                if let Some(index) = close {
                    self.close_document(index);
                }

                if !self.state.recent_files.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("最近文件").weak());
                    let mut open_recent = None;
                    for path in self.state.recent_files.iter().take(6) {
                        if ui
                            .small_button(
                                path.file_name()
                                    .map(|name| name.to_string_lossy())
                                    .unwrap_or_else(|| path.as_os_str().to_string_lossy()),
                            )
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            open_recent = Some(path.clone());
                        }
                    }
                    if let Some(path) = open_recent {
                        self.open_paths([path]);
                    }
                }
            });
    }

    fn outline(&mut self, root: &mut Ui) {
        if !self.state.show_outline {
            return;
        }
        let headings = self
            .active
            .and_then(|index| self.documents.get(index))
            .map(|document| document.analysis.headings.clone())
            .unwrap_or_default();

        let mut jump_to_line = None;
        let palette = app_palette(self.state.dark);
        Panel::right("outline")
            .default_size(220.0)
            .size_range(180.0..=340.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(12, 10))
                    .stroke(Stroke::new(1.0, palette.border)),
            )
            .show(root, |ui| {
                ui.label(
                    RichText::new("大纲")
                        .size(11.5)
                        .strong()
                        .color(palette.secondary),
                );
                ui.add_space(4.0);
                ui.separator();
                if headings.is_empty() {
                    ui.label(RichText::new("暂无标题").weak());
                } else {
                    ScrollArea::vertical().show(ui, |ui| {
                        for heading in headings {
                            if outline_row(ui, &heading) {
                                jump_to_line = Some(heading.line);
                            }
                        }
                    });
                }
            });
        if let Some(line) = jump_to_line {
            self.jump_to_line(line);
        }
    }

    fn editor_tabs(&mut self, root: &mut Ui) {
        if self.documents.is_empty() {
            return;
        }
        let tabs = self
            .documents
            .iter()
            .map(|document| (document.title(), document.dirty))
            .collect::<Vec<_>>();
        let palette = app_palette(self.state.dark);
        let mut activate = None;
        let mut close = None;
        Panel::top("editor-tabs")
            .exact_size(38.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.toolbar)
                    .stroke(Stroke::new(1.0, palette.border)),
            )
            .show(root, |ui| {
                ScrollArea::horizontal()
                    .id_salt("editor-tabs-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for (index, (title, dirty)) in tabs.iter().enumerate() {
                                let selected = self.active == Some(index);
                                let tab = egui::Frame::new()
                                    .fill(if selected {
                                        palette.surface
                                    } else {
                                        palette.toolbar
                                    })
                                    .stroke(Stroke::new(
                                        1.0,
                                        if selected {
                                            palette.surface
                                        } else {
                                            palette.border
                                        },
                                    ))
                                    .inner_margin(Margin::symmetric(9, 3))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            let label = if *dirty {
                                                format!("{title}  •")
                                            } else {
                                                title.clone()
                                            };
                                            if ui
                                                .add(
                                                    Button::new(RichText::new(label).color(
                                                        if selected {
                                                            palette.text
                                                        } else {
                                                            palette.secondary
                                                        },
                                                    ))
                                                    .frame(false)
                                                    .truncate()
                                                    .min_size(Vec2::new(112.0, 26.0)),
                                                )
                                                .clicked()
                                            {
                                                activate = Some(index);
                                            }
                                            if ui
                                                .add(AppIconButton {
                                                    icon: AppIcon::Close,
                                                    selected: false,
                                                    palette,
                                                    size: 21.0,
                                                })
                                                .on_hover_text("关闭编辑器")
                                                .clicked()
                                            {
                                                close = Some(index);
                                            }
                                        });
                                    });
                                if selected {
                                    ui.painter().line_segment(
                                        [
                                            tab.response.rect.left_top(),
                                            tab.response.rect.right_top(),
                                        ],
                                        Stroke::new(2.0, palette.accent),
                                    );
                                }
                            }
                            ui.add_space(5.0);
                            if app_icon_button(
                                ui,
                                AppIcon::New,
                                false,
                                "新建编辑器 · Ctrl+N",
                                palette,
                            )
                            .clicked()
                            {
                                self.new_document();
                            }
                        });
                    });
            });
        if let Some(index) = activate {
            self.activate_document(index);
        }
        if let Some(index) = close {
            self.close_document(index);
        }
    }

    fn editor(&mut self, root: &mut Ui) {
        let palette = app_palette(self.state.dark);
        CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.canvas))
            .show(root, |ui| {
                let Some(index) = self.active else {
                    ui.centered_and_justified(|ui| {
                        egui::Frame::new()
                            .fill(palette.canvas)
                            .stroke(Stroke::NONE)
                            .corner_radius(2)
                            .inner_margin(Margin::symmetric(48, 40))
                            .show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new("R").size(42.0).color(palette.accent));
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("开始书写").size(24.0).strong());
                                    ui.label(
                                        RichText::new("创建新文稿，或继续编辑已有 Markdown 文件")
                                            .color(palette.secondary),
                                    );
                                    ui.add_space(18.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add(
                                                Button::new(
                                                    RichText::new("新建文稿").color(Color32::WHITE),
                                                )
                                                .fill(palette.accent)
                                                .stroke(Stroke::NONE),
                                            )
                                            .clicked()
                                        {
                                            self.execute(AppCommand::New);
                                        }
                                        if ui
                                            .add(
                                                Button::new("打开文件")
                                                    .fill(palette.canvas)
                                                    .stroke(Stroke::new(1.0, palette.border)),
                                            )
                                            .clicked()
                                        {
                                            self.execute(AppCommand::Open);
                                        }
                                    });
                                });
                            });
                    });
                    return;
                };

                let mode = self.state.view_mode;
                match mode {
                    ViewMode::Edit => {
                        self.edit_pane(ui, index, None);
                    }
                    ViewMode::Preview => {
                        self.preview_pane(ui, index, None);
                    }
                    ViewMode::Hybrid => self.hybrid_pane(ui, index),
                    ViewMode::Split => {
                        if self.split_scroll_document != Some(index) {
                            self.split_scroll_document = Some(index);
                            self.split_scroll_ratio = 0.0;
                            self.split_editor_maximum = 0.0;
                            self.split_preview_maximum = 0.0;
                        }
                        let editor_target = (self.split_scroll_driver
                            == SplitScrollDriver::Preview)
                            .then_some(self.split_scroll_ratio * self.split_editor_maximum);
                        let preview_target = (self.split_scroll_driver
                            == SplitScrollDriver::Editor)
                            .then_some(self.split_scroll_ratio * self.split_preview_maximum);
                        let mut editor_scroll = PaneScroll::default();
                        let mut preview_scroll = PaneScroll::default();
                        ui.columns(2, |columns| {
                            columns[0].push_id("source-pane", |ui| {
                                editor_scroll = self.edit_pane(ui, index, editor_target);
                            });
                            columns[1].separator();
                            columns[1].push_id("preview-pane", |ui| {
                                preview_scroll = self.preview_pane(ui, index, preview_target);
                            });
                        });
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
            });
    }

    fn edit_pane(&mut self, ui: &mut Ui, index: usize, scroll_offset: Option<f32>) -> PaneScroll {
        let selection_before = self.editor_cursor.map(cursor_range_to_char_range);
        let mut scroll_area = ScrollArea::vertical().id_salt(("editor-scroll", index));
        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }
        let mut context_command = None;
        let palette = app_palette(self.state.dark);
        let output = scroll_area.show(ui, |ui| {
            let viewport = ui.available_size();
            ui.set_min_size(Vec2::new(viewport.x, viewport.y.max(420.0)));
            ui.add_space(28.0);
            let available_width = ui.available_width();
            let page_width = (available_width - 48.0)
                .clamp(280.0, 860.0)
                .min(available_width);
            let side_margin = ((available_width - page_width) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(side_margin);
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(Stroke::new(1.0, palette.border))
                    .corner_radius(12)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 18,
                        spread: 0,
                        color: if self.state.dark {
                            Color32::from_black_alpha(48)
                        } else {
                            Color32::from_black_alpha(18)
                        },
                    })
                    .inner_margin(Margin::symmetric(56, 38))
                    .show(ui, |ui| {
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.set_width((page_width - 112.0).max(160.0));
                            let available =
                                Vec2::new(ui.available_width(), (viewport.y - 144.0).max(360.0));
                            let row_height =
                                ui.text_style_height(&egui::TextStyle::Monospace).max(1.0);
                            let desired_rows = (available.y / row_height).max(20.0) as usize;
                            let editor_id = ui.make_persistent_id(("editor", index));
                            if let Some(cursor_range) = self.pending_editor_cursor.take() {
                                let mut state =
                                    TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
                                state.cursor.set_char_range(Some(cursor_range));
                                state.store(ui.ctx(), editor_id);
                                ui.memory_mut(|memory| memory.request_focus(editor_id));
                                self.editor_cursor = Some(cursor_range);
                            }
                            let input_action = editor_input_action(ui);
                            let mut editor_buffer =
                                TrackingTextBuffer::new(&mut self.documents[index].content);
                            let output = TextEdit::multiline(&mut editor_buffer)
                                .id(editor_id)
                                .font(egui::TextStyle::Monospace)
                                .code_editor()
                                .hint_text("Markdown 源码编辑区")
                                .desired_width(f32::INFINITY)
                                .desired_rows(desired_rows)
                                .lock_focus(true)
                                .show(ui);
                            set_accessible_label(ui.ctx(), editor_id, "Markdown 源码编辑区");
                            let mut before_content = editor_buffer.take_before();
                            drop(editor_buffer);
                            output.response.context_menu(|ui| {
                                if ui.button("撤销").clicked() {
                                    context_command = Some(AppCommand::Undo);
                                    ui.close();
                                }
                                if ui.button("重做").clicked() {
                                    context_command = Some(AppCommand::Redo);
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("粗体").clicked() {
                                    context_command =
                                        Some(AppCommand::Format(MarkdownCommand::Bold));
                                    ui.close();
                                }
                                if ui.button("斜体").clicked() {
                                    context_command =
                                        Some(AppCommand::Format(MarkdownCommand::Italic));
                                    ui.close();
                                }
                                if ui.button("链接").clicked() {
                                    context_command =
                                        Some(AppCommand::Format(MarkdownCommand::Link));
                                    ui.close();
                                }
                                if ui.button("行内代码").clicked() {
                                    context_command =
                                        Some(AppCommand::Format(MarkdownCommand::InlineCode));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("粘贴剪贴板图片").clicked() {
                                    context_command = Some(AppCommand::PasteImage);
                                    ui.close();
                                }
                                if ui.button("可视化编辑表格").clicked() {
                                    context_command = Some(AppCommand::EditTable);
                                    ui.close();
                                }
                            });
                            let mut selection_after =
                                output.cursor_range.map(cursor_range_to_char_range);
                            if let Some(cursor_range) = output.cursor_range {
                                self.editor_cursor = Some(cursor_range);
                            }
                            let focused = output.response.has_focus();
                            let mut changed = output.response.changed();
                            let mut kind = EditKind::Typing;
                            let mut cursor_adjusted = false;

                            if focused
                                && let (Some(url), Some(selection)) =
                                    (input_action.pasted_url.as_deref(), selection_before.clone())
                                && !selection.is_empty()
                                && let Some(before_content) = before_content.as_ref()
                            {
                                self.documents[index].content.clone_from(before_content);
                                if let Some(next) = editing::paste_url_as_markdown_link(
                                    &mut self.documents[index].content,
                                    selection,
                                    url,
                                ) {
                                    selection_after = Some(next);
                                    kind = EditKind::Other;
                                    changed = true;
                                    cursor_adjusted = true;
                                }
                            } else if focused
                                && input_action.tab
                                && let Some(before_content) = before_content.as_ref()
                            {
                                self.documents[index].content.clone_from(before_content);
                                let selection = selection_before.clone().unwrap_or_else(|| {
                                    let end = before_content.chars().count();
                                    end..end
                                });
                                selection_after = Some(editing::indent_selected_lines(
                                    &mut self.documents[index].content,
                                    selection,
                                    input_action.shift,
                                ));
                                kind = EditKind::Other;
                                changed = true;
                                cursor_adjusted = true;
                            } else if focused
                                && let (Some(typed), Some(selection)) =
                                    (input_action.typed_text.as_deref(), selection_before.clone())
                                && let Some(before_content) = before_content.as_ref()
                            {
                                let mut paired = before_content.clone();
                                if let Some(next) =
                                    editing::apply_smart_pair(&mut paired, selection, typed)
                                {
                                    changed = paired != *before_content;
                                    self.documents[index].content = paired;
                                    selection_after = Some(next);
                                    kind = EditKind::Other;
                                    cursor_adjusted = true;
                                }
                            } else if focused
                                && changed
                                && input_action.enter
                                && let Some(cursor) =
                                    selection_after.as_ref().map(|range| range.end)
                                && let Some(next) = editing::continue_markdown_line(
                                    &mut self.documents[index].content,
                                    cursor,
                                )
                            {
                                selection_after = Some(next);
                                cursor_adjusted = true;
                            }

                            if changed
                                && let Some(before_content) = before_content.take()
                                && self.documents[index].record_edit(
                                    before_content,
                                    selection_before,
                                    selection_after.clone(),
                                    kind,
                                )
                            {
                                if cursor_adjusted && let Some(selection) = selection_after {
                                    self.queue_editor_selection(selection);
                                }
                                self.status = "已修改".to_owned();
                            } else if cursor_adjusted && let Some(selection) = selection_after {
                                self.queue_editor_selection(selection);
                            }
                        });
                    });
            });
            ui.add_space(32.0);
        });
        if let Some(command) = context_command {
            self.execute(command);
        }
        PaneScroll {
            offset: output.state.offset.y,
            maximum: (output.content_size.y - output.inner_rect.height()).max(0.0),
            hovered: ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|position| output.inner_rect.contains(position)),
        }
    }

    fn preview_pane(
        &mut self,
        ui: &mut Ui,
        index: usize,
        scroll_offset: Option<f32>,
    ) -> PaneScroll {
        let viewport_height = ui.available_height();
        let before_content = self.documents[index].content.clone();
        let base_path = self.preview_base_path(index);
        let mut preview_content = prepare_native_preview(
            ui.ctx(),
            &before_content,
            &base_path,
            self.state.dark,
            &mut self.generated_svg_cache.borrow_mut(),
        );
        let base_uri = self.preview_base_uri(index);
        let local_links = markdown::local_link_destinations(&before_content);
        let palette = app_palette(self.state.dark);
        self.preview_cache.link_hooks_clear();
        for destination in &local_links {
            self.preview_cache.add_link_hook(destination);
        }

        let changed = {
            let cache = &mut self.preview_cache;
            let svg_cache = self.generated_svg_cache.clone();
            let dark = self.state.dark;
            let render_math = move |ui: &mut Ui, math: &str, inline: bool| {
                render_math_widget(ui, &mut svg_cache.borrow_mut(), math, inline, dark);
            };
            let mut scroll_area = ScrollArea::vertical().id_salt(("preview-scroll", index));
            if let Some(offset) = scroll_offset {
                scroll_area = scroll_area.vertical_scroll_offset(offset);
            }
            scroll_area.show(ui, |ui| {
                ui.add_space(28.0);
                let available_width = ui.available_width();
                let page_width = (available_width - 48.0)
                    .clamp(280.0, 860.0)
                    .min(available_width);
                let side_margin = ((available_width - page_width) * 0.5).max(0.0);
                let changed = ui
                    .horizontal(|ui| {
                        ui.add_space(side_margin);
                        egui::Frame::new()
                            .fill(palette.surface)
                            .stroke(Stroke::new(1.0, palette.border))
                            .corner_radius(12)
                            .shadow(egui::epaint::Shadow {
                                offset: [0, 4],
                                blur: 18,
                                spread: 0,
                                color: if self.state.dark {
                                    Color32::from_black_alpha(48)
                                } else {
                                    Color32::from_black_alpha(18)
                                },
                            })
                            .inner_margin(Margin::symmetric(56, 38))
                            .show(ui, |ui| {
                                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                    ui.set_width((page_width - 112.0).max(160.0));
                                    ui.set_min_height((viewport_height - 142.0).max(480.0));
                                    CommonMarkViewer::new()
                                        .default_implicit_uri_scheme(base_uri)
                                        .enable_scroll_to_heading(true)
                                        .render_math_fn(Some(&render_math))
                                        .show_mut(ui, cache, &mut preview_content)
                                        .response
                                        .changed()
                                })
                                .inner
                            })
                            .inner
                    })
                    .inner;
                ui.add_space(40.0);
                changed
            })
        };

        if changed.inner
            && let Some(next_content) =
                markdown::synchronize_task_markers(&before_content, &preview_content)
        {
            self.documents[index].content = next_content;
            self.documents[index].record_edit(before_content, None, None, EditKind::TaskList);
            self.status = "已更新任务列表".to_owned();
        }
        let clicked_link = local_links
            .into_iter()
            .find(|destination| self.preview_cache.get_link_hook(destination) == Some(true));
        if let Some(destination) = clicked_link {
            self.open_local_preview_link(index, &destination);
        }
        PaneScroll {
            offset: changed.state.offset.y,
            maximum: (changed.content_size.y - changed.inner_rect.height()).max(0.0),
            hovered: ui
                .ctx()
                .pointer_hover_pos()
                .is_some_and(|position| changed.inner_rect.contains(position)),
        }
    }

    fn apply_hybrid_cross_selection_input(&mut self, ui: &mut Ui, index: usize) {
        let document_id = self.documents[index].id();
        let Some(selection) = self
            .hybrid_cross_selection
            .filter(|selection| selection.document_id == document_id)
        else {
            self.hybrid_cross_selection = None;
            return;
        };
        let range = cursor_range_to_char_range(selection.cursor);
        if range.is_empty() {
            self.hybrid_cross_selection = None;
            return;
        }
        let action = take_cross_block_input(ui);
        let Some(action) = action else {
            return;
        };

        let before = self.documents[index].content.clone();
        let range = clamp_char_range(&before, range);
        let start = char_to_byte(&before, range.start);
        let end = char_to_byte(&before, range.end);
        let selected = selected_markdown_for_clipboard(&before[start..end]);
        if action.copy() {
            ui.ctx().copy_text(selected);
        }
        let Some(replacement) = action.replacement() else {
            return;
        };

        self.documents[index]
            .content
            .replace_range(start..end, replacement);
        let cursor = range.start + replacement.chars().count();
        let cursor = CCursorRange::one(CCursor::new(cursor));
        self.documents[index].record_edit(
            before,
            Some(range),
            Some(cursor_range_to_char_range(cursor)),
            EditKind::Typing,
        );
        self.editor_cursor = Some(cursor);
        self.pending_editor_cursor = Some(cursor);
        self.hybrid_cross_selection = None;
        self.hybrid_pointer_anchor = None;
        self.hybrid_ime_session = None;
        self.status = "已更新跨块选择".to_owned();
    }

    fn hybrid_pane(&mut self, ui: &mut Ui, index: usize) {
        self.apply_hybrid_cross_selection_input(ui, index);
        let viewport_height = ui.available_height();
        let source = self.documents[index].content.clone();
        let document_id = self.documents[index].id();
        let cursor_before = self.editor_cursor;
        let selection_before = cursor_before.map(cursor_range_to_char_range);
        let blocks = self.documents[index].blocks().to_vec();
        let base_uri = self.preview_base_uri(index);
        let base_path = self.preview_base_path(index);
        let local_links = markdown::local_link_destinations(&source);
        let preview_blocks = blocks
            .iter()
            .map(|block| {
                (
                    block.id,
                    prepare_native_preview(
                        ui.ctx(),
                        &source[block.range.clone()],
                        &base_path,
                        self.state.dark,
                        &mut self.generated_svg_cache.borrow_mut(),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        self.preview_cache.link_hooks_clear();
        for destination in &local_links {
            self.preview_cache.add_link_hook(destination);
        }
        let svg_cache = self.generated_svg_cache.clone();
        let dark = self.state.dark;
        let render_math = move |ui: &mut Ui, math: &str, inline: bool| {
            render_math_widget(ui, &mut svg_cache.borrow_mut(), math, inline, dark);
        };

        let mut pending_source_cursor = self.pending_editor_cursor.take();
        if let Some(cursor_range) = pending_source_cursor {
            let [selection_start, _] = cursor_range.sorted_cursors();
            let selected_block = block_for_char_index(&source, &blocks, selection_start.index.0);
            self.hybrid_active = Some((index, selected_block.id));
        }

        let active_id = self
            .hybrid_active
            .filter(|(document, id)| {
                *document == index && blocks.iter().any(|block| block.id == *id)
            })
            .map(|(_, id)| id)
            .or_else(|| source.is_empty().then(|| blocks[0].id));
        let document_title = self.documents[index].title();
        let ime_action = ui.input(|input| ime_frame_action(&input.events));
        let mut ime_session = self.hybrid_ime_session.take().filter(|session| {
            session.document_id == document_id && Some(session.block_id) == active_id
        });

        let mut pending_edit = None;
        let mut activate = None;
        let mut next_global_cursor = None;
        let mut cursor_adjusted = false;
        let mut page_rect = None;
        let mut active_editor_rect = None;
        let mut pointer_regions = Vec::<HybridPointerRegion>::new();
        let mut copied_code_block = false;
        let palette = app_palette(self.state.dark);

        ScrollArea::vertical()
            .id_salt(("hybrid-scroll", index))
            .show(ui, |ui| {
                ui.add_space(28.0);
                let available_width = ui.available_width();
                let page_width = (available_width - 48.0)
                    .clamp(280.0, 860.0)
                    .min(available_width);
                let side_margin = ((available_width - page_width) * 0.5).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(side_margin);
                    let page = egui::Frame::new()
                        .fill(palette.surface)
                        .stroke(Stroke::new(1.0, palette.border))
                        .corner_radius(12)
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 4],
                            blur: 18,
                            spread: 0,
                            color: if self.state.dark {
                                Color32::from_black_alpha(48)
                            } else {
                                Color32::from_black_alpha(18)
                            },
                        })
                        .inner_margin(Margin::symmetric(56, 38))
                        .show(ui, |ui| {
                            set_wysiwyg_document_accessibility(ui, &document_title);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                ui.set_width((page_width - 112.0).max(160.0));
                                ui.set_min_height((viewport_height - 142.0).max(480.0));
                                for (block_index, block) in blocks.iter().enumerate() {
                                    let source_block = &source[block.range.clone()];
                                    let code_content = fenced_code_content(source_block);
                                    let block_is_code = code_content.is_some();
                                    let mut code_surface_rect = None;
                                    if block_index > 0
                                        && Some(blocks[block_index - 1].id) != active_id
                                    {
                                        let gap =
                                            blocks[block_index - 1].range.end..block.range.start;
                                        let blank_lines =
                                            extra_inter_block_blank_lines(&source, gap.clone());
                                        if blank_lines > 0 {
                                            let line_height = ui
                                                .text_style_height(&egui::TextStyle::Body)
                                                .max(1.0);
                                            let response = ui
                                                .allocate_response(
                                                    Vec2::new(
                                                        ui.available_width(),
                                                        blank_lines as f32 * line_height,
                                                    ),
                                                    egui::Sense::click(),
                                                )
                                                .on_hover_cursor(egui::CursorIcon::Text)
                                                .on_hover_text("点击编辑段落之间的空行");
                                            if response.clicked()
                                                && let Some(position) =
                                                    response.interact_pointer_pos()
                                            {
                                                let line = ((position.y - response.rect.top())
                                                    / line_height)
                                                    .floor()
                                                    as usize;
                                                if let Some(cursor_byte) =
                                                    inter_block_blank_line_cursor_byte(
                                                        &source,
                                                        gap,
                                                        line.min(blank_lines - 1),
                                                    )
                                                {
                                                    activate = Some((
                                                        blocks[block_index - 1].id,
                                                        cursor_byte,
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    ui.push_id(("hybrid-block", block.id), |ui| {
                                        if Some(block.id) == active_id {
                                            let edit_range =
                                                hybrid_edit_range(&source, &blocks, block.id);
                                            let block_char_start =
                                                source[..edit_range.start].chars().count();
                                            let local_source_cursor_before =
                                                cursor_before.map(|cursor| {
                                                    cursor_range_saturating_sub(
                                                        cursor,
                                                        block_char_start,
                                                    )
                                                });
                                            let local_source_selection_before =
                                                local_source_cursor_before
                                                    .map(cursor_range_to_char_range);
                                            let (original_block, ime_visual_content) =
                                                if let Some(session) = ime_session.take() {
                                                    (
                                                        session.base_source,
                                                        Some(session.visual_content),
                                                    )
                                                } else {
                                                    (source[edit_range.clone()].to_owned(), None)
                                                };
                                            let had_ime_session = ime_visual_content.is_some();
                                            let projection = match local_source_selection_before
                                                .clone()
                                            {
                                                Some(selection) => {
                                                    VisualProjection::from_markdown_with_selection(
                                                        &original_block,
                                                        Some(selection),
                                                    )
                                                }
                                                None => {
                                                    VisualProjection::from_markdown(&original_block)
                                                }
                                            };
                                            let mut visual_content = ime_visual_content
                                                .unwrap_or_else(|| projection.text().to_owned());
                                            let visual_selection_before =
                                                local_source_selection_before.as_ref().map(
                                                    |selection| {
                                                        projection.visual_char_range(
                                                            &original_block,
                                                            selection.clone(),
                                                        )
                                                    },
                                                );
                                            let editor_id = ui.make_persistent_id((
                                                "hybrid-editor",
                                                index,
                                                block.id,
                                            ));
                                            let pending_local_cursor =
                                                pending_source_cursor.take().map(|cursor_range| {
                                                    cursor_range_saturating_sub(
                                                        cursor_range,
                                                        block_char_start,
                                                    )
                                                });
                                            let cursor_source_cursor =
                                                if pending_local_cursor.is_some() {
                                                    pending_local_cursor.as_ref()
                                                } else if had_ime_session {
                                                    None
                                                } else {
                                                    local_source_cursor_before.as_ref()
                                                };
                                            if let Some(local_cursor) = cursor_source_cursor {
                                                let local_source =
                                                    cursor_range_to_char_range(*local_cursor);
                                                let local_visual = projection.visual_char_range(
                                                    &original_block,
                                                    local_source,
                                                );
                                                let mut state =
                                                    TextEdit::load_state(ui.ctx(), editor_id)
                                                        .unwrap_or_default();
                                                state.cursor.set_char_range(Some(
                                                    cursor_range_with_direction(
                                                        local_visual,
                                                        *local_cursor,
                                                    ),
                                                ));
                                                state.store(ui.ctx(), editor_id);
                                            }
                                            if pending_local_cursor.is_some() {
                                                ui.memory_mut(|memory| {
                                                    memory.request_focus(editor_id)
                                                });
                                            }

                                            let block_is_code =
                                                is_fenced_code_block(&original_block);
                                            let frame = if block_is_code {
                                                egui::Frame::new()
                                                    .fill(palette.code_bg)
                                                    .stroke(Stroke::new(1.0, palette.border))
                                                    .corner_radius(8)
                                                    .inner_margin(Margin::symmetric(14, 10))
                                            } else {
                                                egui::Frame::new()
                                                    .inner_margin(Margin::symmetric(0, 3))
                                            };
                                            let editor_frame = frame.show(ui, |ui| {
                                                if block_is_code
                                                    && let Some(language) =
                                                        fenced_code_language(&original_block)
                                                {
                                                    ui.label(
                                                        RichText::new(language.to_uppercase())
                                                            .monospace()
                                                            .size(11.0)
                                                            .color(palette.secondary),
                                                    );
                                                    ui.add_space(4.0);
                                                }
                                                let desired_rows = if block_is_code {
                                                    multiline_edit_rows(&visual_content).max(2)
                                                } else {
                                                    multiline_edit_rows(&visual_content)
                                                };
                                                let input_action = editor_input_action(ui);
                                                let mut layouter =
                                                    |ui: &Ui,
                                                     buffer: &dyn egui::TextBuffer,
                                                     wrap_width: f32| {
                                                        wysiwyg_layout(
                                                            ui,
                                                            buffer.as_str(),
                                                            &projection,
                                                            wrap_width,
                                                            palette,
                                                            !block_is_code,
                                                        )
                                                    };
                                                let mut editor =
                                                    TextEdit::multiline(&mut visual_content)
                                                        .id(editor_id)
                                                        .frame(egui::Frame::NONE)
                                                        .layouter(&mut layouter)
                                                        .hint_text(if block_is_code {
                                                            "输入代码…"
                                                        } else {
                                                            "开始写作…"
                                                        })
                                                        .desired_width(f32::INFINITY)
                                                        .desired_rows(desired_rows)
                                                        .lock_focus(true);
                                                if block_is_code {
                                                    editor = editor.code_editor();
                                                }
                                                let inline_code_background = (!block_is_code)
                                                    .then(|| ui.painter().add(egui::Shape::Noop));
                                                let mut output = editor.show(ui);
                                                pointer_regions.push(HybridPointerRegion {
                                                    block_id: block.id,
                                                    source_range: edit_range.clone(),
                                                    rect: output.response.rect,
                                                    atomic_range: block_is_code
                                                        .then(|| block.range.clone()),
                                                    mapping: HybridPointerMapping::Exact(
                                                        TextProjectionPreview {
                                                            galley: Arc::clone(&output.galley),
                                                            galley_pos: output.galley_pos,
                                                            projection: projection.clone(),
                                                        },
                                                    ),
                                                });
                                                if let Some(shape_index) = inline_code_background {
                                                    let runs = projection.runs_for(&visual_content);
                                                    ui.painter().set(
                                                        shape_index,
                                                        egui::Shape::Vec(
                                                            rounded_inline_code_backgrounds(
                                                                &output.galley,
                                                                output.galley_pos,
                                                                &runs,
                                                                palette,
                                                            ),
                                                        ),
                                                    );
                                                    paint_inline_code_delimiters(
                                                        ui,
                                                        &output.galley,
                                                        output.galley_pos,
                                                        &runs,
                                                        palette,
                                                    );
                                                }
                                                if output.response.has_focus() {
                                                    let accessible_remainder =
                                                        accessible_document_remainder(
                                                            &source, &blocks, block.id,
                                                        );
                                                    append_accessible_text_runs(
                                                        ui,
                                                        editor_id,
                                                        (document_id, block.id),
                                                        &accessible_remainder,
                                                        output.response.rect,
                                                    );
                                                }
                                                set_accessible_label(
                                                    ui.ctx(),
                                                    editor_id,
                                                    format!(
                                                        "从第 {} 行开始的所见即所得编辑块",
                                                        block.line
                                                    ),
                                                );
                                                let visual_cursor_after =
                                                    text_edit_cursor_after_input(&output);
                                                let mut visual_selection_after =
                                                    visual_cursor_after
                                                        .map(cursor_range_to_char_range);
                                                if output.response.dragged()
                                                    && let Some(anchor_position) = ui
                                                        .input(|input| input.pointer.press_origin())
                                                {
                                                    // `TextEditOutput::cursor_range` is captured
                                                    // before pointer interaction in egui 0.35. On
                                                    // Windows a right-to-left drag can therefore be
                                                    // observed as only the moving caret. Rebuild the
                                                    // directed range from the stable press origin
                                                    // and current pointer position using the exact
                                                    // galley hit-test used by TextEdit itself.
                                                    let anchor = text_edit_cursor_at_position(
                                                        &output,
                                                        anchor_position,
                                                    );
                                                    let current_position = ui
                                                        .input(|input| input.pointer.interact_pos())
                                                        .unwrap_or(anchor_position);
                                                    let current = text_edit_cursor_at_position(
                                                        &output,
                                                        current_position,
                                                    );
                                                    let cursor = CCursorRange {
                                                        primary: current,
                                                        secondary: anchor,
                                                        h_pos: None,
                                                    };
                                                    visual_selection_after =
                                                        Some(cursor_range_to_char_range(cursor));
                                                    output
                                                        .state
                                                        .cursor
                                                        .set_char_range(Some(cursor));
                                                    output
                                                        .state
                                                        .clone()
                                                        .store(ui.ctx(), output.response.id);
                                                }
                                                let focused = output.response.has_focus();
                                                let mut changed = output.response.changed();
                                                let mut kind = EditKind::Typing;
                                                let mut source_update = None;
                                                let mut boundary_input_handled = false;
                                                let defer_ime = focused
                                                    && (ime_action == ImeFrameAction::Preedit
                                                        || had_ime_session
                                                            && ime_action == ImeFrameAction::None);

                                                if defer_ime {
                                                    changed = false;
                                                } else if had_ime_session
                                                    && ime_action == ImeFrameAction::Cancel
                                                {
                                                    ime_session = None;
                                                    changed = false;
                                                } else if had_ime_session
                                                    && ime_action == ImeFrameAction::Commit
                                                {
                                                    ime_session = None;
                                                } else if !defer_ime
                                                    && focused
                                                    && !changed
                                                    && !input_action.horizontal_modified
                                                    && let Some(selection) =
                                                        local_source_selection_before.clone()
                                                    && let Some(selection) =
                                                        move_across_hidden_inline_code_boundary(
                                                            &original_block,
                                                            selection,
                                                            input_action.left,
                                                            input_action.right,
                                                        )
                                                {
                                                    next_global_cursor = Some(CCursorRange::two(
                                                        CCursor::new(
                                                            block_char_start + selection.start,
                                                        ),
                                                        CCursor::new(
                                                            block_char_start + selection.end,
                                                        ),
                                                    ));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                }

                                                if !boundary_input_handled
                                                    && !defer_ime
                                                    && focused
                                                    && let (Some(url), Some(selection)) = (
                                                        input_action.pasted_url.as_deref(),
                                                        visual_selection_before.clone(),
                                                    )
                                                    && !selection.is_empty()
                                                {
                                                    let source_selection = projection
                                                        .source_char_range(
                                                            &original_block,
                                                            selection,
                                                        );
                                                    let mut updated = original_block.clone();
                                                    if let Some(next) =
                                                        editing::paste_url_as_markdown_link(
                                                            &mut updated,
                                                            source_selection,
                                                            url,
                                                        )
                                                    {
                                                        source_update = Some((updated, next));
                                                        kind = EditKind::Other;
                                                        changed = true;
                                                        cursor_adjusted = true;
                                                    }
                                                } else if !defer_ime && focused && input_action.tab
                                                {
                                                    let selection = visual_selection_before
                                                        .clone()
                                                        .unwrap_or_else(|| {
                                                            let end =
                                                                visual_content.chars().count();
                                                            end..end
                                                        });
                                                    let source_selection = projection
                                                        .source_char_range(
                                                            &original_block,
                                                            selection,
                                                        );
                                                    let mut updated = original_block.clone();
                                                    let next = editing::indent_selected_lines(
                                                        &mut updated,
                                                        source_selection,
                                                        input_action.shift,
                                                    );
                                                    source_update = Some((updated, next));
                                                    kind = EditKind::Other;
                                                    changed = true;
                                                    cursor_adjusted = true;
                                                } else if !defer_ime
                                                    && focused
                                                    && let (Some(typed), Some(selection)) = (
                                                        input_action.typed_text.as_deref(),
                                                        visual_selection_before.clone(),
                                                    )
                                                {
                                                    let mut paired = projection.text().to_owned();
                                                    if let Some(next) = editing::apply_smart_pair(
                                                        &mut paired,
                                                        selection,
                                                        typed,
                                                    ) {
                                                        changed = paired != projection.text();
                                                        visual_content = paired;
                                                        visual_selection_after = Some(next);
                                                        kind = EditKind::Other;
                                                        cursor_adjusted = true;
                                                    }
                                                } else if !defer_ime
                                                    && focused
                                                    && block_is_code
                                                    && (input_action.backspace
                                                        || input_action.delete)
                                                    && !changed
                                                    && projection.text().trim().is_empty()
                                                {
                                                    let replacement_range =
                                                        code_block_removal_range(
                                                            &source,
                                                            &blocks,
                                                            block_index,
                                                        );
                                                    let cursor = source[..replacement_range.start]
                                                        .chars()
                                                        .count();
                                                    pending_edit = Some((
                                                        replacement_range,
                                                        String::new(),
                                                        EditKind::Typing,
                                                    ));
                                                    next_global_cursor = Some(CCursorRange::one(
                                                        CCursor::new(cursor),
                                                    ));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                } else if !defer_ime
                                                    && focused
                                                    && input_action.backspace
                                                    && !changed
                                                    && visual_selection_before.as_ref().is_some_and(
                                                        |selection| {
                                                            selection.is_empty()
                                                                && selection.start == 0
                                                        },
                                                    )
                                                    && let Some((replacement_range, cursor)) =
                                                        boundary_backspace_edit(
                                                            &source,
                                                            edit_range.clone(),
                                                        )
                                                {
                                                    pending_edit = Some((
                                                        replacement_range,
                                                        original_block.clone(),
                                                        EditKind::Typing,
                                                    ));
                                                    next_global_cursor = Some(CCursorRange::one(
                                                        CCursor::new(cursor),
                                                    ));
                                                    cursor_adjusted = true;
                                                    boundary_input_handled = true;
                                                }

                                                if !boundary_input_handled
                                                    && !defer_ime
                                                    && source_update.is_none()
                                                    && changed
                                                    && let Some(selection) =
                                                        visual_selection_after.clone()
                                                    && let Some(mut update) = projection.apply_edit(
                                                        &original_block,
                                                        &visual_content,
                                                        selection,
                                                    )
                                                {
                                                    if focused && input_action.enter {
                                                        if !input_action.shift
                                                            && let Some(selection) =
                                                                complete_fenced_code_on_enter(
                                                                    &mut update.source,
                                                                    update.selection.clone(),
                                                                )
                                                        {
                                                            update.selection = selection;
                                                        } else {
                                                            update.selection =
                                                                complete_visual_enter(
                                                                    &mut update.source,
                                                                    update.selection,
                                                                    input_action.shift,
                                                                );
                                                        }
                                                    }
                                                    if let Some(selection) =
                                                        consume_paired_fenced_code_closer(
                                                            &mut update.source,
                                                            update.selection.clone(),
                                                        )
                                                    {
                                                        update.selection = selection;
                                                    } else if let Some(selection) =
                                                        complete_bare_fenced_code_after_typing(
                                                            &mut update.source,
                                                            update.selection.clone(),
                                                        )
                                                    {
                                                        update.selection = selection;
                                                    }
                                                    cursor_adjusted = true;
                                                    source_update =
                                                        Some((update.source, update.selection));
                                                }

                                                if !boundary_input_handled
                                                    && !defer_ime
                                                    && source_update.is_none()
                                                    && focused
                                                    && block_is_code
                                                    && !input_action.horizontal_modified
                                                    && (input_action.right || input_action.down)
                                                    && visual_selection_before.as_ref().is_some_and(
                                                        |selection| {
                                                            selection.is_empty()
                                                                && selection.end
                                                                    == projection
                                                                        .text()
                                                                        .chars()
                                                                        .count()
                                                        },
                                                    )
                                                {
                                                    let mut updated = original_block.clone();
                                                    if let Some(selection) =
                                                        paragraph_after_fenced_code(&mut updated)
                                                    {
                                                        source_update = Some((updated, selection));
                                                        cursor_adjusted = true;
                                                    }
                                                }

                                                if defer_ime {
                                                    // IME pre-edit text belongs to the composition,
                                                    // not to the Markdown document or its undo history.
                                                    // Keeping this visual buffer alive lets egui replace
                                                    // the previous pre-edit range on the next frame.
                                                    ime_session = Some(HybridImeSession {
                                                        document_id,
                                                        block_id: block.id,
                                                        base_source: original_block,
                                                        visual_content,
                                                    });
                                                } else if !boundary_input_handled
                                                    && let Some((updated, selection)) =
                                                        source_update
                                                {
                                                    next_global_cursor = Some(CCursorRange::two(
                                                        CCursor::new(
                                                            block_char_start + selection.start,
                                                        ),
                                                        CCursor::new(
                                                            block_char_start + selection.end,
                                                        ),
                                                    ));
                                                    pending_edit =
                                                        Some((edit_range.clone(), updated, kind));
                                                } else if !boundary_input_handled
                                                    && let Some(selection) = visual_selection_after
                                                {
                                                    let source_selection =
                                                        source_selection_after_visual_input(
                                                            &projection,
                                                            &original_block,
                                                            local_source_selection_before.as_ref(),
                                                            visual_selection_before.as_ref(),
                                                            selection,
                                                        );
                                                    let local_cursor = if let Some(visual_cursor) =
                                                        visual_cursor_after
                                                    {
                                                        cursor_range_with_direction(
                                                            source_selection,
                                                            visual_cursor,
                                                        )
                                                    } else {
                                                        CCursorRange::two(
                                                            CCursor::new(source_selection.start),
                                                            CCursor::new(source_selection.end),
                                                        )
                                                    };
                                                    next_global_cursor = Some(cursor_range_add(
                                                        local_cursor,
                                                        block_char_start,
                                                    ));
                                                }
                                            });
                                            active_editor_rect = Some(editor_frame.response.rect);
                                            if block_is_code {
                                                code_surface_rect =
                                                    Some(editor_frame.response.rect);
                                                if let Some(region) = pointer_regions.last_mut()
                                                    && region.block_id == block.id
                                                {
                                                    region.rect = editor_frame.response.rect;
                                                }
                                            }
                                        } else {
                                            let block_text = preview_blocks
                                                .get(&block.id)
                                                .map(String::as_str)
                                                .unwrap_or(source_block);
                                            let shown = ui.scope(|ui| {
                                                ui.add_space(6.0);
                                                let projection_preview =
                                                    show_text_projection_preview(
                                                        ui,
                                                        source_block,
                                                        palette,
                                                    );
                                                if projection_preview.is_none() {
                                                    let block_text =
                                                        preserve_soft_breaks_for_wysiwyg(
                                                            block_text,
                                                        );
                                                    CommonMarkViewer::new()
                                                        .default_implicit_uri_scheme(
                                                            base_uri.clone(),
                                                        )
                                                        .render_math_fn(Some(&render_math))
                                                        .show(
                                                            ui,
                                                            &mut self.preview_cache,
                                                            &block_text,
                                                        );
                                                }
                                                ui.add_space(6.0);
                                                projection_preview
                                            });
                                            let projection_preview = shown.inner;
                                            let activation_id =
                                                ui.make_persistent_id(("activate-block", block.id));
                                            let response = ui
                                                .interact(
                                                    shown.response.rect,
                                                    activation_id,
                                                    egui::Sense::click_and_drag(),
                                                )
                                                .on_hover_text(format!(
                                                    "点击编辑第 {} 行开始的 Markdown 块",
                                                    block.line
                                                ));
                                            set_markdown_preview_accessibility(
                                                &response,
                                                source_block,
                                            );
                                            pointer_regions.push(HybridPointerRegion {
                                                block_id: block.id,
                                                source_range: block.range.clone(),
                                                rect: shown.response.rect,
                                                atomic_range: block_is_code
                                                    .then(|| block.range.clone()),
                                                mapping: projection_preview
                                                    .as_ref()
                                                    .cloned()
                                                    .map_or(
                                                        HybridPointerMapping::Approximate,
                                                        HybridPointerMapping::Exact,
                                                    ),
                                            });
                                            if response.clicked() {
                                                let local_source_byte = response
                                                    .interact_pointer_pos()
                                                    .map(|position| {
                                                        projection_preview.as_ref().map_or_else(
                                                            || {
                                                                let width =
                                                                    response.rect.width().max(1.0);
                                                                let height =
                                                                    response.rect.height().max(1.0);
                                                                SourceMap::from_markdown(
                                                                    source_block,
                                                                )
                                                                .source_byte_at_normalized_point(
                                                                    (position.x
                                                                        - response.rect.left())
                                                                        / width,
                                                                    (position.y
                                                                        - response.rect.top())
                                                                        / height,
                                                                )
                                                            },
                                                            |preview| {
                                                                preview.source_byte_at_position(
                                                                    source_block,
                                                                    position,
                                                                )
                                                            },
                                                        )
                                                    })
                                                    .unwrap_or_default();
                                                activate = Some((
                                                    block.id,
                                                    block.range.start + local_source_byte,
                                                ));
                                            }
                                            if block_is_code {
                                                code_surface_rect = Some(shown.response.rect);
                                            }
                                        }
                                    });
                                    if let (Some(rect), Some(content)) =
                                        (code_surface_rect, code_content)
                                        && show_code_copy_button(
                                            ui,
                                            (document_id, block.id),
                                            rect,
                                            content,
                                            palette,
                                        )
                                    {
                                        copied_code_block = true;
                                    }

                                    if block_is_code {
                                        let response = ui
                                            .allocate_response(
                                                Vec2::new(ui.available_width(), 24.0),
                                                egui::Sense::click(),
                                            )
                                            .on_hover_cursor(egui::CursorIcon::Text)
                                            .on_hover_text("双击在代码块后新建普通段落");
                                        if response.hovered() {
                                            ui.painter().text(
                                                response.rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "双击新建段落",
                                                FontId::proportional(11.0),
                                                palette.secondary.gamma_multiply(0.75),
                                            );
                                        }
                                        if response.double_clicked() && pending_edit.is_none() {
                                            let (tail_range, replacement, cursor) =
                                                paragraph_after_code_double_click(
                                                    &source,
                                                    &blocks,
                                                    block_index,
                                                );
                                            next_global_cursor =
                                                Some(CCursorRange::one(CCursor::new(cursor)));
                                            if let Some(replacement) = replacement {
                                                pending_edit = Some((
                                                    tail_range,
                                                    replacement,
                                                    EditKind::Typing,
                                                ));
                                            }
                                            cursor_adjusted = true;
                                        }
                                    } else {
                                        ui.add_space(8.0);
                                    }
                                }
                                ui.add_space(60.0);
                            });
                        });
                    page_rect = Some(page.response.rect);
                });
                ui.add_space(40.0);
            });

        if copied_code_block {
            self.status = "已复制代码块".to_owned();
        }

        let pointer = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_released(),
                input.pointer.primary_down(),
                input.pointer.is_decidedly_dragging(),
                input.pointer.press_origin(),
                input.pointer.interact_pos(),
            )
        });
        if pointer.0
            && let Some(origin) = pointer.4
            && let Some((block_id, source_char)) =
                hybrid_pointer_hit(&source, &pointer_regions, origin)
        {
            self.hybrid_pointer_anchor = Some(HybridPointerAnchor {
                document_id,
                block_id,
                source_char,
            });
            self.hybrid_cross_selection = None;
        }
        if pointer.2
            && pointer.3
            && let (Some(anchor), Some(position)) = (self.hybrid_pointer_anchor, pointer.5)
            && anchor.document_id == document_id
            && let Some((current_block, current_char)) =
                hybrid_pointer_hit(&source, &pointer_regions, position)
        {
            if current_block != anchor.block_id {
                let cursor = snap_atomic_cross_block_selection(
                    &source,
                    &blocks,
                    anchor.block_id,
                    anchor.source_char,
                    current_block,
                    current_char,
                );
                self.hybrid_cross_selection = Some(HybridCrossSelection {
                    document_id,
                    cursor,
                });
                next_global_cursor = Some(cursor);
            } else {
                self.hybrid_cross_selection = None;
            }
        }
        if pointer.1 {
            self.hybrid_pointer_anchor = None;
        }
        if let Some(selection) = self
            .hybrid_cross_selection
            .filter(|selection| selection.document_id == document_id)
        {
            next_global_cursor = Some(selection.cursor);
            paint_hybrid_cross_selection(ui, &source, &pointer_regions, selection.cursor);
        }

        let deactivate = ui.input(|input| {
            input.key_pressed(Key::Escape)
                || input.pointer.any_click()
                    && input.pointer.interact_pos().is_some_and(|position| {
                        page_rect.is_some_and(|rect| rect.contains(position))
                            && active_editor_rect.is_some_and(|rect| !rect.contains(position))
                    })
        });

        if let Some(cursor_range) = next_global_cursor {
            self.editor_cursor = Some(cursor_range);
        }
        if let Some((range, replacement, kind)) = pending_edit {
            self.documents[index]
                .content
                .replace_range(range.clone(), &replacement);
            let selection_after = next_global_cursor.map(cursor_range_to_char_range);
            self.documents[index].record_edit(
                source.clone(),
                selection_before,
                selection_after,
                kind,
            );
            let cursor_block_id = next_global_cursor.map(|cursor_range| {
                let cursor = cursor_range.sorted_cursors()[1].index.0;
                let updated_source = self.documents[index].content.clone();
                let updated_blocks = self.documents[index].blocks().to_vec();
                block_for_char_index(&updated_source, &updated_blocks, cursor).id
            });
            if let Some(next_active_id) = cursor_block_id.or(active_id) {
                self.hybrid_active = Some((index, next_active_id));
                if cursor_block_id != active_id {
                    self.pending_editor_cursor = next_global_cursor;
                }
            }
            if !copied_code_block {
                self.status = "已更新当前 Markdown 块".to_owned();
            }
        }
        if cursor_adjusted {
            self.pending_editor_cursor = next_global_cursor;
        }
        if deactivate && self.hybrid_cross_selection.is_none() {
            self.hybrid_active = None;
            ime_session = None;
        }
        if let Some((id, start)) = activate {
            let char_start = source[..start].chars().count();
            self.hybrid_active = Some((index, id));
            ime_session = None;
            self.queue_editor_selection(char_start..char_start);
        }
        self.hybrid_ime_session = ime_session;

        let clicked_link = local_links
            .into_iter()
            .find(|destination| self.preview_cache.get_link_hook(destination) == Some(true));
        if let Some(destination) = clicked_link {
            self.open_local_preview_link(index, &destination);
        }
    }

    fn preview_base_uri(&self, index: usize) -> String {
        file_uri_base(&self.preview_base_path(index))
    }

    fn preview_base_path(&self, index: usize) -> PathBuf {
        self.documents[index]
            .path
            .as_deref()
            .and_then(Path::parent)
            .or_else(|| {
                self.workspace
                    .as_ref()
                    .map(|workspace| workspace.root.as_path())
            })
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn open_local_preview_link(&mut self, index: usize, destination: &str) {
        let path_part = destination.split(['#', '?']).next().unwrap_or(destination);
        let link_path = PathBuf::from(path_part);
        let base = self.documents[index]
            .path
            .as_deref()
            .and_then(Path::parent)
            .or_else(|| {
                self.workspace
                    .as_ref()
                    .map(|workspace| workspace.root.as_path())
            })
            .unwrap_or_else(|| Path::new("."));
        let resolved = if link_path.is_absolute() {
            link_path
        } else {
            base.join(link_path)
        };
        let allowed_root = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.root.as_path())
            .unwrap_or(base);

        if !resolved.exists() {
            self.status = format!("链接目标不存在：{}", resolved.display());
        } else if !path_is_within(&resolved, allowed_root) {
            self.status = format!("已阻止打开工作区之外的本地路径：{}", resolved.display());
        } else if is_markdown_path(&resolved) {
            self.open_paths([resolved]);
        } else if let Err(error) = open::that(&resolved) {
            self.status = format!("无法打开 {}：{error}", resolved.display());
        }
    }

    fn status_bar(&mut self, root: &mut Ui) {
        let document_info = self
            .active
            .and_then(|index| self.documents.get(index))
            .map(|document| {
                format!(
                    "{} 字符 · {} 词 · {} 行 · {} · {}",
                    document.analysis.characters,
                    document.analysis.words,
                    document.analysis.lines,
                    document.encoding.label(),
                    document.line_ending.label()
                )
            })
            .unwrap_or_default();

        let palette = app_palette(self.state.dark);
        let source_mode = self.state.view_mode == ViewMode::Edit;
        let mut toggle_source_mode = false;
        Panel::bottom("status")
            .exact_size(31.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.toolbar)
                    .inner_margin(Margin::symmetric(10, 4))
                    .stroke(Stroke::new(1.0, palette.border)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").size(8.0).color(palette.accent));
                    ui.label(RichText::new(&self.status).small().color(palette.secondary));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(AppIconButton {
                                icon: AppIcon::Source,
                                selected: source_mode,
                                palette,
                                size: 22.0,
                            })
                            .on_hover_text(if source_mode {
                                "返回所见即所得模式"
                            } else {
                                "切换到 Markdown 源码模式"
                            })
                            .clicked()
                        {
                            toggle_source_mode = true;
                        }
                        ui.separator();
                        ui.label(
                            RichText::new(document_info)
                                .small()
                                .color(palette.secondary),
                        );
                    });
                });
            });
        if toggle_source_mode {
            let next_mode = if source_mode {
                ViewMode::Hybrid
            } else {
                ViewMode::Edit
            };
            self.execute(AppCommand::SetView(next_mode));
            self.status = if next_mode == ViewMode::Edit {
                "已切换到 Markdown 源码模式"
            } else {
                "已返回所见即所得模式"
            }
            .to_owned();
        }
    }

    fn confirm_application_close(&mut self, ctx: &Context) {
        if self.allow_close || !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if !self.documents.iter().any(|document| document.dirty) {
            self.allow_close = true;
            return;
        }

        ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        let result = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title("退出 RUPORA")
            .set_description("仍有未保存的文档。确定放弃修改并退出吗？")
            .set_buttons(MessageButtons::YesNo)
            .show();
        if result == MessageDialogResult::Yes {
            self.allow_close = true;
            self.discard_recovery_on_exit = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

impl eframe::App for RuporaApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));
        for document in &mut self.documents {
            document.refresh_derived_state_if_idle(Duration::from_millis(120));
            if document.derived_state_is_stale() {
                ctx.request_repaint_after(Duration::from_millis(40));
            }
        }
        self.poll_instance_requests(ctx);
        self.poll_update_check();
        self.poll_extension();
        self.handle_shortcuts(ctx);
        self.handle_dropped_files(ctx);
        self.save_recovery_snapshot_if_due();
        self.check_external_changes_if_due();
        self.confirm_application_close(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        self.top_bar(ui);
        self.find_bar(ui);
        self.command_palette(ui);
        self.about_window(ui);
        self.shortcut_settings(ui);
        self.external_diff_window(ui);
        self.table_editor_window(ui);
        self.external_change_bar(ui);
        self.status_bar(ui);
        self.sidebar(ui);
        self.outline(ui);
        self.editor_tabs(ui);
        self.editor(ui);
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        self.store_active_view_state();
        self.state.session_files = self
            .documents
            .iter()
            .filter_map(|document| document.path.clone())
            .collect();
        self.state.active_session_file = self
            .active
            .and_then(|index| self.documents.get(index))
            .and_then(|document| document.path.clone());
        eframe::set_value(storage, APP_STATE_KEY, &self.state);
        eframe::set_value(storage, UI_EXPERIENCE_KEY, &CURRENT_UI_EXPERIENCE);
        let _ = self.recovery_store.save(&self.documents);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.discard_recovery_on_exit || self.documents.iter().all(|document| !document.dirty) {
            let _ = self.recovery_store.clear();
        } else {
            let _ = self.recovery_store.save(&self.documents);
        }
        diagnostics::append_event("INFO", "RUPORA exited normally").ok();
    }
}

#[derive(Clone, Copy)]
enum AppIcon {
    New,
    Folder,
    Save,
    Sidebar,
    Outline,
    Theme,
    Source,
    File,
    Close,
}

impl AppIcon {
    const fn accessible_label(self) -> &'static str {
        match self {
            Self::New => "新建",
            Self::Folder => "打开",
            Self::Save => "保存",
            Self::Sidebar => "资源管理器",
            Self::Outline => "文档大纲",
            Self::Theme => "切换主题",
            Self::Source => "源码 / 所见即所得",
            Self::File => "Markdown 文档",
            Self::Close => "关闭",
        }
    }
}

struct AppIconButton {
    icon: AppIcon,
    selected: bool,
    palette: AppPalette,
    size: f32,
}

impl egui::Widget for AppIconButton {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::click());
        let enabled = ui.is_enabled();
        let fill = if self.selected {
            self.palette.accent_soft
        } else if response.hovered() && enabled {
            self.palette.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, fill);
        let color = if !enabled {
            self.palette.secondary.gamma_multiply(0.45)
        } else if self.selected {
            self.palette.accent
        } else {
            self.palette.secondary
        };
        paint_app_icon(
            ui.painter(),
            rect.shrink(self.size * 0.25),
            self.icon,
            color,
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                self.icon.accessible_label(),
            )
        });
        response
    }
}

fn icon_button_widget(icon: AppIcon, selected: bool, palette: AppPalette) -> AppIconButton {
    AppIconButton {
        icon,
        selected,
        palette,
        size: 30.0,
    }
}

fn app_icon_button(
    ui: &mut Ui,
    icon: AppIcon,
    selected: bool,
    tooltip: &str,
    palette: AppPalette,
) -> egui::Response {
    ui.add(icon_button_widget(icon, selected, palette))
        .on_hover_text(tooltip)
}

fn paint_app_icon(painter: &egui::Painter, rect: egui::Rect, icon: AppIcon, color: Color32) {
    let stroke = Stroke::new(1.45, color);
    let center = rect.center();
    let left = rect.left();
    let right = rect.right();
    let top = rect.top();
    let bottom = rect.bottom();
    match icon {
        AppIcon::New => {
            painter.line_segment(
                [egui::pos2(left, center.y), egui::pos2(right, center.y)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(center.x, top), egui::pos2(center.x, bottom)],
                stroke,
            );
        }
        AppIcon::Folder => {
            painter.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(left, top + 3.0),
                    egui::pos2(left + 5.0, top + 3.0),
                    egui::pos2(left + 7.0, top + 5.0),
                    egui::pos2(right, top + 5.0),
                    egui::pos2(right, bottom - 1.0),
                    egui::pos2(left, bottom - 1.0),
                ],
                stroke,
            ));
        }
        AppIcon::Save => {
            painter.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(left + 1.0, top),
                    egui::pos2(right - 2.0, top),
                    egui::pos2(right, top + 2.0),
                    egui::pos2(right, bottom),
                    egui::pos2(left + 1.0, bottom),
                ],
                stroke,
            ));
            painter.line_segment(
                [
                    egui::pos2(left + 4.0, top),
                    egui::pos2(left + 4.0, top + 5.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left + 4.0, bottom - 4.0),
                    egui::pos2(right - 3.0, bottom - 4.0),
                ],
                stroke,
            );
        }
        AppIcon::Sidebar => {
            painter.add(egui::Shape::closed_line(
                vec![
                    egui::pos2(left, top),
                    egui::pos2(right, top),
                    egui::pos2(right, bottom),
                    egui::pos2(left, bottom),
                ],
                stroke,
            ));
            painter.line_segment(
                [egui::pos2(left + 4.5, top), egui::pos2(left + 4.5, bottom)],
                stroke,
            );
        }
        AppIcon::Outline => {
            for row in 0..3 {
                let y = top + 2.0 + row as f32 * 5.0;
                painter.circle_filled(egui::pos2(left + 1.5, y), 1.15, color);
                painter.line_segment([egui::pos2(left + 5.0, y), egui::pos2(right, y)], stroke);
            }
        }
        AppIcon::Theme => {
            painter.circle_stroke(center, 3.3, stroke);
            for index in 0..8 {
                let angle = index as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 5.3, center + direction * 7.0], stroke);
            }
        }
        AppIcon::Source => {
            painter.line_segment(
                [
                    egui::pos2(center.x - 2.0, top + 1.5),
                    egui::pos2(left, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left, center.y),
                    egui::pos2(center.x - 2.0, bottom - 1.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 2.0, top + 1.5),
                    egui::pos2(right, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right, center.y),
                    egui::pos2(center.x + 2.0, bottom - 1.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 1.5, top),
                    egui::pos2(center.x - 1.5, bottom),
                ],
                stroke,
            );
        }
        AppIcon::File => {
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(left + 2.0, top),
                    egui::pos2(right - 4.0, top),
                    egui::pos2(right, top + 4.0),
                    egui::pos2(right, bottom),
                    egui::pos2(left + 2.0, bottom),
                    egui::pos2(left + 2.0, top),
                ],
                stroke,
            ));
            painter.line_segment(
                [
                    egui::pos2(right - 4.0, top),
                    egui::pos2(right - 4.0, top + 4.0),
                ],
                stroke,
            );
        }
        AppIcon::Close => {
            painter.line_segment(
                [
                    egui::pos2(left + 2.0, top + 2.0),
                    egui::pos2(right - 2.0, bottom - 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right - 2.0, top + 2.0),
                    egui::pos2(left + 2.0, bottom - 2.0),
                ],
                stroke,
            );
        }
    }
}

fn outline_row(ui: &mut Ui, heading: &Heading) -> bool {
    let indent = (heading.level.saturating_sub(1) as f32) * 12.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        ui.selectable_label(false, &heading.text)
            .on_hover_text(format!("第 {} 行 · H{}", heading.line, heading.level))
            .clicked()
    })
    .inner
}

fn block_for_char_index<'a>(
    source: &str,
    blocks: &'a [markdown::MarkdownBlock],
    char_index: usize,
) -> &'a markdown::MarkdownBlock {
    let byte_index = source
        .char_indices()
        .nth(char_index)
        .map_or(source.len(), |(index, _)| index);
    blocks
        .iter()
        .find(|block| {
            (block.range.start..block.range.end).contains(&byte_index)
                || (block.range.is_empty() && block.range.start == byte_index)
        })
        // Inter-block whitespace belongs to the preceding block's editable
        // range. Keeping the cursor there prevents a queued blank-line click
        // from being clamped to the beginning of the following paragraph on
        // the next frame. An exact next-block start was handled above.
        .or_else(|| {
            blocks
                .iter()
                .rev()
                .find(|block| block.range.start < byte_index)
        })
        .or_else(|| blocks.iter().find(|block| block.range.start >= byte_index))
        .unwrap_or_else(|| blocks.last().expect("Markdown always has an editing block"))
}

fn cursor_range_to_char_range(range: CCursorRange) -> std::ops::Range<usize> {
    let [start, end] = range.sorted_cursors();
    start.index.0..end.index.0
}

fn cursor_range_saturating_sub(range: CCursorRange, offset: usize) -> CCursorRange {
    CCursorRange {
        primary: range.primary - offset,
        secondary: range.secondary - offset,
        h_pos: range.h_pos,
    }
}

fn cursor_range_add(range: CCursorRange, offset: usize) -> CCursorRange {
    CCursorRange {
        primary: range.primary + offset,
        secondary: range.secondary + offset,
        h_pos: range.h_pos,
    }
}

fn cursor_range_with_direction(
    sorted: std::ops::Range<usize>,
    direction: CCursorRange,
) -> CCursorRange {
    if direction.primary.index >= direction.secondary.index {
        CCursorRange::two(CCursor::new(sorted.start), CCursor::new(sorted.end))
    } else {
        CCursorRange::two(CCursor::new(sorted.end), CCursor::new(sorted.start))
    }
}

fn text_edit_cursor_after_input(output: &egui::text_edit::TextEditOutput) -> Option<CCursorRange> {
    if output.response.clicked() || output.response.dragged() {
        // Pointer interaction happens after `cursor_range` is captured. For
        // clicks and drag-selection the stored state is therefore newer, even
        // if another event also changed text in the same frame.
        output
            .state
            .cursor
            .range(&output.galley)
            .or(output.cursor_range)
    } else if output.response.changed() {
        // With hint text, egui 0.35 deliberately returns the pre-input empty
        // galley for the first keystroke. Resolving `state.cursor` against that
        // stale galley collapses the freshly advanced cursor back to zero, so
        // the next frame inserts before the Markdown marker. The public range
        // is the authoritative post-keyboard-edit cursor in this case.
        output
            .cursor_range
            .or_else(|| output.state.cursor.range(&output.galley))
    } else {
        output
            .state
            .cursor
            .range(&output.galley)
            .or(output.cursor_range)
    }
}

fn text_edit_cursor_at_position(
    output: &egui::text_edit::TextEditOutput,
    position: egui::Pos2,
) -> CCursor {
    output
        .galley
        .cursor_from_pos(position - output.galley_pos + egui::vec2(output.galley.rect.left(), 0.0))
}

fn source_selection_after_visual_input(
    projection: &VisualProjection,
    source: &str,
    previous_source_selection: Option<&std::ops::Range<usize>>,
    previous_visual_selection: Option<&std::ops::Range<usize>>,
    visual_selection: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    if previous_visual_selection == Some(&visual_selection)
        && let Some(previous) = previous_source_selection
    {
        return previous.clone();
    }
    projection.source_char_range(source, visual_selection)
}

fn line_break_before(source: &str, byte_index: usize) -> Option<std::ops::Range<usize>> {
    let before = source.get(..byte_index)?;
    if before.ends_with("\r\n") {
        Some(byte_index - 2..byte_index)
    } else if before.ends_with(['\n', '\r']) {
        Some(byte_index - 1..byte_index)
    } else {
        None
    }
}

fn boundary_backspace_edit(
    source: &str,
    edit_range: std::ops::Range<usize>,
) -> Option<(std::ops::Range<usize>, usize)> {
    let line_break = line_break_before(source, edit_range.start)?;
    let cursor = source[..line_break.start].chars().count();
    Some((line_break.start..edit_range.end, cursor))
}

fn code_block_removal_range(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    block_index: usize,
) -> std::ops::Range<usize> {
    let block = &blocks[block_index];
    if let Some(next) = blocks.get(block_index + 1) {
        if next.range.is_empty() {
            block.range.start..source.len()
        } else {
            block.range.start..next.range.start
        }
    } else if let Some(previous) = block_index
        .checked_sub(1)
        .and_then(|index| blocks.get(index))
    {
        previous.range.end..block.range.end
    } else {
        block.range.start..block.range.end.min(source.len())
    }
}

fn selected_markdown_for_clipboard(selected: &str) -> String {
    selected
        .trim_end_matches(['\r', '\n'])
        .trim_end()
        .to_owned()
}

fn paragraph_after_code_double_click(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    block_index: usize,
) -> (std::ops::Range<usize>, Option<String>, usize) {
    let block = &blocks[block_index];
    if let Some(next) = blocks.get(block_index + 1)
        && !next.range.is_empty()
    {
        let cursor = source[..next.range.start].chars().count();
        return (next.range.start..next.range.start, None, cursor);
    }

    let tail_range = block.range.start..source.len();
    let mut replacement = source[tail_range.clone()].to_owned();
    paragraph_after_fenced_code(&mut replacement)
        .expect("a complete fenced block must expose a trailing paragraph");
    let cursor = source[..tail_range.start].chars().count() + replacement.chars().count();
    let changed = (replacement != source[tail_range.clone()]).then_some(replacement);
    (tail_range, changed, cursor)
}

fn scroll_ratio(scroll: PaneScroll) -> f32 {
    if scroll.maximum <= f32::EPSILON {
        0.0
    } else {
        (scroll.offset / scroll.maximum).clamp(0.0, 1.0)
    }
}

fn is_fenced_code_block(source: &str) -> bool {
    let trimmed = source.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn hybrid_edit_range(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    active_id: BlockId,
) -> std::ops::Range<usize> {
    let index = blocks
        .iter()
        .position(|block| block.id == active_id)
        .expect("active Markdown block must still exist");
    let start = blocks[index].range.start;
    let end = blocks
        .get(index + 1)
        .map_or(source.len(), |block| block.range.start);
    start..end
}

fn snap_atomic_cross_block_selection(
    source: &str,
    blocks: &[markdown::MarkdownBlock],
    anchor_block: BlockId,
    anchor_char: usize,
    current_block: BlockId,
    current_char: usize,
) -> CCursorRange {
    let snap = |block_id: BlockId, cursor: usize, lower_endpoint: bool| {
        let Some(block) = blocks.iter().find(|block| block.id == block_id) else {
            return cursor;
        };
        let Some(block_source) = source.get(block.range.clone()) else {
            return cursor;
        };
        if !is_fenced_code_block(block_source) {
            return cursor;
        }
        let boundary = if lower_endpoint {
            block.range.start
        } else {
            block.range.end
        };
        source[..boundary].chars().count()
    };

    let forward = anchor_char <= current_char;
    let snapped_anchor = snap(anchor_block, anchor_char, forward);
    let snapped_current = snap(current_block, current_char, !forward);
    CCursorRange {
        primary: CCursor::new(snapped_current),
        secondary: CCursor::new(snapped_anchor),
        h_pos: None,
    }
}

fn multiline_edit_rows(source: &str) -> usize {
    source.bytes().filter(|byte| *byte == b'\n').count() + 1
}

fn extra_inter_block_blank_lines(source: &str, gap: std::ops::Range<usize>) -> usize {
    line_break_count(source, gap).saturating_sub(2)
}

fn inter_block_blank_line_cursor_byte(
    source: &str,
    gap: std::ops::Range<usize>,
    blank_line: usize,
) -> Option<usize> {
    // Two line breaks are the normal Markdown paragraph separator. Each
    // additional visible blank row starts after the second, third, … break.
    nth_line_break_end(source, gap, blank_line.saturating_add(1))
}

fn line_break_count(source: &str, range: std::ops::Range<usize>) -> usize {
    let Some(fragment) = source.get(range) else {
        return 0;
    };
    let bytes = fragment.as_bytes();
    let mut cursor = 0usize;
    let mut count = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\r' => {
                cursor += usize::from(bytes.get(cursor + 1) == Some(&b'\n')) + 1;
                count += 1;
            }
            b'\n' => {
                cursor += 1;
                count += 1;
            }
            _ => cursor += 1,
        }
    }
    count
}

fn nth_line_break_end(source: &str, range: std::ops::Range<usize>, target: usize) -> Option<usize> {
    let fragment = source.get(range.clone())?;
    let bytes = fragment.as_bytes();
    let mut cursor = 0usize;
    let mut index = 0usize;
    while cursor < bytes.len() {
        let length = match bytes[cursor] {
            b'\r' if bytes.get(cursor + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                cursor += 1;
                continue;
            }
        };
        cursor += length;
        if index == target {
            return Some(range.start + cursor);
        }
        index += 1;
    }
    None
}

fn wysiwyg_layout(
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
            FontId::new(14.0, FontFamily::Proportional),
            palette.text,
            wrap_width,
        );
        job.keep_trailing_whitespace = true;
        return ui.fonts_mut(|fonts| fonts.layout_job(job));
    }

    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.keep_trailing_whitespace = true;
    let mut previous_was_inline_code = false;
    for run in runs {
        let start = char_to_byte(text, run.range.start);
        let end = char_to_byte(text, run.range.end);
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

fn rounded_inline_code_backgrounds(
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
struct TextProjectionPreview {
    galley: Arc<egui::Galley>,
    galley_pos: egui::Pos2,
    projection: VisualProjection,
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
enum HybridPointerMapping {
    Exact(TextProjectionPreview),
    Approximate,
}

#[derive(Clone)]
struct HybridPointerRegion {
    block_id: BlockId,
    source_range: std::ops::Range<usize>,
    rect: egui::Rect,
    atomic_range: Option<std::ops::Range<usize>>,
    mapping: HybridPointerMapping,
}

impl HybridPointerRegion {
    fn source_char_at_position(&self, source: &str, position: egui::Pos2) -> Option<usize> {
        if !self.rect.expand(3.0).contains(position) {
            return None;
        }
        let block_source = source.get(self.source_range.clone())?;
        let local_byte = match &self.mapping {
            HybridPointerMapping::Exact(preview) => {
                preview.source_byte_at_position(block_source, position)
            }
            HybridPointerMapping::Approximate => {
                let width = self.rect.width().max(1.0);
                let height = self.rect.height().max(1.0);
                SourceMap::from_markdown(block_source).source_byte_at_normalized_point(
                    (position.x - self.rect.left()) / width,
                    (position.y - self.rect.top()) / height,
                )
            }
        };
        let byte = (self.source_range.start + local_byte).min(source.len());
        Some(source[..byte].chars().count())
    }
}

fn hybrid_pointer_hit(
    source: &str,
    regions: &[HybridPointerRegion],
    position: egui::Pos2,
) -> Option<(BlockId, usize)> {
    regions.iter().find_map(|region| {
        region
            .source_char_at_position(source, position)
            .map(|cursor| (region.block_id, cursor))
    })
}

fn paint_hybrid_cross_selection(
    ui: &Ui,
    source: &str,
    regions: &[HybridPointerRegion],
    cursor: CCursorRange,
) {
    let selection = cursor_range_to_char_range(cursor);
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
        match &region.mapping {
            HybridPointerMapping::Exact(preview) => {
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
            HybridPointerMapping::Approximate => {
                let length = block_source.chars().count().max(1);
                let left = region.rect.left()
                    + region.rect.width() * local_source.start as f32 / length as f32;
                let right = region.rect.left()
                    + region.rect.width() * local_source.end as f32 / length as f32;
                let rect = egui::Rect::from_min_max(
                    egui::pos2(left.min(right), region.rect.top()),
                    egui::pos2(left.max(right), region.rect.bottom()),
                );
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    ui.visuals().selection.bg_fill.gamma_multiply(0.45),
                );
            }
        }
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
    (start < end).then_some(start - region_start..end - region_start)
}

fn show_code_copy_button(
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
    let response = ui.put(
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

fn show_text_projection_preview(
    ui: &mut Ui,
    source: &str,
    palette: AppPalette,
) -> Option<TextProjectionPreview> {
    if !supports_text_projection_preview(source) {
        return None;
    }
    let projection = VisualProjection::from_markdown(source);
    let runs = projection.runs_for(projection.text());
    let galley = wysiwyg_layout(
        ui,
        projection.text(),
        &projection,
        ui.available_width(),
        palette,
        true,
    );
    let (galley_pos, galley, _) = egui::Label::new(galley).selectable(false).layout_in_ui(ui);
    ui.painter().extend(rounded_inline_code_backgrounds(
        &galley, galley_pos, &runs, palette,
    ));
    ui.painter()
        .galley(galley_pos, Arc::clone(&galley), palette.text);
    Some(TextProjectionPreview {
        galley,
        galley_pos,
        projection,
    })
}

fn preserve_soft_breaks_for_wysiwyg(source: &str) -> std::borrow::Cow<'_, str> {
    if !source.contains(['\n', '\r']) {
        return std::borrow::Cow::Borrowed(source);
    }

    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (event, range) in Parser::new_ext(source, markdown::parser_options()).into_offset_iter() {
        if !matches!(event, MarkdownEvent::SoftBreak) {
            continue;
        }
        output.push_str(&source[cursor..range.start]);
        let line_break = &source[range.clone()];
        let before = &source[..range.start];
        if before.ends_with("  ") || before.ends_with('\\') {
            output.push_str(line_break);
        } else {
            output.push_str("  ");
            output.push_str(line_break);
        }
        cursor = range.end;
    }
    if cursor == 0 {
        std::borrow::Cow::Borrowed(source)
    } else {
        output.push_str(&source[cursor..]);
        std::borrow::Cow::Owned(output)
    }
}

fn supports_text_projection_preview(source: &str) -> bool {
    // CommonMark deliberately folds soft line breaks to spaces. That is
    // correct for a read-only preview, but surprising in a WYSIWYG editing
    // canvas where the source line structure must remain visible.
    let mut needs_projection = contains_footnote_reference(source) || source.contains(['\n', '\r']);
    for event in Parser::new_ext(source, markdown::parser_options()) {
        match event {
            MarkdownEvent::Code(_) | MarkdownEvent::FootnoteReference(_) => {
                needs_projection = true;
            }
            MarkdownEvent::Start(
                Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough,
            )
            | MarkdownEvent::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough,
            )
            | MarkdownEvent::SoftBreak
            | MarkdownEvent::HardBreak => {}
            MarkdownEvent::Text(_) => {}
            _ => return false,
        }
    }
    needs_projection
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

fn set_wysiwyg_document_accessibility(ui: &Ui, title: &str) {
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
        node.set_role(egui::accesskit::Role::Document);
        node.set_label(format!("{title} 所见即所得文档"));
    });
}

fn accessible_document_remainder(
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

fn append_accessible_text_runs(
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

fn set_markdown_preview_accessibility(response: &egui::Response, source: &str) {
    let accessible_text = accessible_markdown_block_text(source);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &accessible_text));
}

fn paint_inline_code_delimiters(
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

fn visual_text_format(style: VisualStyle, palette: AppPalette) -> TextFormat {
    let size = if style.code && style.marker {
        17.0
    } else if style.footnote {
        11.0
    } else {
        match (style.code, style.heading) {
            (_, 1) => 34.0,
            (_, 2) => 27.0,
            (_, 3) => 22.0,
            (_, 4) => 18.0,
            (true, _) => 15.0,
            _ => 16.0,
        }
    };
    let family = if style.code {
        FontFamily::Monospace
    } else if style.strong || style.heading > 0 {
        FontFamily::Name(WYSIWYG_STRONG_FAMILY.into())
    } else {
        FontFamily::Proportional
    };
    let mut format = TextFormat::simple(
        FontId::new(size, family),
        if style.code && style.marker {
            palette.accent
        } else if style.marker || style.quote {
            palette.secondary
        } else if style.link {
            palette.accent
        } else {
            palette.text
        },
    );
    format.line_height = Some(match (style.code, style.heading) {
        (_, 1) => 44.0,
        (_, 2) => 36.0,
        (_, 3) => 31.0,
        (_, 4) => WYSIWYG_BODY_LINE_HEIGHT,
        (true, _) => WYSIWYG_INLINE_CODE_LINE_HEIGHT,
        _ => WYSIWYG_BODY_LINE_HEIGHT,
    });
    if style.code {
        format.valign = Align::Center;
    } else if style.footnote {
        format.valign = Align::Min;
    }
    if style.strong || style.heading > 0 {
        format.extra_letter_spacing = 0.2;
    }
    format.italics = style.emphasis;
    if style.strikethrough {
        format.strikethrough = Stroke::new(1.0, format.color);
    }
    if style.link {
        format.underline = Stroke::new(1.0, palette.accent);
    }
    if style.code {
        format.extra_letter_spacing = 0.1;
    }
    format
}

fn parse_shortcut(specification: &str) -> Option<egui::KeyboardShortcut> {
    let mut modifiers = egui::Modifiers::default();
    let mut key = None;
    for part in specification
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part.to_ascii_uppercase().as_str() {
            "CTRL" | "CMD" | "COMMAND" => modifiers.command = true,
            "SHIFT" => modifiers.shift = true,
            "ALT" | "OPTION" => modifiers.alt = true,
            token => {
                key = Some(match token {
                    "A" => Key::A,
                    "B" => Key::B,
                    "C" => Key::C,
                    "D" => Key::D,
                    "E" => Key::E,
                    "F" => Key::F,
                    "G" => Key::G,
                    "H" => Key::H,
                    "I" => Key::I,
                    "J" => Key::J,
                    "K" => Key::K,
                    "L" => Key::L,
                    "M" => Key::M,
                    "N" => Key::N,
                    "O" => Key::O,
                    "P" => Key::P,
                    "Q" => Key::Q,
                    "R" => Key::R,
                    "S" => Key::S,
                    "T" => Key::T,
                    "U" => Key::U,
                    "V" => Key::V,
                    "W" => Key::W,
                    "X" => Key::X,
                    "Y" => Key::Y,
                    "Z" => Key::Z,
                    "F1" => Key::F1,
                    "F2" => Key::F2,
                    "F3" => Key::F3,
                    "F4" => Key::F4,
                    "F5" => Key::F5,
                    "F6" => Key::F6,
                    "F7" => Key::F7,
                    "F8" => Key::F8,
                    "F9" => Key::F9,
                    "F10" => Key::F10,
                    "F11" => Key::F11,
                    "F12" => Key::F12,
                    _ => return None,
                });
            }
        }
    }
    Some(egui::KeyboardShortcut {
        modifiers,
        logical_key: key?,
    })
}

fn consume_shortcut(input: &mut egui::InputState, specification: &str) -> bool {
    parse_shortcut(specification).is_some_and(|shortcut| input.consume_shortcut(&shortcut))
}

fn shortcut_row(ui: &mut Ui, label: &str, shortcut: &mut String) {
    ui.label(label);
    let response = ui.add_sized([170.0, 22.0], TextEdit::singleline(shortcut));
    if parse_shortcut(shortcut).is_none() {
        response.on_hover_text("快捷键格式无效，例如 Ctrl+Shift+P");
        ui.colored_label(ui.visuals().error_fg_color, "格式无效");
    }
    ui.end_row();
}

fn duplicate_shortcuts(bindings: &KeyBindings) -> bool {
    let shortcuts = [
        &bindings.new_document,
        &bindings.open_file,
        &bindings.open_folder,
        &bindings.save,
        &bindings.save_as,
        &bindings.undo,
        &bindings.redo,
        &bindings.find,
        &bindings.replace,
        &bindings.command_palette,
        &bindings.bold,
        &bindings.italic,
        &bindings.link,
    ];
    let mut unique = HashSet::new();
    shortcuts
        .into_iter()
        .map(|shortcut| shortcut.trim().to_ascii_uppercase())
        .any(|shortcut| !shortcut.is_empty() && !unique.insert(shortcut))
}

#[derive(Default)]
struct EditorInputAction {
    backspace: bool,
    delete: bool,
    down: bool,
    enter: bool,
    horizontal_modified: bool,
    left: bool,
    tab: bool,
    right: bool,
    shift: bool,
    pasted_url: Option<String>,
    typed_text: Option<String>,
}

enum CrossBlockInput {
    Copy,
    Cut,
    Delete,
    Replace(String),
}

impl CrossBlockInput {
    fn copy(&self) -> bool {
        matches!(self, Self::Copy | Self::Cut)
    }

    fn replacement(&self) -> Option<&str> {
        match self {
            Self::Copy => None,
            Self::Cut | Self::Delete => Some(""),
            Self::Replace(text) => Some(text),
        }
    }
}

fn take_cross_block_input(ui: &mut Ui) -> Option<CrossBlockInput> {
    let mut action = None;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            let next = match event {
                egui::Event::Copy => Some(CrossBlockInput::Copy),
                egui::Event::Cut => Some(CrossBlockInput::Cut),
                egui::Event::Paste(text) | egui::Event::Text(text) => {
                    Some(CrossBlockInput::Replace(text.clone()))
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty() => {
                    Some(CrossBlockInput::Replace(text.clone()))
                }
                egui::Event::Key {
                    key: Key::Backspace | Key::Delete,
                    pressed: true,
                    ..
                } => Some(CrossBlockInput::Delete),
                _ => None,
            };
            if let Some(next) = next {
                action = Some(next);
                false
            } else {
                true
            }
        });
    });
    action
}

fn editor_input_action(ui: &Ui) -> EditorInputAction {
    ui.input(|input| EditorInputAction {
        backspace: input.key_pressed(Key::Backspace),
        delete: input.key_pressed(Key::Delete),
        down: input.key_pressed(Key::ArrowDown),
        enter: input.key_pressed(Key::Enter),
        horizontal_modified: input.modifiers.alt
            || input.modifiers.ctrl
            || input.modifiers.mac_cmd
            || input.modifiers.shift,
        left: input.key_pressed(Key::ArrowLeft),
        tab: input.key_pressed(Key::Tab),
        right: input.key_pressed(Key::ArrowRight),
        shift: input.modifiers.shift,
        pasted_url: input.events.iter().rev().find_map(|event| match event {
            egui::Event::Paste(text) => Some(text.trim().to_owned()),
            _ => None,
        }),
        typed_text: input.events.iter().rev().find_map(|event| match event {
            egui::Event::Text(text) => Some(text.clone()),
            _ => None,
        }),
    })
}

fn ime_frame_action(events: &[egui::Event]) -> ImeFrameAction {
    events
        .iter()
        .fold(ImeFrameAction::None, |action, event| match event {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                if text.is_empty() {
                    ImeFrameAction::Cancel
                } else {
                    ImeFrameAction::Preedit
                }
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                if text.is_empty() {
                    ImeFrameAction::Cancel
                } else {
                    ImeFrameAction::Commit
                }
            }
            _ => action,
        })
}

fn workspace_entries_ui(
    ui: &mut Ui,
    entries: &[WorkspaceEntry],
    active_path: Option<&Path>,
) -> Option<PathBuf> {
    let mut selected = None;
    for entry in entries {
        if entry.is_dir {
            let response = egui::CollapsingHeader::new(&entry.name)
                .id_salt(&entry.path)
                .default_open(false)
                .show(ui, |ui| {
                    workspace_entries_ui(ui, &entry.children, active_path)
                });
            if let Some(path) = response.body_returned.flatten() {
                selected = Some(path);
            }
        } else if ui
            .selectable_label(active_path == Some(entry.path.as_path()), &entry.name)
            .on_hover_text(entry.path.display().to_string())
            .clicked()
        {
            selected = Some(entry.path.clone());
        }
    }
    selected
}

fn file_uri_base(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    if cfg!(target_os = "windows") {
        if normalized.starts_with("//") {
            format!("file://{}/", normalized.trim_start_matches('/'))
        } else {
            format!("file:///{normalized}/")
        }
    } else {
        format!("file://{normalized}/")
    }
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "txt"
            )
        })
        .unwrap_or(false)
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico"
            )
        })
}

fn markdown_resource_destination(path: &Path, base: &Path) -> String {
    let absolute_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let absolute_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let destination = pathdiff::diff_paths(&absolute_path, &absolute_base)
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(absolute_path);
    destination
        .to_string_lossy()
        .replace('\\', "/")
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23")
        .replace('?', "%3F")
        .replace('(', "%28")
        .replace(')', "%29")
}

fn path_is_within(path: &Path, allowed_root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let allowed_root = allowed_root
        .canonicalize()
        .unwrap_or_else(|_| allowed_root.to_path_buf());
    path.starts_with(allowed_root)
}

fn prompt_to_save(title: &str) -> MessageDialogResult {
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title("关闭文档")
        .set_description(format!(
            "“{title}”尚未保存。\n\n是：保存后关闭\n否：放弃修改\n取消：继续编辑"
        ))
        .set_buttons(MessageButtons::YesNoCancel)
        .show()
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

fn clamp_char_range(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let length = text.chars().count();
    range.start.min(length)..range.end.min(length).max(range.start.min(length))
}

fn next_footnote_number(source: &str) -> usize {
    let mut used = HashSet::new();
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index + 3 < bytes.len() {
        if bytes[index] == b'[' && bytes[index + 1] == b'^' {
            let digits_start = index + 2;
            let mut end = digits_start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > digits_start
                && bytes.get(end) == Some(&b']')
                && let Ok(number) = source[digits_start..end].parse::<usize>()
            {
                used.insert(number);
            }
            index = end;
        }
        index += 1;
    }
    (1..).find(|number| !used.contains(number)).unwrap_or(1)
}

fn apply_theme(ctx: &Context, dark: bool) {
    let palette = app_palette(dark);
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.secondary);
    visuals.panel_fill = palette.canvas;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.faint_bg_color = palette.hover;
    visuals.extreme_bg_color = palette.surface;
    visuals.text_edit_bg_color = Some(palette.surface);
    visuals.code_bg_color = palette.code_bg;
    visuals.hyperlink_color = palette.accent;
    visuals.selection.bg_fill = palette.accent_soft;
    visuals.selection.stroke = Stroke::new(1.5, palette.accent);
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = false;

    visuals.widgets.noninteractive.bg_fill = palette.surface;
    visuals.widgets.noninteractive.weak_bg_fill = palette.surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.inactive.bg_fill = palette.surface;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);

    visuals.widgets.hovered.bg_fill = palette.hover;
    visuals.widgets.hovered.weak_bg_fill = palette.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = palette.accent_soft;
    visuals.widgets.active.weak_bg_fill = palette.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.expansion = 0.0;
    visuals.widgets.open = visuals.widgets.active;

    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.global_style_mut(|style| {
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::new(7.0, 5.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.interact_size = Vec2::new(32.0, 29.0);
        style.spacing.window_margin = Margin::same(12);
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(32.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(16.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(15.0, FontFamily::Monospace),
        );
        for (name, size) in [
            ("rupora-title", 30.0),
            ("rupora-h2", 24.0),
            ("rupora-h3", 20.0),
            ("rupora-h4", 17.0),
        ] {
            style.text_styles.insert(
                TextStyle::Name(name.into()),
                FontId::new(size, FontFamily::Proportional),
            );
        }
    });
}

fn install_fonts(ctx: &Context) {
    let regular_font = export::cjk_font_candidates()
        .into_iter()
        .find_map(|path| fs::read(&path).ok().map(|bytes| (path, bytes)));
    let mut fonts = FontDefinitions::default();
    if let Some((path, bytes)) = regular_font {
        let font_name = format!("rupora-cjk-{}", path.display());
        fonts
            .font_data
            .insert(font_name.clone(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, font_name.clone());
        let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
        monospace.insert(monospace.len().min(1), font_name);
    }
    let mut strong_fonts = Vec::new();
    if let Some((bold_path, bold_bytes)) = cjk_bold_font_candidates()
        .into_iter()
        .find_map(|path| fs::read(&path).ok().map(|bytes| (path, bytes)))
    {
        let bold_name = format!("rupora-cjk-bold-{}", bold_path.display());
        fonts.font_data.insert(
            bold_name.clone(),
            Arc::new(FontData::from_owned(bold_bytes)),
        );
        strong_fonts.push(bold_name);
    }
    strong_fonts.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(FontFamily::Name(WYSIWYG_STRONG_FAMILY.into()), strong_fonts);
    ctx.set_fonts(fonts);
}

fn cjk_bold_font_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        vec![
            PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttf"),
            PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
            PathBuf::from("/System/Library/Fonts/STHeiti Medium.ttc"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc"),
            PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_preview::{
        MAX_GENERATED_SVG_CACHE_BYTES, MAX_GENERATED_SVG_CACHE_ENTRIES, cache_generated_svg,
    };

    #[test]
    fn accepts_supported_document_extensions_case_insensitively() {
        assert!(is_markdown_path(Path::new("README.MD")));
        assert!(is_markdown_path(Path::new("notes.markdown")));
        assert!(!is_markdown_path(Path::new("image.png")));
    }

    #[test]
    fn extension_results_follow_document_identity_after_tabs_shift() {
        let first = Document::untitled(1);
        let target = Document::untitled(2);
        let target_id = target.id();
        let trailing = Document::untitled(3);
        let mut documents = vec![first, target, trailing];

        documents.remove(0);
        assert_eq!(extension_document_index(&documents, target_id), Some(0));

        documents.remove(0);
        assert_eq!(extension_document_index(&documents, target_id), None);
    }

    #[test]
    fn allocates_the_first_unused_numeric_footnote() {
        assert_eq!(next_footnote_number("plain"), 1);
        assert_eq!(next_footnote_number("[^1] and [^3]"), 2);
        assert_eq!(next_footnote_number("[^2]: definition"), 1);
    }

    #[test]
    fn replaces_mermaid_fences_with_registered_native_svg_images() {
        let context = Context::default();
        let mut cache = HashMap::new();
        let preview = prepare_native_preview(
            &context,
            "```mermaid\nflowchart LR\nA --> B\n```\n",
            Path::new("."),
            false,
            &mut cache,
        );
        assert!(preview.contains("bytes://rupora/mermaid-"));
        assert!(!preview.contains("```mermaid"));
        assert_eq!(cache.len(), 1);
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
        assert!(cache.contains_key("key-0"));
        assert!(!cache.contains_key(&format!("key-{MAX_GENERATED_SVG_CACHE_ENTRIES}")));
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

        assert!(!cache_generated_svg(
            &context,
            &mut cache,
            "replacement".to_owned(),
            Arc::from([1]),
        ));
        assert_eq!(cache.len(), 4);
        assert!(!cache.contains_key("replacement"));
    }

    #[test]
    fn creates_an_absolute_file_uri_base() {
        let uri = file_uri_base(Path::new(if cfg!(target_os = "windows") {
            r"C:\notes and docs"
        } else {
            "/tmp/notes and docs"
        }));
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("notes and docs/"));
    }

    #[test]
    fn creates_relative_encoded_resource_destinations() {
        let directory = tempfile::tempdir().unwrap();
        let notes = directory.path().join("notes");
        let assets = directory.path().join("assets");
        fs::create_dir_all(&notes).unwrap();
        fs::create_dir_all(&assets).unwrap();
        let image = assets.join("diagram one.png");
        fs::write(&image, b"image").unwrap();

        assert_eq!(
            markdown_resource_destination(&image, &notes),
            "../assets/diagram%20one.png"
        );
        assert!(is_image_path(&image));
        assert!(!is_image_path(Path::new("attachment.pdf")));
    }

    #[test]
    fn parses_configurable_cross_platform_shortcuts() {
        let shortcut = parse_shortcut("Ctrl+Shift+P").unwrap();
        assert!(shortcut.modifiers.command);
        assert!(shortcut.modifiers.shift);
        assert_eq!(shortcut.logical_key, Key::P);
        assert!(parse_shortcut("Ctrl+NoSuchKey").is_none());
    }

    #[test]
    fn detects_duplicate_shortcuts() {
        let mut bindings = KeyBindings::default();
        assert!(!duplicate_shortcuts(&bindings));
        bindings.link.clone_from(&bindings.bold);
        assert!(duplicate_shortcuts(&bindings));
    }

    #[test]
    fn calculates_safe_split_scroll_ratios() {
        assert_eq!(
            scroll_ratio(PaneScroll {
                offset: 50.0,
                maximum: 100.0,
                hovered: true,
            }),
            0.5
        );
        assert_eq!(scroll_ratio(PaneScroll::default()), 0.0);
    }

    #[test]
    fn local_path_policy_blocks_workspace_escape() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        let outside = directory.path().join("outside.txt");
        fs::create_dir(&workspace).unwrap();
        fs::write(&outside, "secret").unwrap();
        let inside = workspace.join("note.md");
        fs::write(&inside, "safe").unwrap();

        assert!(path_is_within(&inside, &workspace));
        assert!(!path_is_within(&outside, &workspace));
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
    fn text_projection_preview_only_replaces_supported_text_blocks() {
        for source in [
            "before `code` after",
            "# heading with `code`",
            "**bold `code`** and *emphasis*",
            "脚注引用[^1]",
            "soft\nline",
            "hard  \nline",
        ] {
            assert!(supports_text_projection_preview(source), "{source}");
        }

        for source in [
            "plain text",
            "- list with `code`",
            "[link](note.md) and `code`",
            "![image](image.png) and `code`",
            "```rust\ncode\n```",
        ] {
            assert!(!supports_text_projection_preview(source), "{source}");
        }
    }

    #[test]
    fn wysiwyg_static_preview_preserves_soft_breaks_without_mutating_markdown() {
        assert_eq!(
            preserve_soft_breaks_for_wysiwyg("[链接](note.md) 第一行\n第二行"),
            "[链接](note.md) 第一行  \n第二行"
        );
        assert_eq!(
            preserve_soft_breaks_for_wysiwyg("- 列表第一行\n  延续行"),
            "- 列表第一行  \n  延续行"
        );
        assert_eq!(
            preserve_soft_breaks_for_wysiwyg("已经硬换行  \n下一行"),
            "已经硬换行  \n下一行"
        );
        assert_eq!(
            preserve_soft_breaks_for_wysiwyg("```\na\nb\n```"),
            "```\na\nb\n```"
        );
    }

    #[test]
    fn cross_block_selection_treats_fenced_code_as_an_atomic_block() {
        let source = "before α\n\n```rust\n代码();\n```\n\nafter β";
        let blocks = markdown::blocks(source);
        assert_eq!(blocks.len(), 3);
        let before = &blocks[0];
        let code = &blocks[1];
        let after = &blocks[2];
        let char_at = |byte| source[..byte].chars().count();

        let forward = snap_atomic_cross_block_selection(
            source,
            &blocks,
            before.id,
            char_at(before.range.end) - 2,
            code.id,
            char_at(code.range.start) + 4,
        );
        assert_eq!(
            forward.primary.index.0,
            char_at(code.range.end),
            "the whole code block must be selected at the upper edge"
        );

        let backward = snap_atomic_cross_block_selection(
            source,
            &blocks,
            code.id,
            char_at(code.range.start) + 5,
            before.id,
            char_at(before.range.end) - 2,
        );
        assert_eq!(
            backward.secondary.index.0,
            char_at(code.range.end),
            "a later code anchor must snap to its far edge"
        );

        let from_code = snap_atomic_cross_block_selection(
            source,
            &blocks,
            code.id,
            char_at(code.range.start) + 5,
            after.id,
            char_at(after.range.start) + 2,
        );
        assert_eq!(from_code.secondary.index.0, char_at(code.range.start));
        let selected = &source[code.range.clone()];
        assert_eq!(
            selected_markdown_for_clipboard(selected),
            "```rust\n代码();\n```"
        );
    }

    #[test]
    fn removing_a_code_block_also_removes_one_structural_gap() {
        for (source, index, expected) in [
            ("```\na\n```\n\nafter", 0, "after"),
            ("before\n\n```\na\n```", 1, "before"),
            ("```\na\n```", 0, ""),
            ("```\n\n```\n\n", 0, ""),
        ] {
            let blocks = markdown::blocks(source);
            let range = code_block_removal_range(source, &blocks, index);
            let mut updated = source.to_owned();
            updated.replace_range(range, "");
            assert_eq!(updated, expected);
        }
    }

    #[test]
    fn double_click_after_code_targets_a_real_plain_paragraph() {
        let source = "```\ncode\n```";
        let blocks = markdown::blocks(source);
        let (range, replacement, cursor) = paragraph_after_code_double_click(source, &blocks, 0);
        assert_eq!(range, 0..source.len());
        assert_eq!(replacement.as_deref(), Some("```\ncode\n```\n\n"));
        assert_eq!(cursor, "```\ncode\n```\n\n".chars().count());

        let source = "```\ncode\n```\n\nnext";
        let blocks = markdown::blocks(source);
        let (range, replacement, cursor) = paragraph_after_code_double_click(source, &blocks, 0);
        assert_eq!(range.start, range.end);
        assert!(replacement.is_none());
        assert_eq!(cursor, source.find("next").unwrap());
    }

    #[test]
    fn inline_code_preview_click_maps_to_the_actual_glyph() {
        use egui::RawInput;

        let context = Context::default();
        let source = "PRE `ab` POST";
        let mut source_byte = None;
        let _ = context.run_ui(RawInput::default(), |ui| {
            ui.set_width(720.0);
            let preview = show_text_projection_preview(ui, source, app_palette(false))
                .expect("inline code should use the precise projection preview");
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
    fn wysiwyg_editor_keeps_trailing_newlines_inside_the_active_range() {
        let source = "第一段\n\n第二段";
        let blocks = markdown::blocks(source);
        let first_range = hybrid_edit_range(source, &blocks, blocks[0].id);
        assert_eq!(&source[first_range], "第一段\n\n");

        let mut trailing = "换句话".to_owned();
        let original_blocks = markdown::blocks(&trailing);
        let original_range = hybrid_edit_range(&trailing, &original_blocks, original_blocks[0].id);
        let replacement = format!("{}\n", &trailing[original_range.clone()]);
        trailing.replace_range(original_range, &replacement);

        let updated_blocks = markdown::blocks(&trailing);
        let updated_range = hybrid_edit_range(&trailing, &updated_blocks, updated_blocks[0].id);
        assert_eq!(&trailing[updated_range], "换句话\n");
        assert_eq!(multiline_edit_rows(&trailing), 2);
    }

    #[test]
    fn hidden_inline_code_boundaries_preserve_the_outside_cursor_side() {
        let source = "`abc`";
        let projection = VisualProjection::from_markdown(source);
        let previous_source = 5..5;
        let previous_visual = projection.visual_char_range(source, previous_source.clone());
        assert_eq!(previous_visual, 3..3);

        assert_eq!(
            source_selection_after_visual_input(
                &projection,
                source,
                Some(&previous_source),
                Some(&previous_visual),
                previous_visual.clone(),
            ),
            previous_source
        );
        assert_eq!(
            source_selection_after_visual_input(
                &projection,
                source,
                Some(&(5..5)),
                Some(&(3..3)),
                2..2,
            ),
            3..3
        );
    }

    #[test]
    fn wysiwyg_reads_click_and_drag_selection_from_the_post_pointer_state() {
        use egui::{Event, Id, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

        fn run_frame(
            context: &Context,
            id: Id,
            text: &mut String,
            events: Vec<Event>,
            request_focus: bool,
        ) -> (Arc<egui::Galley>, egui::Pos2, Option<CCursorRange>) {
            let mut result = None;
            let _ = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 160.0))),
                    events,
                    ..RawInput::default()
                },
                |ui| {
                    ui.set_width(480.0);
                    if request_focus {
                        ui.memory_mut(|memory| memory.request_focus(id));
                    }
                    let output = TextEdit::singleline(text).id(id).show(ui);
                    result = Some((
                        output.galley.clone(),
                        output.galley_pos,
                        text_edit_cursor_after_input(&output),
                    ));
                },
            );
            result.expect("text edit should be laid out")
        }

        fn cursor_position(
            galley: &egui::Galley,
            galley_pos: egui::Pos2,
            index: usize,
        ) -> egui::Pos2 {
            galley_pos
                + galley
                    .pos_from_cursor(CCursor::new(index))
                    .center()
                    .to_vec2()
        }

        let context = Context::default();
        let id = Id::new("wysiwyg-pointer-selection-regression");
        let mut text = "alpha beta 中文".to_owned();
        let (galley, galley_pos, _) = run_frame(&context, id, &mut text, Vec::new(), true);
        let click = cursor_position(&galley, galley_pos, 2);
        let (_, _, clicked) = run_frame(
            &context,
            id,
            &mut text,
            vec![
                Event::PointerMoved(click),
                Event::PointerButton {
                    pos: click,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            false,
        );
        assert_eq!(clicked.map(cursor_range_to_char_range), Some(2..2));

        let (galley, galley_pos, _) = run_frame(&context, id, &mut text, Vec::new(), false);
        let drag_to = cursor_position(&galley, galley_pos, 10);
        let (_, _, selected) = run_frame(
            &context,
            id,
            &mut text,
            vec![Event::PointerMoved(drag_to)],
            false,
        );
        let selected = selected.expect("dragging should create a selection");
        assert_eq!(cursor_range_to_char_range(selected), 2..10);
        assert_eq!(selected.primary.index.0, 10);
        assert_eq!(selected.secondary.index.0, 2);

        let (galley, galley_pos, _) = run_frame(
            &context,
            id,
            &mut text,
            vec![Event::PointerButton {
                pos: drag_to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            false,
        );

        let reverse_start = cursor_position(&galley, galley_pos, 12);
        let (_, _, reverse_anchor) = run_frame(
            &context,
            id,
            &mut text,
            vec![
                Event::PointerMoved(reverse_start),
                Event::PointerButton {
                    pos: reverse_start,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            false,
        );
        assert_eq!(reverse_anchor.map(cursor_range_to_char_range), Some(12..12));

        let (galley, galley_pos, _) = run_frame(&context, id, &mut text, Vec::new(), false);
        let reverse_to = cursor_position(&galley, galley_pos, 6);
        let (_, _, reverse_selected) = run_frame(
            &context,
            id,
            &mut text,
            vec![Event::PointerMoved(reverse_to)],
            false,
        );
        let reverse_selected = reverse_selected.expect("reverse dragging should select text");
        assert_eq!(cursor_range_to_char_range(reverse_selected), 6..12);
        assert_eq!(reverse_selected.primary.index.0, 6);
        assert_eq!(reverse_selected.secondary.index.0, 12);

        let reverse = cursor_range_with_direction(
            2..10,
            CCursorRange::two(CCursor::new(10), CCursor::new(2)),
        );
        assert_eq!(reverse.primary.index.0, 2);
        assert_eq!(reverse.secondary.index.0, 10);
    }

    #[test]
    fn wysiwyg_text_edit_keeps_source_cursor_when_markers_become_hidden() {
        use egui::{Event, Id, Modifiers, RawInput};

        fn edit_frame(
            context: &Context,
            id: Id,
            source: &str,
            source_selection: std::ops::Range<usize>,
            events: Vec<Event>,
        ) -> crate::wysiwyg::VisualSourceEdit {
            let projection = VisualProjection::from_markdown_with_selection(
                source,
                Some(source_selection.clone()),
            );
            let visual_selection = projection.visual_char_range(source, source_selection);
            let mut visual_content = projection.text().to_owned();
            let mut result = None;
            let _ = context.run_ui(
                RawInput {
                    events,
                    ..RawInput::default()
                },
                |ui| {
                    let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(
                            visual_selection.start,
                        ))));
                    state.store(ui.ctx(), id);
                    ui.memory_mut(|memory| memory.request_focus(id));
                    let output = TextEdit::multiline(&mut visual_content)
                        .id(id)
                        .hint_text("开始写作…")
                        .show(ui);
                    let visual_cursor = text_edit_cursor_after_input(&output)
                        .map(cursor_range_to_char_range)
                        .expect("focused TextEdit should retain a cursor");
                    result = projection.apply_edit(source, &visual_content, visual_cursor);
                },
            );
            result.expect("typed frame should update the source")
        }

        fn type_frame(
            context: &Context,
            id: Id,
            source: &str,
            source_selection: std::ops::Range<usize>,
            text: &str,
        ) -> crate::wysiwyg::VisualSourceEdit {
            edit_frame(
                context,
                id,
                source,
                source_selection,
                vec![Event::Text(text.to_owned())],
            )
        }

        let context = Context::default();
        let id = Id::new("wysiwyg-hidden-marker-cursor-regression");
        let mut update = type_frame(&context, id, "", 0..0, "#");
        assert_eq!(update.source, "#");
        update = type_frame(&context, id, &update.source, update.selection, " ");
        assert_eq!(update.source, "# ");
        update = type_frame(&context, id, &update.source, update.selection, "ATX 标题");
        assert_eq!(update.source, "# ATX 标题");

        update = edit_frame(
            &context,
            id,
            &update.source,
            update.selection,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        update.selection = complete_visual_enter(&mut update.source, update.selection, false);
        assert_eq!(update.source, "# ATX 标题\n");
        update = type_frame(&context, id, &update.source, update.selection, "下一行");
        assert_eq!(update.source, "# ATX 标题\n下一行");
    }

    #[test]
    fn block_boundary_backspace_can_remove_one_blank_line_at_a_time() {
        let mut source = "`a\n\n\n`".to_owned();
        for expected in ["`a\n\n`", "`a\n`"] {
            let blocks = markdown::blocks(&source);
            let closing = source.chars().count() - 1;
            let block = block_for_char_index(&source, &blocks, closing);
            let edit_range = hybrid_edit_range(&source, &blocks, block.id);
            assert_eq!(&source[edit_range.clone()], "`");

            let original_block = source[edit_range.clone()].to_owned();
            let (replacement_range, cursor) = boundary_backspace_edit(&source, edit_range).unwrap();
            source.replace_range(replacement_range, &original_block);
            assert_eq!(source, expected);
            assert_eq!(cursor, expected.chars().count() - 1);
        }

        assert_eq!(line_break_before("one\r\ntwo", 5), Some(3..5));
        assert_eq!(line_break_before("one two", 4), None);
    }

    #[test]
    fn paragraph_boundary_backspace_removes_each_extra_blank_line() {
        let mut source = "first\n\n\n\nsecond".to_owned();
        for expected in ["first\n\n\nsecond", "first\n\nsecond"] {
            let blocks = markdown::blocks(&source);
            let second_range = hybrid_edit_range(&source, &blocks, blocks[1].id);
            let second = source[second_range.clone()].to_owned();
            let (replacement, cursor) = boundary_backspace_edit(&source, second_range).unwrap();
            source.replace_range(replacement, &second);
            assert_eq!(source, expected);
            assert_eq!(cursor, expected.find("second").unwrap());
        }
    }

    #[test]
    fn wysiwyg_editor_preserves_extra_blank_lines_between_blocks() {
        for (source, expected) in [
            ("第一段\n\n第二段", 0),
            ("第一段\n\n\n第二段", 1),
            ("第一段\n\n\n\n第二段", 2),
        ] {
            let blocks = markdown::blocks(source);
            assert_eq!(blocks.len(), 2, "source: {source:?}");
            let gap = blocks[0].range.end..blocks[1].range.start;
            assert_eq!(
                extra_inter_block_blank_lines(source, gap),
                expected,
                "source: {source:?}"
            );
        }

        let source = "第一段\n\n\n\n第二段";
        let blocks = markdown::blocks(source);
        let first_range = hybrid_edit_range(source, &blocks, blocks[0].id);
        assert_eq!(&source[first_range], "第一段\n\n\n\n");

        let gap = blocks[0].range.end..blocks[1].range.start;
        let first_blank_cursor = "第一段\n\n".chars().count();
        assert_eq!(
            block_for_char_index(source, &blocks, first_blank_cursor).id,
            blocks[0].id
        );
        assert_eq!(
            block_for_char_index(source, &blocks, "第一段\n\n\n\n".chars().count()).id,
            blocks[1].id
        );
        assert_eq!(
            inter_block_blank_line_cursor_byte(source, gap.clone(), 0),
            Some("第一段\n\n".len())
        );
        assert_eq!(
            inter_block_blank_line_cursor_byte(source, gap, 1),
            Some("第一段\n\n\n".len())
        );

        let crlf = "first\r\n\r\n\r\n\r\nsecond";
        let crlf_blocks = markdown::blocks(crlf);
        let crlf_gap = crlf_blocks[0].range.end..crlf_blocks[1].range.start;
        assert_eq!(extra_inter_block_blank_lines(crlf, crlf_gap.clone()), 2);
        assert_eq!(
            inter_block_blank_line_cursor_byte(crlf, crlf_gap, 0),
            Some("first\r\n\r\n".len())
        );
    }

    #[test]
    fn classifies_ime_preedit_commit_and_cancel_frames() {
        use egui::{Event, ImeEvent};

        assert_eq!(
            ime_frame_action(&[Event::Ime(ImeEvent::Preedit {
                text: "ni".to_owned(),
                active_range_chars: Some(0..2),
            })]),
            ImeFrameAction::Preedit
        );
        assert_eq!(
            ime_frame_action(&[Event::Ime(ImeEvent::Commit("你".to_owned()))]),
            ImeFrameAction::Commit
        );
        assert_eq!(
            ime_frame_action(&[Event::Ime(ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            })]),
            ImeFrameAction::Cancel
        );
    }

    #[test]
    fn wysiwyg_ime_session_preserves_preedit_text_between_frames() {
        use egui::{Event, Id, ImeEvent, RawInput};

        fn run_frame(
            context: &Context,
            text: &mut String,
            events: Vec<Event>,
            request_focus: bool,
        ) {
            let _ = context.run_ui(
                RawInput {
                    events,
                    ..RawInput::default()
                },
                |ui| {
                    let id = Id::new("wysiwyg-ime-regression");
                    if request_focus {
                        ui.memory_mut(|memory| memory.request_focus(id));
                    }
                    TextEdit::multiline(text).id(id).show(ui);
                },
            );
        }

        let context = Context::default();
        let block_id = markdown::blocks("")[0].id;
        let mut session = HybridImeSession {
            document_id: 1,
            block_id,
            base_source: String::new(),
            visual_content: String::new(),
        };

        run_frame(&context, &mut session.visual_content, Vec::new(), true);
        run_frame(
            &context,
            &mut session.visual_content,
            vec![Event::Ime(ImeEvent::Preedit {
                text: "n".to_owned(),
                active_range_chars: Some(0..1),
            })],
            false,
        );
        assert_eq!(session.visual_content, "n");

        run_frame(
            &context,
            &mut session.visual_content,
            vec![Event::Ime(ImeEvent::Preedit {
                text: "ni".to_owned(),
                active_range_chars: Some(0..2),
            })],
            false,
        );
        assert_eq!(session.visual_content, "ni");

        run_frame(
            &context,
            &mut session.visual_content,
            vec![Event::Ime(ImeEvent::Commit("你".to_owned()))],
            false,
        );
        assert_eq!(session.visual_content, "你");
    }

    #[test]
    fn wysiwyg_third_backtick_accepts_code_on_the_very_next_frame() {
        use egui::{Event, Id, Modifiers, RawInput};

        fn edit_frame(
            context: &Context,
            id: Id,
            source: &str,
            source_selection: std::ops::Range<usize>,
            events: Vec<Event>,
        ) -> crate::wysiwyg::VisualSourceEdit {
            let projection = VisualProjection::from_markdown_with_selection(
                source,
                Some(source_selection.clone()),
            );
            let visual_selection = projection.visual_char_range(source, source_selection);
            let mut visual_content = projection.text().to_owned();
            let mut result = None;
            let _ = context.run_ui(
                RawInput {
                    events,
                    ..RawInput::default()
                },
                |ui| {
                    let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(
                            visual_selection.start,
                        ))));
                    state.store(ui.ctx(), id);
                    ui.memory_mut(|memory| memory.request_focus(id));
                    let mut editor = TextEdit::multiline(&mut visual_content).id(id);
                    if is_fenced_code_block(source) {
                        editor = editor.code_editor();
                    }
                    let output = editor.show(ui);
                    let visual_cursor = text_edit_cursor_after_input(&output)
                        .map(cursor_range_to_char_range)
                        .expect("focused editor should retain a cursor");
                    result = projection.apply_edit(source, &visual_content, visual_cursor);
                },
            );
            result.expect("typing should update the projected source")
        }

        fn type_frame(
            context: &Context,
            id: Id,
            source: &str,
            source_selection: std::ops::Range<usize>,
            text: &str,
        ) -> crate::wysiwyg::VisualSourceEdit {
            edit_frame(
                context,
                id,
                source,
                source_selection,
                vec![Event::Text(text.to_owned())],
            )
        }

        let context = Context::default();
        let id = Id::new("wysiwyg-fence-first-code-regression");
        let mut update = crate::wysiwyg::VisualSourceEdit {
            source: String::new(),
            selection: 0..0,
        };
        for marker in ["`", "`", "`"] {
            update = type_frame(&context, id, &update.source, update.selection, marker);
            if let Some(selection) =
                complete_bare_fenced_code_after_typing(&mut update.source, update.selection.clone())
            {
                update.selection = selection;
            }
        }
        assert_eq!(update.source, "```\n\n```");
        assert_eq!(update.selection, 4..4);

        update = type_frame(
            &context,
            id,
            &update.source,
            update.selection,
            "fn main() {} 中文",
        );
        assert_eq!(update.source, "```\nfn main() {} 中文\n```");
        assert_eq!(
            VisualProjection::from_markdown(&update.source).text(),
            "fn main() {} 中文"
        );

        update = edit_frame(
            &context,
            id,
            &update.source,
            update.selection,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: Some(Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        for marker in ["`", "`", "`"] {
            update = type_frame(&context, id, &update.source, update.selection, marker);
            if let Some(selection) =
                consume_paired_fenced_code_closer(&mut update.source, update.selection.clone())
            {
                update.selection = selection;
            } else if let Some(selection) =
                complete_bare_fenced_code_after_typing(&mut update.source, update.selection.clone())
            {
                update.selection = selection;
            }
        }
        assert_eq!(update.source, "```\nfn main() {} 中文\n```\n\n");
        assert_eq!(update.selection.end, update.source.chars().count());

        update = type_frame(&context, id, &update.source, update.selection, "AFTER");
        assert_eq!(update.source, "```\nfn main() {} 中文\n```\n\nAFTER");
    }

    #[test]
    fn cross_block_input_is_consumed_before_the_active_widget_can_duplicate_it() {
        use egui::{Event, RawInput};

        let context = Context::default();
        let mut action = None;
        let mut remaining = usize::MAX;
        let _ = context.run_ui(
            RawInput {
                events: vec![Event::Text("替换".to_owned())],
                ..RawInput::default()
            },
            |ui| {
                action = take_cross_block_input(ui);
                remaining = ui.input(|input| input.events.len());
            },
        );
        assert!(matches!(action, Some(CrossBlockInput::Replace(ref text)) if text == "替换"));
        assert_eq!(remaining, 0);

        let selected = "前段\n\n```\ncode\n```";
        let range = clamp_char_range(selected, 2..selected.chars().count());
        let start = char_to_byte(selected, range.start);
        let end = char_to_byte(selected, range.end);
        let mut replaced = selected.to_owned();
        replaced.replace_range(start..end, "新");
        assert_eq!(replaced, "前段新");
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
