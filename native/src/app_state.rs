use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::editing::MarkdownCommand;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ViewMode {
    Edit,
    #[default]
    Split,
    Hybrid,
    Preview,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedState {
    pub(crate) dark: bool,
    pub(crate) show_sidebar: bool,
    pub(crate) show_outline: bool,
    pub(crate) view_mode: ViewMode,
    pub(crate) recent_files: Vec<PathBuf>,
    pub(crate) session_files: Vec<PathBuf>,
    pub(crate) active_session_file: Option<PathBuf>,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) cursor_positions: HashMap<PathBuf, usize>,
    pub(crate) scroll_positions: HashMap<PathBuf, f32>,
    pub(crate) key_bindings: KeyBindings,
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
pub(crate) struct KeyBindings {
    pub(crate) new_document: String,
    pub(crate) open_file: String,
    pub(crate) open_folder: String,
    pub(crate) save: String,
    pub(crate) save_as: String,
    pub(crate) undo: String,
    pub(crate) redo: String,
    pub(crate) find: String,
    pub(crate) replace: String,
    pub(crate) command_palette: String,
    pub(crate) bold: String,
    pub(crate) italic: String,
    pub(crate) link: String,
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
pub(crate) enum AppCommand {
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
    CheckUpdates,
    OpenDiagnostics,
    OpenExtensionConfig,
    ReloadExtensions,
    RunExtension(usize),
    OpenReleasePage,
    About,
    Format(MarkdownCommand),
    SetView(ViewMode),
}

pub(crate) enum ShortcutAction {
    Command(AppCommand),
    Find,
    Replace,
    Palette,
}
