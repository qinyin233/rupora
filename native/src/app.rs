use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eframe::{
    CreationContext, Frame, Storage,
    egui::{
        self, Align, Button, CentralPanel, Color32, Context, FontData, FontDefinitions, FontFamily,
        Key, Layout, Panel, RichText, ScrollArea, TextEdit, Ui, Vec2, ViewportCommand,
        text::{CCursor, CCursorRange},
    },
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};

use crate::{
    document::{Document, EditKind},
    editing::{self, MarkdownCommand},
    export,
    markdown::{self, BlockId, Heading},
    recovery::RecoveryStore,
    table::{self, MarkdownTable},
    workspace::{Workspace, WorkspaceEntry},
};

const APP_STATE_KEY: &str = "rupora-native-state";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum ViewMode {
    Edit,
    #[default]
    Split,
    Hybrid,
    Preview,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct PersistedState {
    dark: bool,
    show_sidebar: bool,
    show_outline: bool,
    view_mode: ViewMode,
    recent_files: Vec<PathBuf>,
    session_files: Vec<PathBuf>,
    active_session_file: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    cursor_positions: HashMap<PathBuf, usize>,
    scroll_positions: HashMap<PathBuf, f32>,
    key_bindings: KeyBindings,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            dark: false,
            show_sidebar: true,
            show_outline: true,
            view_mode: ViewMode::Split,
            recent_files: Vec::new(),
            session_files: Vec::new(),
            active_session_file: None,
            workspace_root: None,
            cursor_positions: HashMap::new(),
            scroll_positions: HashMap::new(),
            key_bindings: KeyBindings::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct KeyBindings {
    new_document: String,
    open_file: String,
    open_folder: String,
    save: String,
    save_as: String,
    undo: String,
    redo: String,
    find: String,
    replace: String,
    command_palette: String,
    bold: String,
    italic: String,
    link: String,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            new_document: "Ctrl+N".to_owned(),
            open_file: "Ctrl+O".to_owned(),
            open_folder: "Ctrl+Shift+O".to_owned(),
            save: "Ctrl+S".to_owned(),
            save_as: "Ctrl+Shift+S".to_owned(),
            undo: "Ctrl+Z".to_owned(),
            redo: "Ctrl+Shift+Z".to_owned(),
            find: "Ctrl+F".to_owned(),
            replace: "Ctrl+H".to_owned(),
            command_palette: "Ctrl+Shift+P".to_owned(),
            bold: "Ctrl+B".to_owned(),
            italic: "Ctrl+I".to_owned(),
            link: "Ctrl+K".to_owned(),
        }
    }
}

#[derive(Clone, Copy)]
enum AppCommand {
    New,
    Open,
    OpenFolder,
    ShortcutSettings,
    Save,
    SaveAs,
    Undo,
    Redo,
    ExportHtml,
    ExportPdf,
    Print,
    EditTable,
    InsertToc,
    InsertFootnote,
    PasteImage,
    Format(MarkdownCommand),
    SetView(ViewMode),
}

struct TableEditorState {
    document: usize,
    table: MarkdownTable,
}

enum ShortcutAction {
    Command(AppCommand),
    Find,
    Replace,
    Palette,
}

pub struct RuporaApp {
    documents: Vec<Document>,
    active: Option<usize>,
    next_untitled_id: usize,
    state: PersistedState,
    status: String,
    preview_cache: CommonMarkCache,
    allow_close: bool,
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
    collapsed_blocks: HashSet<(usize, BlockId)>,
    shortcut_settings_open: bool,
    external_diff_view: Option<String>,
    generated_svg_cache: Rc<RefCell<HashMap<String, Arc<[u8]>>>>,
    table_editor: Option<TableEditorState>,
}

