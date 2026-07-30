use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
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
    markdown::{self, BlockId, Heading},
    recovery::RecoveryStore,
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
        }
    }
}

enum AppCommand {
    New,
    Open,
    OpenFolder,
    Save,
    SaveAs,
    Undo,
    Redo,
    ExportHtml,
    Format(MarkdownCommand),
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
        app
    }

    fn new_document(&mut self) {
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
                self.active = Some(index);
                self.editor_cursor = None;
                self.pending_editor_cursor = None;
                continue;
            }

            match Document::open(&path) {
                Ok(document) => {
                    self.status =
                        format!("已打开：{} · {}", path.display(), document.encoding.label());
                    self.remove_initial_placeholder();
                    self.documents.push(document);
                    self.active = Some(self.documents.len() - 1);
                    self.editor_cursor = None;
                    self.pending_editor_cursor = None;
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
                    self.remember_recent(path);
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

    fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        if self.documents[index].dirty {
            match prompt_to_save(&self.documents[index].title()) {
                MessageDialogResult::Yes => {
                    self.active = Some(index);
                    self.save_active(false);
                    if self.documents[index].dirty {
                        return;
                    }
                }
                MessageDialogResult::No => {}
                _ => return,
            }
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
        let history_command = ctx.input_mut(|input| {
            let command_shift = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
            if input.consume_key(command_shift, Key::Z)
                || input.consume_key(egui::Modifiers::COMMAND, Key::Y)
            {
                Some(AppCommand::Redo)
            } else if input.consume_key(egui::Modifiers::COMMAND, Key::Z) {
                Some(AppCommand::Undo)
            } else {
                None
            }
        });
        if let Some(command) = history_command {
            self.execute(command);
        }

        let command = ctx.input(|input| {
            if input.modifiers.command && input.key_pressed(Key::S) {
                Some(if input.modifiers.shift {
                    AppCommand::SaveAs
                } else {
                    AppCommand::Save
                })
            } else if input.modifiers.command && input.key_pressed(Key::O) {
                Some(if input.modifiers.shift {
                    AppCommand::OpenFolder
                } else {
                    AppCommand::Open
                })
            } else if input.modifiers.command && input.key_pressed(Key::N) {
                Some(AppCommand::New)
            } else if input.modifiers.command && input.key_pressed(Key::B) {
                Some(AppCommand::Format(MarkdownCommand::Bold))
            } else if input.modifiers.command && input.key_pressed(Key::I) {
                Some(AppCommand::Format(MarkdownCommand::Italic))
            } else if input.modifiers.command && input.key_pressed(Key::K) {
                Some(AppCommand::Format(MarkdownCommand::Link))
            } else {
                None
            }
        });

        if let Some(command) = command {
            self.execute(command);
        }

        if ctx.input(|input| input.modifiers.command && input.key_pressed(Key::F))
            || ctx.input(|input| input.modifiers.command && input.key_pressed(Key::H))
        {
            self.find_open = true;
            self.find_focus_requested = true;
        }
        if ctx.input(|input| input.key_pressed(Key::Escape)) && self.find_open {
            self.find_open = false;
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
            AppCommand::Save => self.save_active(false),
            AppCommand::SaveAs => self.save_active(true),
            AppCommand::Undo => self.undo_active(),
            AppCommand::Redo => self.redo_active(),
            AppCommand::ExportHtml => self.export_html(),
            AppCommand::Format(command) => self.apply_format(command),
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
            let mut files = Vec::new();
            for path in paths {
                if path.is_dir() {
                    self.open_workspace(path);
                } else {
                    files.push(path);
                }
            }
            self.open_paths(files);
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
                if ui.button("导出 HTML").clicked() {
                    self.execute(AppCommand::ExportHtml);
                }

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
                });
                if ui.button("查找").on_hover_text("Ctrl+F").clicked() {
                    self.find_open = true;
                    self.find_focus_requested = true;
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
                    self.active = Some(index);
                    self.editor_cursor = None;
                    self.pending_editor_cursor = None;
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
                ViewMode::Edit => self.edit_pane(ui, index),
                ViewMode::Preview => self.preview_pane(ui, index),
                ViewMode::Hybrid => self.hybrid_pane(ui, index),
                ViewMode::Split => {
                    ui.columns(2, |columns| {
                        columns[0].push_id("source-pane", |ui| self.edit_pane(ui, index));
                        columns[1].separator();
                        columns[1].push_id("preview-pane", |ui| self.preview_pane(ui, index));
                    });
                }
            }
        });
    }

    fn edit_pane(&mut self, ui: &mut Ui, index: usize) {
        let before_content = self.documents[index].content.clone();
        let selection_before = self.editor_cursor.map(cursor_range_to_char_range);
        ScrollArea::vertical()
            .id_salt(("editor-scroll", index))
            .show(ui, |ui| {
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
                let output = TextEdit::multiline(&mut self.documents[index].content)
                    .id(editor_id)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(desired_rows)
                    .lock_focus(true)
                    .show(ui);
                let selection_after = output.cursor_range.map(cursor_range_to_char_range);
                if let Some(cursor_range) = output.cursor_range {
                    self.editor_cursor = Some(cursor_range);
                }
                if output.response.changed()
                    && self.documents[index].record_edit(
                        before_content,
                        selection_before,
                        selection_after,
                        EditKind::Typing,
                    )
                {
                    self.status = "已修改".to_owned();
                }
            });
    }

    fn preview_pane(&mut self, ui: &mut Ui, index: usize) {
        let before_content = self.documents[index].content.clone();
        let base_uri = self.preview_base_uri(index);
        let local_links = markdown::local_link_destinations(&self.documents[index].content);
        self.preview_cache.link_hooks_clear();
        for destination in &local_links {
            self.preview_cache.add_link_hook(destination);
        }

        let changed = {
            let content = &mut self.documents[index].content;
            let cache = &mut self.preview_cache;
            ScrollArea::vertical()
                .id_salt(("preview-scroll", index))
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    let changed = ui
                        .horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 24.0).max(200.0));
                                CommonMarkViewer::new()
                                    .default_implicit_uri_scheme(base_uri)
                                    .enable_scroll_to_heading(true)
                                    .show_mut(ui, cache, content)
                                    .response
                                    .changed()
                            })
                            .inner
                        })
                        .inner;
                    ui.add_space(40.0);
                    changed
                })
                .inner
        };

        if changed {
            self.documents[index].record_edit(before_content, None, None, EditKind::TaskList);
            self.status = "已更新任务列表".to_owned();
        }
        let clicked_link = local_links
            .into_iter()
            .find(|destination| self.preview_cache.get_link_hook(destination) == Some(true));
        if let Some(destination) = clicked_link {
            self.open_local_preview_link(index, &destination);
        }
    }

    fn hybrid_pane(&mut self, ui: &mut Ui, index: usize) {
        let source = self.documents[index].content.clone();
        let selection_before = self.editor_cursor.map(cursor_range_to_char_range);
        let blocks = self.documents[index].blocks().to_vec();
        let base_uri = self.preview_base_uri(index);
        let local_links = markdown::local_link_destinations(&source);
        self.preview_cache.link_hooks_clear();
        for destination in &local_links {
            self.preview_cache.add_link_hook(destination);
        }

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

        ScrollArea::vertical()
            .id_salt(("hybrid-scroll", index))
            .show(ui, |ui| {
                ui.add_space(12.0);
                ui.set_max_width((ui.available_width() - 28.0).max(260.0));
                for block in &blocks {
                    ui.push_id(("hybrid-block", block.id), |ui| {
                        if block.id == active_id {
                            let mut block_content = source[block.range.clone()].to_owned();
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
                                let output = TextEdit::multiline(&mut block_content)
                                    .id(editor_id)
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(desired_rows)
                                    .lock_focus(true)
                                    .show(ui);
                                if let Some(cursor_range) = output.cursor_range {
                                    let [local_start, local_end] = cursor_range.sorted_cursors();
                                    let block_char_start =
                                        source[..block.range.start].chars().count();
                                    next_global_cursor = Some(CCursorRange::two(
                                        CCursor::new(block_char_start + local_start.index.0),
                                        CCursor::new(block_char_start + local_end.index.0),
                                    ));
                                }
                                if output.response.changed() {
                                    pending_edit =
                                        Some((block.range.clone(), block_content.clone()));
                                }
                            });
                        } else {
                            let shown = ui.scope(|ui| {
                                ui.add_space(8.0);
                                CommonMarkViewer::new()
                                    .default_implicit_uri_scheme(base_uri.clone())
                                    .show(
                                        ui,
                                        &mut self.preview_cache,
                                        &source[block.range.clone()],
                                    );
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
        if let Some((range, replacement)) = pending_edit {
            self.documents[index]
                .content
                .replace_range(range.clone(), &replacement);
            let selection_after = next_global_cursor.map(cursor_range_to_char_range);
            self.documents[index].record_edit(
                source.clone(),
                selection_before,
                selection_after,
                EditKind::Typing,
            );
            self.hybrid_active = Some((index, active_id));
            self.status = "已更新当前 Markdown 块".to_owned();
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

        if !resolved.exists() {
            self.status = format!("链接目标不存在：{}", resolved.display());
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
        self.handle_shortcuts(ctx);
        self.handle_dropped_files(ctx);
        self.save_recovery_snapshot_if_due();
        self.confirm_application_close(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        self.top_bar(ui);
        self.find_bar(ui);
        self.status_bar(ui);
        self.sidebar(ui);
        self.outline(ui);
        self.editor(ui);
    }

    fn save(&mut self, storage: &mut dyn Storage) {
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

fn apply_theme(ctx: &Context, dark: bool) {
    if dark {
        ctx.set_visuals(egui::Visuals::dark());
    } else {
        ctx.set_visuals(egui::Visuals::light());
    }
}

fn install_fonts(ctx: &Context) {
    let candidates = if cfg!(target_os = "windows") {
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
    };

    let Some((path, bytes)) = candidates
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
    fn creates_an_absolute_file_uri_base() {
        let uri = file_uri_base(Path::new(if cfg!(target_os = "windows") {
            r"C:\notes and docs"
        } else {
            "/tmp/notes and docs"
        }));
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("notes and docs/"));
    }
}
