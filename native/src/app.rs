use crate::background::{BackgroundEvent, BackgroundTasks, ExtensionJobResult};
#[cfg(test)]
use crate::wysiwyg::*;
use crate::{
    app_state::{AppCommand, KeyBindings, PersistedState, ShortcutAction, ViewMode},
    diagnostics,
    document::{Document, DocumentSnapshot, EditKind},
    editing::{self, MarkdownCommand, char_to_byte},
    editor::{
        EditorBookmark, EditorCommand, EditorOptions, EditorSurface, cursor_range_to_char_range,
    },
    export,
    extensions::ExtensionRegistry,
    external_changes::{ExternalChanges, ExternalEvent, ExternalResolution},
    instance::InstanceCoordinator,
    markdown,
    native_preview::decode_local_resource_path,
    presentation::*,
    recovery::{RecoveryEntry, RecoveryStore},
    session::{DocumentSession, SnapshotApplyError},
    table::{self, MarkdownTable},
    updater::{self, UpdateStatus},
    workspace::{Workspace, WorkspaceEntry},
};
#[cfg(test)]
use eframe::egui::{FontDefinitions, FontFamily};
use eframe::{
    CreationContext, Frame, Storage,
    egui::{
        self, Align, Button, CentralPanel, Color32, Context, Key, Layout, Margin, Panel, RichText,
        ScrollArea, Stroke, TextEdit, Ui, Vec2, ViewportCommand,
        text::{CCursor, CCursorRange},
    },
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
#[cfg(test)]
use std::sync::Arc;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const APP_STATE_KEY: &str = "rupora-native-state";
const UI_EXPERIENCE_KEY: &str = "rupora-native-ui-experience";
const CURRENT_UI_EXPERIENCE: u32 = 3;
struct TableEditorState {
    snapshot: DocumentSnapshot,
    table: MarkdownTable,
}

pub struct RuporaApp {
    editor_surface: EditorSurface,
    session: DocumentSession,
    state: PersistedState,
    status: String,
    allow_close: bool,
    discard_recovery_on_exit: bool,
    recovery_store: RecoveryStore,
    last_recovery_write: Instant,
    recovery_error_reported: bool,
    find_open: bool,
    find_query: String,
    replace_query: String,
    find_match_case: bool,
    find_focus_requested: bool,
    workspace: Option<Workspace>,
    external_changes: ExternalChanges,
    command_palette_open: bool,
    command_query: String,
    command_focus_requested: bool,
    shortcut_settings_open: bool,
    external_diff_view: Option<String>,
    table_editor: Option<TableEditorState>,
    instance_coordinator: Option<InstanceCoordinator>,
    background: BackgroundTasks,
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
        egui_extras::install_image_loaders(&creation_context.egui_ctx);
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

        let mut app = Self::from_state(
            state,
            recovery_store,
            workspace,
            extension_registry,
            instance_coordinator,
        );
        if let Some(error) = extension_error {
            app.status = error;
        }

        let mut recovered_count = 0usize;
        match recovered_entries {
            Ok(entries) if !entries.is_empty() => {
                recovered_count = entries.len();
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
                        app.session.next_untitled_number(),
                    );
                    conflict_count += outcome.conflicts;
                    if let Some(warning) = outcome.warning {
                        warning_count += 1;
                        diagnostics::append_event("WARN", &warning).ok();
                    }
                    app.session.insert(outcome.document);
                }
                app.session.activate(app.session[0].id());
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
        } else {
            let recovered_active = app
                .session
                .active_index()
                .and_then(|index| app.session.documents().get(index))
                .map(Document::id);
            let session_files = app
                .session
                .restorable_files(app.state.session_files.iter().cloned());
            if !session_files.is_empty() {
                let documents_before = app.session.documents().len();
                app.open_paths(session_files);
                if let Some(active_path) = app.state.active_session_file.as_ref()
                    && let Some(index) = app
                        .session
                        .documents()
                        .iter()
                        .position(|document| document.path.as_ref() == Some(active_path))
                {
                    app.activate_document(index);
                } else if let Some(recovered_active) = recovered_active
                    && let Some(index) = app
                        .session
                        .documents()
                        .iter()
                        .position(|document| document.id() == recovered_active)
                {
                    app.activate_document(index);
                }
                let reopened = app
                    .session
                    .documents()
                    .len()
                    .saturating_sub(documents_before);
                app.status = if recovered_count > 0 {
                    format!(
                        "已恢复 {recovered_count} 个未保存文档，并重新打开 {reopened} 个会话文档"
                    )
                } else {
                    format!("已恢复上次会话（{reopened} 个文档）")
                };
            }
        }
        if app.session.documents().is_empty() {
            app.new_document();
        }
        app.restore_active_view_state();
        app
    }

    fn from_state(
        state: PersistedState,
        recovery_store: RecoveryStore,
        workspace: Option<Workspace>,
        extension_registry: ExtensionRegistry,
        instance_coordinator: Option<InstanceCoordinator>,
    ) -> Self {
        Self {
            editor_surface: EditorSurface::default(),
            session: DocumentSession::default(),
            state,
            status: "纯 Rust 原生内核已就绪".to_owned(),
            allow_close: false,
            discard_recovery_on_exit: false,
            recovery_store,
            last_recovery_write: Instant::now(),
            recovery_error_reported: false,
            find_open: false,
            find_query: String::new(),
            replace_query: String::new(),
            find_match_case: false,
            find_focus_requested: false,
            workspace,
            external_changes: ExternalChanges::new(Instant::now()),
            command_palette_open: false,
            command_query: String::new(),
            command_focus_requested: false,
            shortcut_settings_open: false,
            external_diff_view: None,
            table_editor: None,
            instance_coordinator,
            background: BackgroundTasks::new(extension_registry),
            about_open: false,
        }
    }

    fn store_active_view_state(&mut self) {
        let Some(document) = self.session.active() else {
            return;
        };
        self.editor_surface.remember(document.id());
        let view = self.editor_surface.bookmark();
        if let Some(path) = document.path.as_ref() {
            if let Some(cursor) = view.cursor {
                self.state
                    .cursor_positions
                    .insert(path.clone(), cursor.primary.index.0);
            }
            self.state
                .scroll_positions
                .insert(path.clone(), view.scroll_ratio);
        }
    }

    fn restore_active_view_state(&mut self) {
        let document = self.session.active();
        let path = document.and_then(|document| document.path.as_ref());
        let fallback = EditorBookmark {
            cursor: path
                .and_then(|path| self.state.cursor_positions.get(path))
                .map(|at| CCursorRange::one(CCursor::new(*at))),
            scroll_ratio: path
                .and_then(|path| self.state.scroll_positions.get(path))
                .copied()
                .unwrap_or(0.0),
        };
        self.editor_surface.bind_document(document, fallback);
    }

    fn activate_document(&mut self, index: usize) {
        let Some(document) = self.session.documents().get(index) else {
            return;
        };
        let id = document.id();
        if self.session.active_id() == Some(id) {
            return;
        }
        self.store_active_view_state();
        self.session.activate(id);
        self.restore_active_view_state();
    }

    fn new_document(&mut self) {
        self.store_active_view_state();
        self.session.new_document();
        self.restore_active_view_state();
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
                let mut notices = Vec::new();
                if workspace.truncated {
                    notices.push("文件过多，列表已截断".to_owned());
                }
                if workspace.skipped_directories > 0 {
                    notices.push(format!(
                        "已跳过 {} 个不可读目录",
                        workspace.skipped_directories
                    ));
                }
                let suffix = (!notices.is_empty()).then(|| format!("（{}）", notices.join("；")));
                self.workspace = Some(workspace);
                self.state.workspace_root = Some(path.clone());
                self.state.show_sidebar = true;
                self.status = format!(
                    "已打开工作区：{}{}",
                    path.display(),
                    suffix.as_deref().unwrap_or_default()
                );
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

            if let Some(id) = self.session.document_id_for_path(&path) {
                self.activate_document(self.session.index_of(id).expect("session document exists"));
                continue;
            }

            match Document::open(&path) {
                Ok(document) => {
                    self.status =
                        format!("已打开：{} · {}", path.display(), document.encoding.label());
                    self.remove_initial_placeholder();
                    self.store_active_view_state();
                    self.session.insert(document);
                    self.restore_active_view_state();
                    self.remember_recent(path);
                }
                Err(error) => self.show_error("打开失败", &error),
            }
        }
    }

    fn remove_initial_placeholder(&mut self) {
        if self.session.documents().len() == 1 {
            let document = &self.session[0];
            if document.path.is_none() && !document.dirty && document.content.is_empty() {
                self.editor_surface.forget(self.session[0].id());
                self.session.remove(self.session[0].id());
            }
        }
    }

    fn remember_recent(&mut self, path: PathBuf) {
        self.state.recent_files.retain(|existing| existing != &path);
        self.state.recent_files.insert(0, path);
        self.state.recent_files.truncate(12);
    }

    fn save_active(&mut self, force_dialog: bool) {
        let Some(index) = self.session.active_index() else {
            return;
        };

        let needs_path = self.session[index].path.is_none() || force_dialog;
        let selected_path = needs_path
            .then(|| {
                let title = self.session[index].title();
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
            match self.session[index].has_external_changes() {
                Ok(true) => {
                    self.external_changes
                        .mark_conflict(self.session[index].id());
                    let path = self.session[index]
                        .path
                        .as_deref()
                        .map(Path::display)
                        .map(|display| display.to_string())
                        .unwrap_or_default();
                    let confirmed = MessageDialog::new()
                        .set_level(MessageLevel::Warning)
                        .set_title("检测到外部修改")
                        .set_description(format!(
                            "{path}\n\n文件已被其他程序修改。确定用 RUPORA 中的内容覆盖吗？"
                        ))
                        .set_buttons(MessageButtons::YesNo)
                        .show()
                        == MessageDialogResult::Yes;
                    if !confirmed {
                        return;
                    }
                    true
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

        let result = if let Some(path) = selected_path {
            self.session[index].save_as(path, true)
        } else {
            self.session[index].save(overwrite_external)
        };

        match result {
            Ok(()) => {
                let document = &self.session[index];
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
                self.external_changes.forget(self.session[index].id());
            }
            Err(error) => self.show_error("保存失败", &error),
        }
    }

    fn export_html(&mut self) {
        let Some(index) = self.session.active_index() else {
            return;
        };
        let document = &self.session[index];
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
        let resource_base = self.preview_base_path(index);
        let images = match export::load_local_images(&document.content, Some(&resource_base)) {
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
        let Some(index) = self.session.active_index() else {
            return;
        };
        let document = &self.session[index];
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
        let resource_base = self.preview_base_path(index);
        let images = match export::load_local_images(&document.content, Some(&resource_base)) {
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
        let Some(index) = self.session.active_index() else {
            return;
        };
        let document = &self.session[index];
        let html =
            markdown::render_html_document(&document.content, &document.title(), self.state.dark);
        let resource_base = self.preview_base_path(index);
        let images = match export::load_local_images(&document.content, Some(&resource_base)) {
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
        let Some(index) = self.session.active_index() else {
            return;
        };
        let selection = self.active_selection(index);
        let cursor = selection.start + text.chars().count();
        self.session[index].edit(kind, Some(selection.clone()), |content| {
            let start = char_to_byte(content, selection.start);
            let end = char_to_byte(content, selection.end);
            content.replace_range(start..end, text);
            Some(cursor..cursor)
        });
        self.finish_document_edit(self.session[index].id(), Some(cursor..cursor));
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
    }

    fn insert_footnote(&mut self) {
        let Some(index) = self.session.active_index() else {
            return;
        };
        let number = next_footnote_number(&self.session[index].content);
        let selection = self.active_selection(index);
        let reference = format!("[^{number}]");
        let cursor = selection.start + reference.chars().count();
        self.session[index].edit(EditKind::Format, Some(selection.clone()), |content| {
            let start = char_to_byte(content, selection.start);
            let end = char_to_byte(content, selection.end);
            content.replace_range(start..end, &reference);
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&format!("\n[^{number}]: 脚注内容\n"));
            Some(cursor..cursor)
        });
        self.finish_document_edit(self.session[index].id(), Some(cursor..cursor));
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
        let Some(index) = self.session.active_index() else {
            return;
        };
        let cursor = self.active_selection(index).start;
        let cursor_byte = char_to_byte(&self.session[index].content, cursor);
        let table = table::find_table(&self.session[index].content, cursor_byte)
            .unwrap_or_else(|| table::new_table(cursor_byte));
        self.table_editor = Some(TableEditorState {
            snapshot: self.session[index].snapshot(),
            table,
        });
    }

    fn close_document(&mut self, index: usize) {
        if index >= self.session.documents().len() {
            return;
        }
        if self.session[index].dirty {
            match prompt_to_save(&self.session[index].title()) {
                MessageDialogResult::Yes => {
                    self.activate_document(index);
                    self.save_active(false);
                    if self.session[index].dirty {
                        return;
                    }
                }
                MessageDialogResult::No => {}
                _ => return,
            }
        }

        let id = self.session[index].id();
        self.external_changes.forget(id);
        let was_active = self.session.active_index() == Some(index);
        self.store_active_view_state();
        self.editor_surface.forget(id);
        self.session.remove(id);
        if self.session.documents().is_empty() {
            self.new_document();
        } else if was_active {
            self.restore_active_view_state();
        }
        self.save_recovery_snapshot();
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
        let document_editing = ctx.memory(|memory| memory.focused()).is_none_or(|focused| {
            Some(focused) == self.editor_surface.widget_id()
                || TextEdit::load_state(ctx, focused).is_none()
        });
        if document_editing {
            let shortcuts = [
                (bindings.redo.as_str(), true),
                ("Ctrl+Y", true),
                (bindings.undo.as_str(), false),
            ];
            let deferred = ctx.input_mut(|input| {
                let (position, redo) =
                    input
                        .events
                        .iter()
                        .enumerate()
                        .rev()
                        .find_map(|(position, event)| {
                            let egui::Event::Key {
                                key,
                                pressed: true,
                                modifiers,
                                ..
                            } = event
                            else {
                                return None;
                            };
                            shortcuts.iter().find_map(|(binding, redo)| {
                                let shortcut = parse_shortcut(binding)?;
                                (*key == shortcut.logical_key
                                    && modifiers.matches_logically(shortcut.modifiers))
                                .then_some((position, *redo))
                            })
                        })?;
                let edits = |event: &egui::Event| {
                    matches!(
                        event,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Cut
                            | egui::Event::Ime(_)
                            | egui::Event::Key {
                                key: Key::Backspace | Key::Delete | Key::Enter | Key::Tab,
                                pressed: true,
                                ..
                            }
                    )
                };
                if input.events[..position].iter().any(edits)
                    && !input.events[position + 1..].iter().any(edits)
                {
                    input.events.remove(position);
                    Some(redo)
                } else {
                    None
                }
            });
            if let Some(redo) = deferred {
                self.editor_surface.defer_history(redo);
                return;
            }
        }
        let action = ctx.input_mut(|input| {
            if document_editing
                && (consume_shortcut(input, &bindings.redo) || consume_shortcut(input, "Ctrl+Y"))
            {
                Some(ShortcutAction::Command(AppCommand::Redo))
            } else if document_editing && consume_shortcut(input, &bindings.undo) {
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
            } else if document_editing && consume_shortcut(input, &bindings.bold) {
                Some(ShortcutAction::Command(AppCommand::Format(
                    MarkdownCommand::Bold,
                )))
            } else if document_editing && consume_shortcut(input, &bindings.italic) {
                Some(ShortcutAction::Command(AppCommand::Format(
                    MarkdownCommand::Italic,
                )))
            } else if document_editing && consume_shortcut(input, &bindings.link) {
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
                if self.state.view_mode != mode {
                    self.editor_surface.change_mode(mode);
                }
                self.state.view_mode = mode;
            }
        }
    }

    fn start_update_check(&mut self) {
        self.status = match self.background.start_update_check() {
            Ok(()) => "正在后台检查更新…".to_owned(),
            Err(error) => error,
        };
    }

    fn poll_background(&mut self) {
        for event in self.background.poll() {
            match event {
                BackgroundEvent::UpdateChecked(result) => self.apply_update_result(result),
                BackgroundEvent::ExtensionFinished(result) => self.apply_extension_result(result),
            }
        }
    }

    fn apply_update_result(&mut self, result: Result<UpdateStatus, String>) {
        match result {
            Ok(UpdateStatus::Current { latest }) => {
                self.status = format!("当前已是最新版本（{latest}）");
            }
            Ok(UpdateStatus::Available(info)) => {
                self.status = format!("发现新版本 {}，可从“帮助”菜单打开发布页", info.version);
            }
            Err(error) => {
                diagnostics::append_event("WARN", &format!("update check failed: {error}")).ok();
                self.status = format!("检查更新失败：{error}");
            }
        }
    }

    fn open_release_page(&mut self) {
        let url = self
            .background
            .available_update()
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
        if let Err(error) = self.background.extensions().ensure_template() {
            self.show_error("无法创建扩展配置", &error);
            return;
        }
        if let Err(error) = open::that(self.background.extensions().config_path()) {
            self.show_error("无法打开扩展配置", &error.to_string());
        }
    }

    fn reload_extensions(&mut self) {
        match self.background.reload_extensions() {
            Ok(()) => {
                self.status = if self.background.extensions().is_enabled() {
                    format!(
                        "已加载 {} 个扩展服务",
                        self.background.extensions().services().len()
                    )
                } else {
                    "扩展服务保持关闭".to_owned()
                };
            }
            Err(error) => self.show_error("无法加载扩展配置", &error),
        }
    }

    fn start_extension(&mut self, service_index: usize) {
        if self.background.extension_running() {
            self.status = "已有扩展服务正在运行".to_owned();
            return;
        }
        let Some(document) = self.session.active_index() else {
            self.status = "没有可交给扩展的活动文档".to_owned();
            return;
        };
        let request = self.session[document].snapshot();
        self.status = match self.background.start_extension(service_index, request) {
            Ok(name) => format!("正在运行扩展“{name}”…"),
            Err(error) => error,
        };
    }

    fn apply_extension_result(&mut self, result: Result<ExtensionJobResult, String>) {
        let job = match result {
            Err(error) => {
                diagnostics::append_event("WARN", &format!("extension failed: {error}")).ok();
                self.status = format!("扩展失败：{error}");
                return;
            }
            Ok(job) => job,
        };
        let id = job.token.document_id();
        let outcome = self
            .session
            .edit_if_current(job.token, EditKind::Other, None, |text| {
                if let Some(replacement) = job.invocation.replacement {
                    *text = replacement;
                }
                None
            });
        match outcome {
            Err(SnapshotApplyError::Closed) => {
                self.status = "扩展运行期间目标文档已关闭，已丢弃过期结果".to_owned();
            }
            Err(SnapshotApplyError::Changed) => {
                self.status = "扩展运行期间目标文档或路径已变化，已丢弃过期结果".to_owned();
            }
            Ok(changed) => {
                if changed {
                    self.finish_document_edit(id, None);
                }
                self.status = job.invocation.message.unwrap_or_else(|| {
                    if changed {
                        "扩展已更新活动文档"
                    } else {
                        "扩展已完成，没有文档修改"
                    }
                    .to_owned()
                });
            }
        }
    }

    fn finish_document_edit(&mut self, id: u64, selection: Option<std::ops::Range<usize>>) {
        let Some(index) = self.session.index_of(id) else {
            return;
        };
        self.activate_document(index);
        self.editor_surface
            .document_edited(&self.session[index], selection);
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
        let Some(document) = self.session.active_mut() else {
            return;
        };
        self.status = self
            .editor_surface
            .apply_format(document, command)
            .to_owned();
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
    }

    fn undo_active(&mut self) {
        if let Some(document) = self.session.active_mut() {
            self.status = self
                .editor_surface
                .apply_history(document, false)
                .to_owned();
        }
    }

    fn redo_active(&mut self) {
        if let Some(document) = self.session.active_mut() {
            self.status = self.editor_surface.apply_history(document, true).to_owned();
        }
    }

    fn active_selection(&self, index: usize) -> std::ops::Range<usize> {
        self.editor_surface.selection(&self.session[index])
    }

    fn queue_editor_selection(&mut self, range: std::ops::Range<usize>) {
        self.editor_surface.queue_editor_selection(range);
    }

    fn jump_to_line(&mut self, one_based_line: usize) {
        let Some(index) = self.session.active_index() else {
            return;
        };
        let char_index = editing::char_index_for_line(&self.session[index].content, one_based_line);
        self.queue_editor_selection(char_index..char_index);
        if self.state.view_mode == ViewMode::Preview {
            self.state.view_mode = ViewMode::Edit;
        }
        self.status = format!("已跳转到第 {one_based_line} 行");
    }

    fn find_match(&mut self, forward: bool) {
        let Some(index) = self.session.active_index() else {
            return;
        };
        if self.find_query.is_empty() {
            self.status = "请输入查找内容".to_owned();
            return;
        }
        let selection = self.active_selection(index);
        let found = if forward {
            editing::find_next(
                &self.session[index].content,
                &self.find_query,
                selection.end,
                self.find_match_case,
            )
        } else {
            editing::find_previous(
                &self.session[index].content,
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
        let Some(index) = self.session.active_index() else {
            return;
        };
        let selection = self.active_selection(index);
        if editing::selection_matches(
            &self.session[index].content,
            selection.clone(),
            &self.find_query,
            self.find_match_case,
        ) {
            let mut cursor = selection.clone();
            self.session[index].edit(EditKind::Replace, Some(selection.clone()), |content| {
                cursor = editing::replace_range(content, selection, &self.replace_query);
                Some(cursor.clone())
            });
            self.finish_document_edit(self.session[index].id(), Some(cursor));
            self.status = "已替换 1 处".to_owned();
        }
        self.find_match(true);
    }

    fn replace_all_matches(&mut self) {
        let Some(index) = self.session.active_index() else {
            return;
        };
        let selection_before = self.editor_surface.cursor().map(cursor_range_to_char_range);
        let mut count = 0;
        self.session[index].edit(EditKind::Replace, selection_before, |content| {
            count = editing::replace_all(
                content,
                &self.find_query,
                &self.replace_query,
                self.find_match_case,
            );
            None
        });
        if count > 0 {
            self.editor_surface.invalidate_content();
            self.editor_surface.clear_selection();
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
        let Some(index) = self.session.active_index() else {
            return;
        };
        if !path.is_file() {
            self.status = format!("资源不存在：{}", path.display());
            return;
        }

        let base = self.session[index]
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
        let mut next = selection.clone();
        self.session[index].edit(EditKind::Other, Some(selection.clone()), |content| {
            next = editing::insert_resource_link(content, selection, &label, &destination, image);
            Some(next.clone())
        });
        self.finish_document_edit(self.session[index].id(), Some(next));
        self.status = if image {
            format!("已插入图片：{}", path.display())
        } else {
            format!("已插入附件链接：{}", path.display())
        };
    }

    fn paste_clipboard_image(&mut self) {
        let Some(index) = self.session.active_index() else {
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
        let path = if let Some(base) = self.session[index]
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
        self.save_recovery_snapshot();
    }

    fn save_recovery_snapshot(&mut self) {
        self.last_recovery_write = Instant::now();
        match self.recovery_store.save(self.session.documents()) {
            Ok(()) => self.recovery_error_reported = false,
            Err(error) if !self.recovery_error_reported => {
                self.status = error;
                self.recovery_error_reported = true;
            }
            Err(_) => {}
        }
    }

    fn check_external_changes_if_due(&mut self) {
        let events = self
            .external_changes
            .scan(&mut self.session, Instant::now());
        self.apply_external_events(events);
    }

    fn apply_external_events(&mut self, events: Vec<ExternalEvent>) {
        let mut errors = Vec::new();
        let mut last_reloaded = None;
        for event in events {
            match event {
                ExternalEvent::Reloaded { document_id, path } => {
                    self.reconcile_reloaded_document(document_id);
                    last_reloaded = Some(path);
                }
                ExternalEvent::Error { error, .. } => errors.push(error),
                ExternalEvent::Conflict { .. } => {}
            }
        }
        if !errors.is_empty() {
            self.status = errors.join("；");
        } else if let Some(path) = last_reloaded {
            self.status = format!("已自动重新加载外部修改：{}", path.display());
        }
    }

    fn reconcile_reloaded_document(&mut self, id: u64) {
        self.editor_surface.forget(id);
        if self.session.active_id() == Some(id) {
            self.editor_surface
                .bind_document(self.session.active(), EditorBookmark::default());
        }
    }

    fn external_change_bar(&mut self, root: &mut Ui) {
        let Some(index) = self.session.active_index() else {
            return;
        };
        let Some(path) = self.session[index].path.clone() else {
            return;
        };
        if !self.external_changes.has_conflict(self.session[index].id()) {
            return;
        }

        let document_id = self.session[index].id();
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
                match self.external_changes.resolve(
                    &mut self.session,
                    document_id,
                    ExternalResolution::Relink(new_path.clone()),
                ) {
                    Ok(result) => {
                        let conflicts = result.conflicts;
                        self.remember_recent(new_path);
                        self.reconcile_reloaded_document(document_id);
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
            match self.session[index].external_diff() {
                Ok(diff) => self.external_diff_view = Some(diff),
                Err(error) => self.show_error("比较失败", &error),
            }
        } else if merge {
            match self.external_changes.resolve(
                &mut self.session,
                document_id,
                ExternalResolution::Merge,
            ) {
                Ok(result) => {
                    let conflicts = result.conflicts;
                    self.reconcile_reloaded_document(document_id);
                    self.status = if conflicts == 0 {
                        "已自动合并外部修改".to_owned()
                    } else {
                        format!("合并完成，存在 {conflicts} 处冲突；请搜索 <<<<<<< 并人工处理")
                    };
                }
                Err(error) => self.show_error("合并失败", &error),
            }
        } else if reload {
            let confirmed = !self.session[index].dirty
                || MessageDialog::new()
                    .set_level(MessageLevel::Warning)
                    .set_title("重新加载外部版本")
                    .set_description("这会丢弃 RUPORA 中尚未保存的修改。确定继续吗？")
                    .set_buttons(MessageButtons::YesNo)
                    .show()
                    == MessageDialogResult::Yes;
            if confirmed {
                match self.external_changes.resolve(
                    &mut self.session,
                    document_id,
                    ExternalResolution::Reload,
                ) {
                    Ok(_) => {
                        self.reconcile_reloaded_document(document_id);
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
        let token = state.snapshot.token();
        let stale = self
            .session
            .get(token.document_id())
            .is_none_or(|document| document.snapshot_token() != token);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("可视化表格编辑器")
            .id(egui::Id::new("table-editor"))
            .default_size([760.0, 420.0])
            .open(&mut open)
            .show(root.ctx(), |ui| {
                if stale {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "目标文档已变化。表格草稿已保留，可复制后重新打开编辑器。",
                    );
                }
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
                    if ui.button("复制表格 Markdown").clicked() {
                        ui.ctx().copy_text(state.table.to_markdown());
                    }
                });
            });

        if apply {
            self.apply_table_editor();
        } else if !open || cancel {
            self.table_editor = None;
        }
    }

    fn apply_table_editor(&mut self) {
        let Some(state) = self.table_editor.take() else {
            return;
        };
        let before = state.snapshot.text();
        let range = state.table.range.clone();
        if range.start > range.end
            || range.end > before.len()
            || !before.is_char_boundary(range.start)
            || !before.is_char_boundary(range.end)
        {
            self.status = "表格范围已失效，请重新打开表格编辑器。".to_owned();
            self.table_editor = Some(state);
            return;
        }
        let mut replacement = state.table.to_markdown();
        if range.is_empty() {
            if range.start > 0 && !before[..range.start].ends_with('\n') {
                replacement.insert_str(0, "\n\n");
            }
            if range.start < before.len() && !before[range.start..].starts_with('\n') {
                replacement.push_str("\n\n");
            }
        }
        let cursor = before[..range.start].chars().count() + replacement.chars().count();
        let token = state.snapshot.token();
        match self
            .session
            .edit_if_current(token, EditKind::Format, None, |text| {
                text.replace_range(range, &replacement);
                Some(cursor..cursor)
            }) {
            Ok(_) => {
                self.finish_document_edit(token.document_id(), Some(cursor..cursor));
                self.status = "已应用可视化表格修改".to_owned();
            }
            Err(SnapshotApplyError::Closed) => {
                self.status = "目标文档已关闭，请重新打开表格编辑器。".to_owned();
                self.table_editor = Some(state);
            }
            Err(SnapshotApplyError::Changed) => {
                self.status = "文档或路径已发生变化，请重新打开表格编辑器。".to_owned();
                self.table_editor = Some(state);
            }
        }
    }

    fn navigation_rail(&mut self, root: &mut Ui) {
        let palette = app_palette(self.state.dark);
        Panel::left("workspace-rail")
            .exact_size(64.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(12, 14)),
            )
            .show(root, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let (brand, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                ui.painter().text(
                    brand.center(),
                    egui::Align2::CENTER_CENTER,
                    "R",
                    egui::FontId::proportional(27.0),
                    palette.text,
                );
                ui.add_space(18.0);
                let rail_button = |ui: &mut Ui, icon, selected, tooltip: &str| {
                    ui.add(AppIconButton {
                        icon,
                        selected,
                        palette,
                        size: 40.0,
                    })
                    .on_hover_text(tooltip)
                };
                if rail_button(
                    ui,
                    AppIcon::Sidebar,
                    self.state.show_sidebar,
                    "文稿架：显示 / 隐藏文稿与文件夹",
                )
                .clicked()
                {
                    self.state.show_sidebar = !self.state.show_sidebar;
                }
                if rail_button(ui, AppIcon::Search, self.find_open, "查找文稿内容 · Ctrl+F")
                    .clicked()
                {
                    self.find_open = !self.find_open;
                    self.find_focus_requested = self.find_open;
                }
                ui.add_space(8.0);
                if rail_button(ui, AppIcon::New, false, "新建文稿 · Ctrl+N").clicked() {
                    self.execute(AppCommand::New);
                }
                if rail_button(ui, AppIcon::Folder, false, "打开 Markdown 文件 · Ctrl+O").clicked()
                {
                    self.execute(AppCommand::Open);
                }
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if rail_button(ui, AppIcon::Theme, false, "切换浅色 / 深色外观").clicked()
                    {
                        self.state.dark = !self.state.dark;
                        apply_theme(ui.ctx(), self.state.dark);
                    }
                });
            });
    }

    fn top_bar(&mut self, root: &mut Ui) {
        let palette = app_palette(self.state.dark);
        let document = self
            .session
            .active_index()
            .map(|index| &self.session[index]);
        let title = document
            .map(Document::title)
            .unwrap_or_else(|| "RUPORA".to_owned());
        let identity = document
            .map(|document| {
                if document.dirty {
                    "有未保存的修改".to_owned()
                } else if let Some(path) = document.path.as_deref() {
                    path.parent()
                        .map(|parent| parent.display().to_string())
                        .unwrap_or_else(|| "已保存".to_owned())
                } else {
                    "开始新的文稿".to_owned()
                }
            })
            .unwrap_or_else(|| "你的写作空间".to_owned());
        let document_exists = document.is_some();
        Panel::top("toolbar")
            .exact_size(76.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(24, 14)),
            )
            .show(root, |ui| {
                let labelled_modes = ui.available_width() >= 720.0;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.more_menu(ui);
                    if ui
                        .add_enabled(
                            document_exists,
                            icon_button_widget(AppIcon::Save, false, palette),
                        )
                        .on_hover_text("保存当前文稿 · Ctrl+S")
                        .clicked()
                    {
                        self.execute(AppCommand::Save);
                    }
                    if app_icon_button(
                        ui,
                        AppIcon::Outline,
                        self.state.show_outline,
                        "显示 / 隐藏文稿大纲",
                        palette,
                    )
                    .clicked()
                    {
                        self.state.show_outline = !self.state.show_outline;
                    }
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(palette.canvas)
                        .corner_radius(8)
                        .inner_margin(3)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for (mode, icon, label) in [
                                    (ViewMode::Preview, AppIcon::Read, "阅读"),
                                    (ViewMode::Split, AppIcon::Split, "分栏"),
                                    (ViewMode::Edit, AppIcon::Source, "源码"),
                                    (ViewMode::Hybrid, AppIcon::Write, "写作"),
                                ] {
                                    let selected = self.state.view_mode == mode;
                                    let response = if labelled_modes {
                                        app_action_button(ui, icon, label, selected, palette, 76.0)
                                    } else {
                                        app_icon_button(ui, icon, selected, label, palette)
                                    };
                                    if response
                                        .on_hover_text(format!("切换到{label}视图"))
                                        .clicked()
                                    {
                                        self.execute(AppCommand::SetView(mode));
                                    }
                                }
                            });
                        });
                    ui.add_space(12.0);
                    let identity_width = ui.available_width().max(32.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(identity_width, 46.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_min_size(Vec2::new(identity_width, 46.0));
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&title)
                                        .size(17.0)
                                        .strong()
                                        .color(palette.text),
                                )
                                .halign(Align::Min)
                                .truncate(),
                            )
                            .on_hover_text(&title);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&identity).size(12.0).color(palette.secondary),
                                )
                                .halign(Align::Min)
                                .truncate(),
                            )
                            .on_hover_text(&identity);
                        },
                    );
                });
            });
    }

    fn more_menu(&mut self, ui: &mut Ui) {
        let extension_names = self
            .background
            .extensions()
            .services()
            .iter()
            .map(|service| service.name.clone())
            .collect::<Vec<_>>();
        let palette = app_palette(self.state.dark);
        let response = app_icon_button(ui, AppIcon::More, false, "更多操作与设置", palette);
        egui::Popup::menu(&response).width(248.0).show(|ui| {
            Self::compact_menu_contents(ui, "more-menu-scroll", |ui| {
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
                    .session
                    .active_index()
                    .and_then(|index| self.session.documents().get(index))
                    .is_some_and(Document::can_undo);
                if ui
                    .add_enabled(can_undo, Button::new("撤销"))
                    .on_hover_text("Ctrl+Z")
                    .clicked()
                {
                    self.execute(AppCommand::Undo);
                }
                let can_redo = self
                    .session
                    .active_index()
                    .and_then(|index| self.session.documents().get(index))
                    .is_some_and(Document::can_redo);
                if ui
                    .add_enabled(can_redo, Button::new("重做"))
                    .on_hover_text("Ctrl+Shift+Z / Ctrl+Y")
                    .clicked()
                {
                    self.execute(AppCommand::Redo);
                }
                ui.menu_button("导出", |ui| {
                    Self::compact_menu_contents(ui, "export-menu-scroll", |ui| {
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
                });
                ui.menu_button("扩展", |ui| {
                    Self::compact_menu_contents(ui, "extensions-menu-scroll", |ui| {
                        if !self.background.extensions().is_enabled() {
                            ui.label("扩展默认关闭");
                        } else if extension_names.is_empty() {
                            ui.label("没有已配置的扩展服务");
                        }
                        for (index, name) in extension_names.iter().enumerate() {
                            if ui
                                .add_enabled(
                                    !self.background.extension_running()
                                        && self.session.active_index().is_some(),
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
                });
                ui.menu_button("帮助", |ui| {
                    Self::compact_menu_contents(ui, "help-menu-scroll", |ui| {
                        if ui
                            .add_enabled(
                                !self.background.update_check_running(),
                                Button::new("检查更新…"),
                            )
                            .clicked()
                        {
                            self.execute(AppCommand::CheckUpdates);
                            ui.close();
                        }
                        if self.background.available_update().is_some()
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
                });

                ui.separator();
                ui.menu_button("格式", |ui| {
                    Self::compact_menu_contents(ui, "format-menu-scroll", |ui| {
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
                            .session
                            .active_index()
                            .map(|index| markdown::heading_anchors(&self.session[index].content))
                            .unwrap_or_default();
                        ui.menu_button("交叉引用", |ui| {
                            Self::compact_menu_contents(ui, "references-menu-scroll", |ui| {
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
                ui.menu_button("外观与导航", |ui| {
                    Self::compact_menu_contents(ui, "appearance-menu-scroll", |ui| {
                        let theme_label = if self.state.dark { "浅色" } else { "深色" };
                        if ui.button(theme_label).clicked() {
                            self.state.dark = !self.state.dark;
                            apply_theme(ui.ctx(), self.state.dark);
                        }
                        ui.checkbox(&mut self.state.show_outline, "大纲");
                        ui.checkbox(&mut self.state.show_sidebar, "文稿架");
                    });
                });
            });
        });
    }

    /// Menus keep their own width and scrolling budget instead of inheriting
    /// the application's horizontal toolbar layout or the popup's area width.
    fn compact_menu_contents<R>(
        ui: &mut Ui,
        scroll_id: &'static str,
        contents: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        const MENU_WIDTH: f32 = 248.0;
        ui.set_width(MENU_WIDTH);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 2.0);
        ui.spacing_mut().interact_size.y = 26.0;
        let maximum_height = (ui.ctx().content_rect().height() - 140.0).clamp(120.0, 440.0);
        ScrollArea::vertical()
            .id_salt(scroll_id)
            .max_height(maximum_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_max_width(MENU_WIDTH);
                contents(ui)
            })
            .inner
    }

    fn find_bar(&mut self, root: &mut Ui) {
        if !self.find_open {
            return;
        }
        let palette = app_palette(self.state.dark);
        Panel::top("find-and-replace")
            .frame(
                egui::Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(20, 10)),
            )
            .show(root, |ui| {
                let field_width = (ui.available_width() - 278.0).clamp(110.0, 360.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("查找").size(13.0));
                    let response = ui.add_sized(
                        [field_width, 30.0],
                        TextEdit::singleline(&mut self.find_query).hint_text("查找内容"),
                    );
                    if self.find_focus_requested {
                        self.editor_surface.cancel_focus_request();
                        response.request_focus();
                        self.find_focus_requested = false;
                    }
                    if response.lost_focus()
                        && ui.input_mut(|input| input.consume_key(input.modifiers, Key::Enter))
                    {
                        self.find_match(!ui.input(|input| input.modifiers.shift));
                    }
                    if app_icon_button(
                        ui,
                        AppIcon::ArrowUp,
                        false,
                        "上一个匹配项 · Shift+Enter",
                        palette,
                    )
                    .clicked()
                    {
                        self.find_match(false);
                    }
                    if app_icon_button(
                        ui,
                        AppIcon::ArrowDown,
                        false,
                        "下一个匹配项 · Enter",
                        palette,
                    )
                    .clicked()
                    {
                        self.find_match(true);
                    }
                    ui.checkbox(&mut self.find_match_case, "区分大小写");
                    if app_icon_button(ui, AppIcon::Close, false, "关闭查找", palette).clicked()
                    {
                        self.find_open = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("替换").size(13.0));
                    ui.add_sized(
                        [field_width, 30.0],
                        TextEdit::singleline(&mut self.replace_query).hint_text("替换为"),
                    );
                    if ui.add_sized([64.0, 30.0], Button::new("替换")).clicked() {
                        self.replace_current();
                    }
                    if ui
                        .add_sized([88.0, 30.0], Button::new("全部替换"))
                        .clicked()
                    {
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
        let available_update = self.background.available_update().cloned();
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
                    self.editor_surface.cancel_focus_request();
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
                if (response.has_focus() || response.lost_focus())
                    && ui.input_mut(|input| input.consume_key(input.modifiers, Key::Enter))
                {
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
        let palette = app_palette(self.state.dark);
        let editor_width = if self.state.view_mode == ViewMode::Split {
            400.0
        } else {
            380.0
        };
        let outline_width = if self.state.show_outline { 160.0 } else { 0.0 };
        let maximum_width =
            (root.available_width() - editor_width - outline_width).clamp(180.0, 320.0);
        let mut activate = None;
        let mut close = None;
        let mut open_path = None;
        let mut refresh_workspace = false;
        let mut close_workspace = false;
        let mut open_folder = false;
        Panel::left("documents")
            .default_size(220.0)
            .size_range(180.0..=maximum_width)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(14, 22)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("文稿")
                            .size(20.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(self.session.documents().len().to_string())
                            .size(12.0)
                            .color(palette.secondary),
                    );
                });
                ui.add_space(18.0);
                if app_action_button(
                    ui,
                    AppIcon::Write,
                    "新建文稿",
                    true,
                    palette,
                    ui.available_width(),
                )
                .on_hover_text("Ctrl+N")
                .clicked()
                {
                    self.new_document();
                }
                ui.add_space(20.0);
                ScrollArea::vertical()
                    .id_salt("document-shelf")
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("打开的文稿")
                                .size(12.0)
                                .color(palette.secondary),
                        );
                        ui.add_space(8.0);
                        for (index, document) in self.session.documents().iter().enumerate() {
                            let selected = self.session.active_index() == Some(index);
                            let row = egui::Frame::new()
                                .fill(if selected {
                                    palette.surface
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .corner_radius(6)
                                .inner_margin(Margin::symmetric(6, 5))
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 5.0;
                                    ui.horizontal(|ui| {
                                        let (marker, _) = ui.allocate_exact_size(
                                            Vec2::new(18.0, 28.0),
                                            egui::Sense::hover(),
                                        );
                                        if document.dirty {
                                            ui.painter().circle_filled(
                                                marker.center(),
                                                3.0,
                                                palette.accent,
                                            );
                                        } else {
                                            paint_app_icon(
                                                ui.painter(),
                                                egui::Rect::from_center_size(
                                                    marker.center(),
                                                    Vec2::splat(16.0),
                                                ),
                                                AppIcon::File,
                                                if selected {
                                                    palette.accent
                                                } else {
                                                    palette.secondary
                                                },
                                            );
                                        }
                                        let title = document.title();
                                        let title_width = (ui.available_width() - 29.0).max(24.0);
                                        let response = ui
                                            .add_sized(
                                                [title_width, 28.0],
                                                Button::new("")
                                                    .left_text(
                                                        RichText::new(&title)
                                                            .size(13.0)
                                                            .color(palette.text),
                                                    )
                                                    .frame(false)
                                                    .truncate(),
                                            )
                                            .on_hover_text(format!(
                                                "{}\n{}",
                                                title,
                                                document
                                                    .path
                                                    .as_deref()
                                                    .map(Path::display)
                                                    .map(|display| display.to_string())
                                                    .unwrap_or_else(|| "尚未保存".to_owned())
                                            ));
                                        response.widget_info(|| {
                                            egui::WidgetInfo::selected(
                                                egui::WidgetType::Button,
                                                ui.is_enabled(),
                                                selected,
                                                &title,
                                            )
                                        });
                                        if response.clicked() {
                                            activate = Some(index);
                                        }
                                        if ui
                                            .add(AppIconButton {
                                                icon: AppIcon::Close,
                                                selected: false,
                                                palette,
                                                size: 24.0,
                                            })
                                            .on_hover_text(format!("关闭 {title}"))
                                            .clicked()
                                        {
                                            close = Some(index);
                                        }
                                    });
                                });
                            if selected {
                                let rect = row.response.rect;
                                ui.painter().rect_filled(
                                    egui::Rect::from_center_size(
                                        egui::pos2(rect.left(), rect.center().y),
                                        Vec2::new(2.0, 18.0),
                                    ),
                                    1.0,
                                    palette.accent,
                                );
                            }
                            ui.add_space(2.0);
                        }
                        ui.add_space(22.0);
                        ui.label(RichText::new("文件夹").size(12.0).color(palette.secondary));
                        ui.add_space(8.0);
                        if let Some(workspace) = self.workspace.as_ref() {
                            ui.horizontal(|ui| {
                                let root_name = workspace
                                    .root
                                    .file_name()
                                    .map(|name| name.to_string_lossy())
                                    .unwrap_or_else(|| {
                                        workspace.root.as_os_str().to_string_lossy()
                                    });
                                ui.add_sized(
                                    [(ui.available_width() - 61.0).max(24.0), 28.0],
                                    egui::Label::new(RichText::new(root_name).strong().size(13.0))
                                        .halign(Align::Min)
                                        .truncate(),
                                )
                                .on_hover_text(workspace.root.display().to_string());
                                if ui
                                    .add(AppIconButton {
                                        icon: AppIcon::Refresh,
                                        selected: false,
                                        palette,
                                        size: 24.0,
                                    })
                                    .on_hover_text("刷新工作区")
                                    .clicked()
                                {
                                    refresh_workspace = true;
                                }
                                if ui
                                    .add(AppIconButton {
                                        icon: AppIcon::Close,
                                        selected: false,
                                        palette,
                                        size: 24.0,
                                    })
                                    .on_hover_text("关闭工作区")
                                    .clicked()
                                {
                                    close_workspace = true;
                                }
                            });
                            let active_path = self
                                .session
                                .active_index()
                                .and_then(|index| self.session[index].path.as_deref());
                            open_path = workspace_entries_ui(ui, &workspace.entries, active_path);
                        } else if app_action_button(
                            ui,
                            AppIcon::Folder,
                            "打开文件夹",
                            false,
                            palette,
                            ui.available_width(),
                        )
                        .on_hover_text("Ctrl+Shift+O")
                        .clicked()
                        {
                            open_folder = true;
                        }
                        if !self.state.recent_files.is_empty() {
                            ui.add_space(22.0);
                            ui.label(
                                RichText::new("最近打开")
                                    .size(12.0)
                                    .color(palette.secondary),
                            );
                            ui.add_space(8.0);
                            for path in self.state.recent_files.iter().take(6) {
                                let name = path
                                    .file_name()
                                    .map(|name| name.to_string_lossy())
                                    .unwrap_or_else(|| path.as_os_str().to_string_lossy());
                                if app_action_button(
                                    ui,
                                    AppIcon::File,
                                    &name,
                                    false,
                                    palette,
                                    ui.available_width(),
                                )
                                .on_hover_text(path.display().to_string())
                                .clicked()
                                {
                                    open_path = Some(path.clone());
                                }
                            }
                        }
                    });
            });
        if let Some(index) = activate {
            self.activate_document(index);
        }
        if let Some(index) = close {
            self.close_document(index);
        }
        if close_workspace {
            self.workspace = None;
            self.state.workspace_root = None;
            self.status = "已关闭工作区".to_owned();
        } else if refresh_workspace && let Some(workspace) = self.workspace.as_mut() {
            self.status = match workspace.refresh() {
                Ok(()) => "工作区已刷新".to_owned(),
                Err(error) => error,
            };
        }
        if open_folder {
            self.execute(AppCommand::OpenFolder);
        }
        if let Some(path) = open_path {
            self.open_paths([path]);
        }
    }

    fn outline(&mut self, root: &mut Ui) {
        if !self.state.show_outline {
            return;
        }
        let headings = self
            .session
            .active_index()
            .and_then(|index| self.session.documents().get(index))
            .map(|document| document.analysis.headings.clone())
            .unwrap_or_default();

        let mut jump_to_line = None;
        let palette = app_palette(self.state.dark);
        let editor_width = if self.state.view_mode == ViewMode::Split {
            380.0
        } else {
            340.0
        };
        let maximum_width = (root.available_width() - editor_width).clamp(160.0, 300.0);
        Panel::right("outline")
            .default_size(200.0)
            .size_range(160.0..=maximum_width)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(16, 18))
                    .stroke(Stroke::new(0.5, palette.border)),
            )
            .show(root, |ui| {
                ui.label(
                    RichText::new("大纲")
                        .size(13.0)
                        .strong()
                        .color(palette.text),
                );
                ui.add_space(4.0);
                ui.add_space(12.0);
                if headings.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new("文档结构将在这里显示").color(palette.secondary));
                    ui.label(
                        RichText::new("在正文中添加标题，即可快速跳转。")
                            .small()
                            .color(palette.secondary),
                    );
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
        if self.session.documents().len() <= 1 {
            return;
        }
        let tabs = self
            .session
            .documents()
            .iter()
            .map(|document| (document.title(), document.dirty))
            .collect::<Vec<_>>();
        let palette = app_palette(self.state.dark);
        let mut activate = None;
        let mut close = None;
        Panel::top("editor-tabs")
            .exact_size(44.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(14, 3)),
            )
            .show(root, |ui| {
                let reveal_key = ui.id().with("visible-active-tab");
                let active_layout = (self.session.active_id(), ui.available_width().to_bits());
                let reveal_active = ui.data_mut(|data| {
                    let previous = data.get_temp::<(Option<u64>, u32)>(reveal_key);
                    data.insert_temp(reveal_key, active_layout);
                    previous != Some(active_layout)
                });
                let tab_width = (ui.available_width() * 0.3).clamp(160.0, 240.0);
                ScrollArea::horizontal()
                    .id_salt("editor-tabs-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (index, (title, dirty)) in tabs.iter().enumerate() {
                                let selected = self.session.active_index() == Some(index);
                                let tab =
                                    document_tab(ui, title, *dirty, selected, palette, tab_width);
                                if tab.activate {
                                    activate = Some(index);
                                }
                                if tab.close {
                                    close = Some(index);
                                }
                                if selected && reveal_active {
                                    tab.response.scroll_to_me(Some(Align::Center));
                                }
                            }
                            ui.add_space(5.0);
                            if app_icon_button(
                                ui,
                                AppIcon::New,
                                false,
                                "新建文稿 · Ctrl+N",
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
            .frame(egui::Frame::new().fill(palette.surface))
            .show(root, |ui| {
                let Some(index) = self.session.active_index() else {
                    let gutter = (ui.available_width() * 0.12).clamp(24.0, 96.0);
                    ui.add_space((ui.available_height() * 0.18).min(100.0));
                    ui.horizontal(|ui| {
                        ui.add_space(gutter);
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - gutter).max(100.0));
                            ui.label(
                                RichText::new("留一页给想法")
                                    .size(30.0)
                                    .strong()
                                    .color(palette.text),
                            );
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new("从一段文字开始，或打开已有的文稿。")
                                    .size(15.0)
                                    .color(palette.secondary),
                            );
                            ui.add_space(28.0);
                            if app_action_button(
                                ui,
                                AppIcon::Write,
                                "新建文稿",
                                true,
                                palette,
                                180.0,
                            )
                            .on_hover_text("Ctrl+N")
                            .clicked()
                            {
                                self.execute(AppCommand::New);
                            }
                            if app_action_button(
                                ui,
                                AppIcon::Folder,
                                "打开 Markdown 文件",
                                false,
                                palette,
                                220.0,
                            )
                            .on_hover_text("Ctrl+O")
                            .clicked()
                            {
                                self.execute(AppCommand::Open);
                            }
                            ui.add_space(24.0);
                            ui.label(
                                RichText::new("Ctrl+N 新建   Ctrl+O 打开   Ctrl+Shift+P 命令")
                                    .size(12.0)
                                    .color(palette.secondary),
                            );
                        });
                    });
                    return;
                };
                self.show_editor_pane(ui, index, self.state.view_mode);
            });
    }

    fn show_editor_pane(&mut self, ui: &mut Ui, index: usize, mode: ViewMode) {
        let base_path = self.preview_base_path(index);
        let output = self.editor_surface.show(
            ui,
            &mut self.session[index],
            EditorOptions {
                mode,
                dark: self.state.dark,
                base_path: &base_path,
            },
        );
        if !output.notice.is_empty() {
            self.status = output.notice;
        }
        if let Some(destination) = output.destination {
            self.open_preview_destination(index, &destination);
        }
        for command in output.commands {
            self.execute(match command {
                EditorCommand::PasteImage => AppCommand::PasteImage,
                EditorCommand::EditTable => AppCommand::EditTable,
            });
        }
    }

    #[cfg(test)]
    fn hybrid_pane(&mut self, ui: &mut Ui, index: usize) {
        self.show_editor_pane(ui, index, ViewMode::Hybrid);
    }
    #[cfg(test)]
    fn edit_pane(&mut self, ui: &mut Ui, index: usize, _offset: Option<f32>) {
        self.show_editor_pane(ui, index, ViewMode::Edit);
    }
    #[cfg(test)]
    fn preview_pane(&mut self, ui: &mut Ui, index: usize, _offset: Option<f32>) {
        self.show_editor_pane(ui, index, ViewMode::Preview);
    }

    fn preview_base_path(&self, index: usize) -> PathBuf {
        self.session[index]
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

    fn open_preview_destination(&mut self, index: usize, destination: &str) {
        if destination.starts_with('#') {
            let anchor = decode_uri_fragment(destination.trim_start_matches('#'))
                .unwrap_or_else(|| destination.trim_start_matches('#').to_owned());
            self.jump_to_anchor(index, &anchor);
            return;
        }
        let path_part = destination.split(['#', '?']).next().unwrap_or(destination);
        if markdown::is_local_link_destination(destination) || Path::new(path_part).is_absolute() {
            self.open_local_preview_link(index, destination);
        } else if let Err(error) = open::that(destination) {
            self.status = format!("无法打开链接 {destination}：{error}");
        }
    }

    fn open_local_preview_link(&mut self, index: usize, destination: &str) {
        let fragment = destination
            .split_once('#')
            .and_then(|(_, fragment)| decode_uri_fragment(fragment));
        let Some(path_part) = decode_local_resource_path(destination) else {
            self.status = format!("本地链接路径编码无效：{destination}");
            return;
        };
        let link_path = PathBuf::from(path_part);
        let base = self.session[index]
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
            let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
            self.open_paths([resolved]);
            if let Some(fragment) = fragment
                && let Some(target_index) = self.session.documents().iter().position(|document| {
                    document.path.as_ref().is_some_and(|path| {
                        path.canonicalize().unwrap_or_else(|_| path.clone()) == canonical
                    })
                })
            {
                self.jump_to_anchor(target_index, &fragment);
            }
        } else if let Err(error) = open::that(&resolved) {
            self.status = format!("无法打开 {}：{error}", resolved.display());
        }
    }

    fn jump_to_anchor(&mut self, index: usize, anchor: &str) {
        let target = markdown::heading_anchors(&self.session[index].content)
            .into_iter()
            .find(|heading| heading.id == anchor);
        if let Some(target) = target {
            self.activate_document(index);
            let source = &self.session[index].content;
            let byte = line_start_byte(source, target.heading.line);
            let cursor = source[..byte].chars().count();
            self.queue_editor_selection(cursor..cursor);
            self.status = format!("已定位到：{}", target.heading.text);
        } else {
            self.status = format!("找不到文档内锚点：#{anchor}");
        }
    }

    fn status_bar(&mut self, root: &mut Ui) {
        let document_info = self
            .session
            .active_index()
            .and_then(|index| self.session.documents().get(index))
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
            .exact_size(30.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(24, 2)),
            )
            .show(root, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(AppIconButton {
                            icon: AppIcon::Source,
                            selected: source_mode,
                            palette,
                            size: 24.0,
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
                    let info_width = (ui.available_width() * 0.48).min(360.0);
                    ui.add_sized(
                        [info_width, 24.0],
                        egui::Label::new(
                            RichText::new(&document_info)
                                .size(12.0)
                                .color(palette.secondary),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&document_info);
                    let status_width = ui.available_width().max(0.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(status_width, 24.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.set_min_width(status_width);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.status)
                                        .size(12.0)
                                        .color(palette.secondary),
                                )
                                .halign(Align::Min)
                                .truncate(),
                            )
                            .on_hover_text(&self.status);
                        },
                    );
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
        if !self
            .session
            .documents()
            .iter()
            .any(|document| document.dirty)
        {
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
        for document in self.session.documents_mut() {
            document.refresh_derived_state_if_idle(Duration::from_millis(120));
            if document.derived_state_is_stale() {
                ctx.request_repaint_after(Duration::from_millis(40));
            }
        }
        self.poll_instance_requests(ctx);
        self.poll_background();
        self.handle_shortcuts(ctx);
        self.handle_dropped_files(ctx);
        self.save_recovery_snapshot_if_due();
        self.check_external_changes_if_due();
        self.confirm_application_close(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        self.navigation_rail(ui);
        self.sidebar(ui);
        self.top_bar(ui);
        self.find_bar(ui);
        self.command_palette(ui);
        self.about_window(ui);
        self.shortcut_settings(ui);
        self.external_diff_window(ui);
        self.table_editor_window(ui);
        self.external_change_bar(ui);
        self.status_bar(ui);
        self.outline(ui);
        self.editor_tabs(ui);
        self.editor(ui);
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        self.store_active_view_state();
        self.state.session_files = self
            .session
            .documents()
            .iter()
            .filter_map(|document| document.path.clone())
            .collect();
        self.state.active_session_file = self
            .session
            .active_index()
            .and_then(|index| self.session.documents().get(index))
            .and_then(|document| document.path.clone());
        eframe::set_value(storage, APP_STATE_KEY, &self.state);
        eframe::set_value(storage, UI_EXPERIENCE_KEY, &CURRENT_UI_EXPERIENCE);
        let _ = self.recovery_store.save(self.session.documents());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.discard_recovery_on_exit
            || self
                .session
                .documents()
                .iter()
                .all(|document| !document.dirty)
        {
            let _ = self.recovery_store.clear();
        } else {
            let _ = self.recovery_store.save(self.session.documents());
        }
        diagnostics::append_event("INFO", "RUPORA exited normally").ok();
    }
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

fn workspace_entries_ui(
    ui: &mut Ui,
    entries: &[WorkspaceEntry],
    active_path: Option<&Path>,
) -> Option<PathBuf> {
    let mut selected = None;
    for entry in entries {
        if entry.is_dir {
            let mut job = egui::text::LayoutJob::simple_singleline(
                entry.name.clone(),
                egui::FontId::proportional(13.0),
                ui.visuals().text_color(),
            );
            job.wrap.max_width =
                (ui.available_width() - ui.spacing().indent - 2.0 * ui.spacing().button_padding.x)
                    .max(1.0);
            job.wrap.max_rows = 1;
            let label = ui.fonts_mut(|fonts| fonts.layout_job(job));
            let response = egui::CollapsingHeader::new(label)
                .id_salt(&entry.path)
                .default_open(false)
                .icon(|ui, openness, response| {
                    paint_app_icon(
                        ui.painter(),
                        response.rect.shrink(1.0),
                        if openness > 0.5 {
                            AppIcon::ChevronDown
                        } else {
                            AppIcon::ChevronRight
                        },
                        ui.visuals().weak_text_color(),
                    );
                })
                .show(ui, |ui| {
                    workspace_entries_ui(ui, &entry.children, active_path)
                });
            response
                .header_response
                .on_hover_text(entry.path.display().to_string());
            if let Some(path) = response.body_returned.flatten() {
                selected = Some(path);
            }
        } else if ui
            .add_sized(
                [ui.available_width(), 28.0],
                Button::new("")
                    .left_text(&entry.name)
                    .selected(active_path == Some(entry.path.as_path()))
                    .frame(false)
                    .truncate(),
            )
            .on_hover_text(entry.path.display().to_string())
            .clicked()
        {
            selected = Some(entry.path.clone());
        }
    }
    selected
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

fn decode_uri_fragment(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = uri_hex_value(*bytes.get(index + 1)?)?;
            let low = uri_hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

const fn uri_hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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

fn line_start_byte(text: &str, one_based_line: usize) -> usize {
    if one_based_line <= 1 {
        return 0;
    }
    let mut line = 1usize;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            line += 1;
            if line == one_based_line {
                return index + 1;
            }
        }
    }
    text.len()
}

fn next_footnote_number(source: &str) -> usize {
    let mut used = HashSet::new();
    let bytes = source.as_bytes();
    for (index, _) in source.match_indices("[^") {
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
    }
    (1..).find(|number| !used.contains(number)).unwrap_or(1)
}

#[cfg(test)]
#[path = "product_input_tests.rs"]
mod product_input_tests;

#[cfg(test)]
#[path = "product_document_tests.rs"]
mod product_document_tests;

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