impl RuporaApp {
    pub fn new(creation_context: &CreationContext<'_>) -> Self {
        install_fonts(&creation_context.egui_ctx);
        let state: PersistedState = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, APP_STATE_KEY))
            .unwrap_or_default();
        apply_theme(&creation_context.egui_ctx, state.dark);
        let recovery_store = RecoveryStore::for_app("RUPORA");
        let recovered_entries = recovery_store.load();
        let workspace = state
            .workspace_root
            .as_ref()
            .and_then(|root| Workspace::open(root.clone()).ok());

        let mut app = Self {
            documents: Vec::new(),
            active: None,
            next_untitled_id: 1,
            state,
            status: "纯 Rust 原生内核已就绪".to_owned(),
            preview_cache: CommonMarkCache::default(),
            allow_close: false,
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
            collapsed_blocks: HashSet::new(),
            shortcut_settings_open: false,
            external_diff_view: None,
            generated_svg_cache: Rc::new(RefCell::new(HashMap::new())),
            table_editor: None,
        };

        match recovered_entries {
            Ok(entries) if !entries.is_empty() => {
                let recovered_count = entries.len();
                for entry in entries {
                    let document =
                        Document::recover(entry.path, entry.content, app.next_untitled_id);
                    app.next_untitled_id += 1;
                    app.documents.push(document);
                }
                app.active = Some(0);
                app.status = format!("已恢复 {recovered_count} 个未保存文档");
            }
            Ok(_) => {}
            Err(error) => {
                app.status = error;
                app.recovery_error_reported = true;
            }
        }

        let startup_files = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
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
        match fs::write(&path, output) {
            Ok(()) => self.status = format!("已导出 HTML：{}", path.display()),
            Err(error) => {
                self.show_error("导出失败", &format!("无法写入 {}：{error}", path.display()))
            }
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
        match export::write_pdf(&path, &html) {
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
        match export::print_html(&html) {
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
        self.collapsed_blocks.clear();
        self.active = match (self.active, self.documents.is_empty()) {
            (_, true) => None,
            (Some(active), false) if active > index => Some(active - 1),
            (Some(active), false) if active == index => Some(index.min(self.documents.len() - 1)),
            (active, false) => active,
        };
        self.editor_cursor = None;
        self.pending_editor_cursor = None;
        self.restore_active_view_state();
        if self.documents.is_empty() {
            self.new_document();
        }
    }

    fn show_error(&mut self, title: &str, message: &str) {
        self.status = message.to_owned();
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
            AppCommand::Format(command) => self.apply_format(command),
            AppCommand::SetView(mode) => self.state.view_mode = mode,
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
        Panel::top("toolbar").exact_size(48.0).show(root, |ui| {
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
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
                            self.execute(AppCommand::Format(MarkdownCommand::Heading(level)));
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
                        .map(|index| markdown::heading_anchors(&self.documents[index].content))
                        .unwrap_or_default();
                    ui.menu_button("交叉引用", |ui| {
                        if anchors.is_empty() {
                            ui.label("当前文档没有标题");
                        }
                        for anchor in &anchors {
                            let label =
                                format!("H{}  {}", anchor.heading.level, anchor.heading.text);
                            if ui.button(label).clicked() {
                                self.insert_cross_reference(&anchor.heading.text, &anchor.id);
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
                ui.separator();
                ui.selectable_value(&mut self.state.view_mode, ViewMode::Edit, "编辑");
                ui.selectable_value(&mut self.state.view_mode, ViewMode::Split, "分屏");
                ui.selectable_value(&mut self.state.view_mode, ViewMode::Hybrid, "混合");
                ui.selectable_value(&mut self.state.view_mode, ViewMode::Preview, "预览");

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
            ("切换到编辑模式", AppCommand::SetView(ViewMode::Edit)),
            ("切换到分屏模式", AppCommand::SetView(ViewMode::Split)),
            ("切换到混合模式", AppCommand::SetView(ViewMode::Hybrid)),
            ("切换到预览模式", AppCommand::SetView(ViewMode::Preview)),
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
                        .hint_text("输入命令，例如：保存、混合、粗体"),
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
        if !self.state.show_sidebar {
            return;
        }

        Panel::left("documents")
            .default_size(230.0)
            .size_range(160.0..=420.0)
            .resizable(true)
            .show(root, |ui| {
                ui.add_space(8.0);
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

                ui.horizontal(|ui| {
                    ui.heading("文档");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("+").on_hover_text("新建文档").clicked() {
                            self.new_document();
                        }
                    });
                });
                ui.separator();

                let mut activate = None;
                let mut close = None;
                ScrollArea::vertical().show(ui, |ui| {
                    for (index, document) in self.documents.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let title = if document.dirty {
                                format!("● {}", document.title())
                            } else {
                                document.title()
                            };
                            let selected = self.active == Some(index);
                            let response = ui.selectable_label(selected, title).on_hover_text(
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
                                .add(Button::new("×").frame(false).small())
                                .on_hover_text("关闭")
                                .clicked()
                            {
                                close = Some(index);
                            }
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
        Panel::right("outline")
            .default_size(220.0)
            .size_range(150.0..=400.0)
            .resizable(true)
            .show(root, |ui| {
                ui.add_space(8.0);
                ui.heading("大纲");
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

    fn editor(&mut self, root: &mut Ui) {
        CentralPanel::default().show(root, |ui| {
            let Some(index) = self.active else {
                ui.centered_and_justified(|ui| {
                    ui.label("新建或打开一个 Markdown 文档");
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
                    let editor_target = (self.split_scroll_driver == SplitScrollDriver::Preview)
                        .then_some(self.split_scroll_ratio * self.split_editor_maximum);
                    let preview_target = (self.split_scroll_driver == SplitScrollDriver::Editor)
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
        let before_content = self.documents[index].content.clone();
        let selection_before = self.editor_cursor.map(cursor_range_to_char_range);
        let mut scroll_area = ScrollArea::vertical().id_salt(("editor-scroll", index));
        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }
        let output = scroll_area.show(ui, |ui| {
            let available = ui.available_size();
            ui.set_min_size(Vec2::new(available.x, available.y.max(420.0)));
            let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(1.0);
            let desired_rows = (available.y / row_height).max(20.0) as usize;
            let editor_id = ui.make_persistent_id(("editor", index));
            if let Some(cursor_range) = self.pending_editor_cursor.take() {
                let mut state = TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
                state.cursor.set_char_range(Some(cursor_range));
                state.store(ui.ctx(), editor_id);
                ui.memory_mut(|memory| memory.request_focus(editor_id));
                self.editor_cursor = Some(cursor_range);
            }
            let input_action = editor_input_action(ui);
            let output = TextEdit::multiline(&mut self.documents[index].content)
                .id(editor_id)
                .font(egui::TextStyle::Monospace)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(desired_rows)
                .lock_focus(true)
                .show(ui);
            let mut selection_after = output.cursor_range.map(cursor_range_to_char_range);
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
            {
                self.documents[index].content.clone_from(&before_content);
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
            } else if focused && input_action.tab {
                self.documents[index].content.clone_from(&before_content);
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
            {
                let mut paired = before_content.clone();
                if let Some(next) = editing::apply_smart_pair(&mut paired, selection, typed) {
                    changed = paired != before_content;
                    self.documents[index].content = paired;
                    selection_after = Some(next);
                    kind = EditKind::Other;
                    cursor_adjusted = true;
                }
            } else if focused
                && changed
                && input_action.enter
                && let Some(cursor) = selection_after.as_ref().map(|range| range.end)
                && let Some(next) =
                    editing::continue_markdown_line(&mut self.documents[index].content, cursor)
            {
                selection_after = Some(next);
                cursor_adjusted = true;
            }

            if changed
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
        let before_content = self.documents[index].content.clone();
        let mut preview_content = prepare_native_preview(
            ui.ctx(),
            &before_content,
            self.state.dark,
            &mut self.generated_svg_cache.borrow_mut(),
        );
        let base_uri = self.preview_base_uri(index);
        let local_links = markdown::local_link_destinations(&before_content);
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
                ui.add_space(12.0);
                let changed = ui
                    .horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 24.0).max(200.0));
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

    fn hybrid_pane(&mut self, ui: &mut Ui, index: usize) {
        let source = self.documents[index].content.clone();
        let selection_before = self.editor_cursor.map(cursor_range_to_char_range);
        let blocks = self.documents[index].blocks().to_vec();
        let base_uri = self.preview_base_uri(index);
        let local_links = markdown::local_link_destinations(&source);
        let preview_blocks = blocks
            .iter()
            .map(|block| {
                (
                    block.id,
                    prepare_native_preview(
                        ui.ctx(),
                        &source[block.range.clone()],
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

        let mut pending_local = None;
        if let Some(cursor_range) = self.pending_editor_cursor.take() {
            let [selection_start, selection_end] = cursor_range.sorted_cursors();
            let selected_block = block_for_char_index(&source, &blocks, selection_start.index.0);
            let block_char_start = source[..selected_block.range.start].chars().count();
            pending_local = Some(CCursorRange::two(
                CCursor::new(selection_start.index.0.saturating_sub(block_char_start)),
                CCursor::new(selection_end.index.0.saturating_sub(block_char_start)),
            ));
            self.hybrid_active = Some((index, selected_block.id));
        }

        let active_block = self
            .hybrid_active
            .filter(|(document, id)| {
                *document == index && blocks.iter().any(|block| block.id == *id)
            })
            .and_then(|(_, id)| blocks.iter().find(|block| block.id == id))
            .unwrap_or(&blocks[0]);
        let active_id = active_block.id;
        self.hybrid_active = Some((index, active_id));

        let mut pending_edit = None;
        let mut activate = None;
        let mut next_global_cursor = None;
        let mut cursor_adjusted = false;

        ScrollArea::vertical()
            .id_salt(("hybrid-scroll", index))
            .show(ui, |ui| {
                ui.add_space(12.0);
                ui.set_max_width((ui.available_width() - 28.0).max(260.0));
                for block in &blocks {
                    ui.push_id(("hybrid-block", block.id), |ui| {
                        if block.id == active_id {
                            let mut block_content = source[block.range.clone()].to_owned();
                            let original_block = block_content.clone();
                            let block_char_start = source[..block.range.start].chars().count();
                            let local_selection_before =
                                selection_before.as_ref().map(|selection| {
                                    selection.start.saturating_sub(block_char_start)
                                        ..selection.end.saturating_sub(block_char_start)
                                });
                            let editor_id =
                                ui.make_persistent_id(("hybrid-editor", index, block.id));
                            if let Some(cursor_range) = pending_local.take() {
                                let mut state =
                                    TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
                                state.cursor.set_char_range(Some(cursor_range));
                                state.store(ui.ctx(), editor_id);
                                ui.memory_mut(|memory| memory.request_focus(editor_id));
                            }

                            let frame = egui::Frame::group(ui.style())
                                .inner_margin(10.0)
                                .stroke(ui.visuals().selection.stroke);
                            frame.show(ui, |ui| {
                                let desired_rows = block_content.lines().count().max(1);
                                let input_action = editor_input_action(ui);
                                let output = TextEdit::multiline(&mut block_content)
                                    .id(editor_id)
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(desired_rows)
                                    .lock_focus(true)
                                    .show(ui);
                                let mut local_selection_after =
                                    output.cursor_range.map(cursor_range_to_char_range);
                                let focused = output.response.has_focus();
                                let mut changed = output.response.changed();
                                let mut kind = EditKind::Typing;

                                if focused
                                    && let (Some(url), Some(selection)) = (
                                        input_action.pasted_url.as_deref(),
                                        local_selection_before.clone(),
                                    )
                                    && !selection.is_empty()
                                {
                                    block_content.clone_from(&original_block);
                                    if let Some(next) = editing::paste_url_as_markdown_link(
                                        &mut block_content,
                                        selection,
                                        url,
                                    ) {
                                        local_selection_after = Some(next);
                                        kind = EditKind::Other;
                                        changed = true;
                                        cursor_adjusted = true;
                                    }
                                } else if focused && input_action.tab {
                                    block_content.clone_from(&original_block);
                                    let selection =
                                        local_selection_before.clone().unwrap_or_else(|| {
                                            let end = original_block.chars().count();
                                            end..end
                                        });
                                    local_selection_after = Some(editing::indent_selected_lines(
                                        &mut block_content,
                                        selection,
                                        input_action.shift,
                                    ));
                                    kind = EditKind::Other;
                                    changed = true;
                                    cursor_adjusted = true;
                                } else if focused
                                    && let (Some(typed), Some(selection)) = (
                                        input_action.typed_text.as_deref(),
                                        local_selection_before.clone(),
                                    )
                                {
                                    let mut paired = original_block.clone();
                                    if let Some(next) =
                                        editing::apply_smart_pair(&mut paired, selection, typed)
                                    {
                                        changed = paired != original_block;
                                        block_content = paired;
                                        local_selection_after = Some(next);
                                        kind = EditKind::Other;
                                        cursor_adjusted = true;
                                    }
                                } else if focused
                                    && changed
                                    && input_action.enter
                                    && let Some(cursor) =
                                        local_selection_after.as_ref().map(|range| range.end)
                                    && let Some(next) =
                                        editing::continue_markdown_line(&mut block_content, cursor)
                                {
                                    local_selection_after = Some(next);
                                    cursor_adjusted = true;
                                }

                                if let Some(selection) = local_selection_after {
                                    next_global_cursor = Some(CCursorRange::two(
                                        CCursor::new(block_char_start + selection.start),
                                        CCursor::new(block_char_start + selection.end),
                                    ));
                                }
                                if changed {
                                    pending_edit =
                                        Some((block.range.clone(), block_content.clone(), kind));
                                }
                            });
                        } else {
                            let block_text = preview_blocks
                                .get(&block.id)
                                .map(String::as_str)
                                .unwrap_or(&source[block.range.clone()]);
                            let foldable = is_foldable_block(block_text);
                            let collapse_key = (index, block.id);
                            let collapsed = self.collapsed_blocks.contains(&collapse_key);
                            if foldable {
                                ui.horizontal(|ui| {
                                    let icon = if collapsed { "▸" } else { "▾" };
                                    if ui
                                        .small_button(icon)
                                        .on_hover_text(if collapsed {
                                            "展开 Markdown 块"
                                        } else {
                                            "折叠 Markdown 块"
                                        })
                                        .clicked()
                                    {
                                        if collapsed {
                                            self.collapsed_blocks.remove(&collapse_key);
                                        } else {
                                            self.collapsed_blocks.insert(collapse_key);
                                        }
                                    }
                                    ui.label(
                                        RichText::new(format!("第 {} 行", block.line))
                                            .small()
                                            .weak(),
                                    );
                                });
                            }
                            if collapsed {
                                return;
                            }
                            let shown = ui.scope(|ui| {
                                ui.add_space(8.0);
                                CommonMarkViewer::new()
                                    .default_implicit_uri_scheme(base_uri.clone())
                                    .render_math_fn(Some(&render_math))
                                    .show(ui, &mut self.preview_cache, block_text);
                                ui.add_space(8.0);
                            });
                            let response = ui
                                .interact(
                                    shown.response.rect,
                                    ui.make_persistent_id(("activate-block", block.id)),
                                    egui::Sense::click(),
                                )
                                .on_hover_text(format!(
                                    "点击编辑第 {} 行开始的 Markdown 块",
                                    block.line
                                ));
                            if response.hovered() {
                                ui.painter().rect_stroke(
                                    response.rect,
                                    4.0,
                                    ui.visuals().selection.stroke,
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if response.clicked() {
                                activate = Some((block.id, block.range.start));
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(40.0);
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
            self.hybrid_active = Some((index, active_id));
            self.status = "已更新当前 Markdown 块".to_owned();
        }
        if cursor_adjusted {
            self.pending_editor_cursor = next_global_cursor;
        }
        if let Some((id, start)) = activate {
            let char_start = source[..start].chars().count();
            self.hybrid_active = Some((index, id));
            self.queue_editor_selection(char_start..char_start);
        }

        let clicked_link = local_links
            .into_iter()
            .find(|destination| self.preview_cache.get_link_hook(destination) == Some(true));
        if let Some(destination) = clicked_link {
            self.open_local_preview_link(index, &destination);
        }
    }

    fn preview_base_uri(&self, index: usize) -> String {
        let base = self.documents[index]
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
            .unwrap_or_else(|| PathBuf::from("."));
        file_uri_base(&base)
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

        Panel::bottom("status").exact_size(28.0).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(&self.status).small().color(Color32::GRAY));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(document_info).small().color(Color32::GRAY));
                });
            });
        });
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
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

impl eframe::App for RuporaApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));
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
        self.shortcut_settings(ui);
        self.external_diff_window(ui);
        self.table_editor_window(ui);
        self.external_change_bar(ui);
        self.status_bar(ui);
        self.sidebar(ui);
        self.outline(ui);
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
        let _ = self.recovery_store.save(&self.documents);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.recovery_store.clear();
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
        .or_else(|| blocks.iter().find(|block| block.range.start >= byte_index))
        .unwrap_or_else(|| blocks.last().expect("Markdown always has an editing block"))
}

fn cursor_range_to_char_range(range: CCursorRange) -> std::ops::Range<usize> {
    let [start, end] = range.sorted_cursors();
    start.index.0..end.index.0
}

fn scroll_ratio(scroll: PaneScroll) -> f32 {
    if scroll.maximum <= f32::EPSILON {
        0.0
    } else {
        (scroll.offset / scroll.maximum).clamp(0.0, 1.0)
    }
}

fn is_foldable_block(source: &str) -> bool {
    source.contains('\n')
        || source.starts_with('#')
        || source.starts_with("```")
        || source.starts_with("~~~")
        || source.starts_with("> ")
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
    enter: bool,
    tab: bool,
    shift: bool,
    pasted_url: Option<String>,
    typed_text: Option<String>,
}

fn editor_input_action(ui: &Ui) -> EditorInputAction {
    ui.input(|input| EditorInputAction {
        enter: input.key_pressed(Key::Enter),
        tab: input.key_pressed(Key::Tab),
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

fn prepare_native_preview(
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
                cache.insert(key.clone(), bytes.clone());
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

fn render_math_widget(
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
            cache.insert(key.clone(), bytes.clone());
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

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
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
    if dark {
        ctx.set_visuals(egui::Visuals::dark());
    } else {
        ctx.set_visuals(egui::Visuals::light());
    }
}

fn install_fonts(ctx: &Context) {
    let Some((path, bytes)) = export::cjk_font_candidates()
        .into_iter()
        .find_map(|path| fs::read(&path).ok().map(|bytes| (path, bytes)))
    else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    let font_name = format!("rupora-cjk-{}", path.display());
    fonts
        .font_data
        .insert(font_name.clone(), Arc::new(FontData::from_owned(bytes)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.clone());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_supported_document_extensions_case_insensitively() {
        assert!(is_markdown_path(Path::new("README.MD")));
        assert!(is_markdown_path(Path::new("notes.markdown")));
        assert!(!is_markdown_path(Path::new("image.png")));
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
            false,
            &mut cache,
        );
        assert!(preview.contains("bytes://rupora/mermaid-"));
        assert!(!preview.contains("```mermaid"));
        assert_eq!(cache.len(), 1);
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
}
