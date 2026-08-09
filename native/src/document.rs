use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{Read, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime},
};

use encoding_rs::{Encoding, GB18030, GBK};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

use crate::{
    markdown::{BlockIndex, MarkdownAnalysis, MarkdownBlock, analyze},
    merge,
};

const MAX_HISTORY_ENTRIES: usize = 256;
const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;
const TYPING_COALESCE_WINDOW: Duration = Duration::from_millis(900);
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Format,
    Replace,
    TaskList,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryOutcome {
    pub selection: Option<Range<usize>>,
}

#[derive(Debug)]
pub struct RecoveryOutcome {
    pub document: Document,
    pub conflicts: usize,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Legacy(&'static Encoding),
}

impl TextEncoding {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Legacy(encoding) => encoding.name(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
        }
    }

    fn apply(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Lf => Cow::Borrowed(text),
            Self::CrLf => Cow::Owned(text.replace('\n', "\r\n")),
            Self::Cr => Cow::Owned(text.replace('\n', "\r")),
        }
    }
}

#[derive(Debug)]
pub struct Document {
    id: u64,
    pub path: Option<PathBuf>,
    pub content: String,
    pub encoding: TextEncoding,
    pub line_ending: LineEnding,
    pub dirty: bool,
    pub analysis: MarkdownAnalysis,
    untitled_id: usize,
    saved_content: String,
    file_fingerprint: Option<FileFingerprint>,
    undo_history: Vec<EditTransaction>,
    redo_history: Vec<EditTransaction>,
    block_index: BlockIndex,
    derived_state_stale: bool,
    last_content_edit: Option<Instant>,
    lock: Option<DocumentLock>,
}

impl Document {
    pub fn untitled(id: usize) -> Self {
        let content = String::new();
        let block_index = BlockIndex::new(&content);
        Self {
            id: next_document_id(),
            path: None,
            analysis: analyze(&content),
            content,
            encoding: TextEncoding::Utf8,
            line_ending: LineEnding::default(),
            dirty: false,
            untitled_id: id,
            saved_content: String::new(),
            file_fingerprint: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            block_index,
            derived_state_stale: false,
            last_content_edit: None,
            lock: None,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = canonical_document_path(path.as_ref())?;
        let lock = DocumentLock::acquire(&path)?;
        Self::open_with_lock(&path, Some(lock))
    }

    fn open_unlocked(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = canonical_document_path(path.as_ref())?;
        Self::open_with_lock(&path, None)
    }

    fn open_with_lock(path: &Path, lock: Option<DocumentLock>) -> Result<Self, String> {
        let bounded = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)?
            .ok_or_else(|| format!("文档不存在：{}", path.display()))?;
        let bytes = bounded.bytes;
        let decoded = decode_bytes(&bytes)
            .map_err(|error| format!("无法安全解码 {}：{error}", path.display()))?;
        let line_ending = detect_line_ending(&decoded.text);
        let content = normalize_line_endings(&decoded.text);
        let block_index = BlockIndex::new(&content);
        let file_fingerprint = fingerprint_from_metadata(&bounded.metadata, &bytes);
        Ok(Self {
            id: next_document_id(),
            path: Some(path.to_path_buf()),
            analysis: analyze(&content),
            saved_content: content.clone(),
            content,
            encoding: decoded.encoding,
            line_ending,
            dirty: false,
            untitled_id: 0,
            file_fingerprint: Some(file_fingerprint),
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            block_index,
            derived_state_stale: false,
            last_content_edit: None,
            lock,
        })
    }

    pub fn title(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if self.untitled_id <= 1 {
                    "未命名.md".to_owned()
                } else {
                    format!("未命名-{}.md", self.untitled_id)
                }
            })
    }

    /// Returns an identity that remains stable while the document moves within the tab list.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn update_after_edit(&mut self) {
        self.dirty = self.content != self.saved_content;
        self.analysis = analyze(&self.content);
        self.block_index.update(&self.content);
        self.derived_state_stale = false;
        self.last_content_edit = None;
    }

    fn mark_after_edit(&mut self, edited_at: Instant) {
        self.dirty = self.content != self.saved_content;
        self.derived_state_stale = true;
        self.last_content_edit = Some(edited_at);
    }

    pub fn derived_state_is_stale(&self) -> bool {
        self.derived_state_stale
    }

    pub fn refresh_derived_state(&mut self) -> bool {
        if !self.derived_state_stale {
            return false;
        }
        self.analysis = analyze(&self.content);
        self.block_index.update(&self.content);
        self.derived_state_stale = false;
        self.last_content_edit = None;
        true
    }

    pub fn refresh_derived_state_if_idle(&mut self, delay: Duration) -> bool {
        if !self.derived_state_stale
            || self
                .last_content_edit
                .is_some_and(|edited_at| edited_at.elapsed() < delay)
        {
            return false;
        }
        self.refresh_derived_state()
    }

    pub fn blocks(&mut self) -> &[MarkdownBlock] {
        self.refresh_derived_state();
        self.block_index.blocks()
    }

    pub fn record_edit(
        &mut self,
        before: String,
        selection_before: Option<Range<usize>>,
        selection_after: Option<Range<usize>>,
        kind: EditKind,
    ) -> bool {
        if before == self.content {
            return false;
        }

        let now = Instant::now();
        let before_hash = text_hash(&before);
        let after_hash = text_hash(&self.content);
        let can_coalesce = kind == EditKind::Typing
            && self.undo_history.last().is_some_and(|previous| {
                previous.kind == EditKind::Typing
                    && previous.after_hash == before_hash
                    && previous.selection_after == selection_before
                    && now.duration_since(previous.created_at) <= TYPING_COALESCE_WINDOW
            });

        let mut coalesced = false;
        if can_coalesce {
            let mut original_before = before.clone();
            let reversed = self
                .undo_history
                .last()
                .expect("coalescing requires an existing transaction")
                .patch
                .apply_reverse(&mut original_before);
            if reversed {
                let patch = TextPatch::between(&original_before, &self.content);
                let previous = self
                    .undo_history
                    .last_mut()
                    .expect("coalescing requires an existing transaction");
                previous.patch = patch;
                previous.selection_after = selection_after.clone();
                previous.created_at = now;
                previous.after_hash = after_hash;
                coalesced = true;
            }
        }
        if !coalesced {
            let patch = TextPatch::between(&before, &self.content);
            self.undo_history.push(EditTransaction {
                patch,
                before_hash,
                after_hash,
                selection_before,
                selection_after,
                kind,
                created_at: now,
            });
        }
        self.redo_history.clear();
        trim_history(&mut self.undo_history);
        self.mark_after_edit(now);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_history.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_history.is_empty()
    }

    pub fn undo(&mut self) -> Option<HistoryOutcome> {
        let transaction = self.undo_history.pop()?;
        if text_hash(&self.content) != transaction.after_hash
            || !transaction.patch.apply_reverse(&mut self.content)
        {
            self.undo_history.push(transaction);
            return None;
        }
        let outcome = HistoryOutcome {
            selection: transaction.selection_before.clone(),
        };
        self.redo_history.push(transaction);
        trim_history(&mut self.redo_history);
        self.mark_after_edit(Instant::now());
        Some(outcome)
    }

    pub fn redo(&mut self) -> Option<HistoryOutcome> {
        let transaction = self.redo_history.pop()?;
        if text_hash(&self.content) != transaction.before_hash
            || !transaction.patch.apply_forward(&mut self.content)
        {
            self.redo_history.push(transaction);
            return None;
        }
        let outcome = HistoryOutcome {
            selection: transaction.selection_after.clone(),
        };
        self.undo_history.push(transaction);
        trim_history(&mut self.undo_history);
        self.mark_after_edit(Instant::now());
        Some(outcome)
    }

    pub fn has_external_changes(&self) -> Result<bool, String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(false);
        };
        let Some(expected) = self.file_fingerprint.as_ref() else {
            return Ok(path.exists());
        };
        let current = fingerprint(path)?;
        Ok(current.as_ref() != Some(expected))
    }

    pub fn external_change_hint(&self) -> Result<bool, String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(false);
        };
        let Some(expected) = self.file_fingerprint.as_ref() else {
            return Ok(path.exists());
        };
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => {
                return Err(format!("无法检查 {}：{error}", path.display()));
            }
        };
        Ok(metadata.len() != expected.length || metadata.modified().ok() != expected.modified)
    }

    pub fn external_diff(&self) -> Result<String, String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| "未命名文档没有外部版本".to_owned())?;
        let external = Self::open_unlocked(path)?;
        Ok(diffy::create_patch(&self.content, &external.content).to_string())
    }

    pub fn merge_external(&mut self) -> Result<usize, String> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| "未命名文档没有外部版本".to_owned())?;
        let external = Self::open_unlocked(path)?;
        let before = self.content.clone();
        let merged = merge::three_way_merge(&self.saved_content, &self.content, &external.content);

        self.content = merged.content;
        self.saved_content = external.content;
        self.file_fingerprint = external.file_fingerprint;
        self.encoding = external.encoding;
        self.line_ending = external.line_ending;
        if !self.record_edit(before, None, None, EditKind::Other) {
            self.update_after_edit();
        }
        Ok(merged.conflicts)
    }

    pub fn relink_external(&mut self, path: PathBuf) -> Result<usize, String> {
        let path = canonical_document_path(&path)?;
        let new_lock = DocumentLock::acquire(&path)?;
        let external = Self::open_unlocked(&path)?;
        let before = self.content.clone();
        let merged = merge::three_way_merge(&self.saved_content, &self.content, &external.content);

        self.path = Some(path);
        self.lock = Some(new_lock);
        self.content = merged.content;
        self.saved_content = external.content;
        self.file_fingerprint = external.file_fingerprint;
        self.encoding = external.encoding;
        self.line_ending = external.line_ending;
        if !self.record_edit(before, None, None, EditKind::Other) {
            self.update_after_edit();
        }
        Ok(merged.conflicts)
    }

    pub fn save(&mut self, overwrite_external: bool) -> Result<(), String> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| "文档尚未选择保存路径".to_owned())?;
        if !overwrite_external && self.has_external_changes()? {
            return Err(format!(
                "文件已被其他程序修改：{}。保存会覆盖外部修改。",
                path.display()
            ));
        }
        let content = self.line_ending.apply(&self.content);
        let bytes = encode_text(&content, &self.encoding)?;
        ensure_document_size(bytes.len())?;
        write_atomically(path, &bytes, true)?;
        self.file_fingerprint = Some(fingerprint_from_bytes(path, &bytes));
        self.saved_content.clone_from(&self.content);
        self.dirty = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf, overwrite_existing: bool) -> Result<(), String> {
        if path.exists() && !overwrite_existing {
            return Err(format!("目标文件已存在：{}", path.display()));
        }
        let path = canonical_save_target(&path)?;
        if self
            .path
            .as_deref()
            .is_some_and(|current| absolute_path_identity(current) == absolute_path_identity(&path))
        {
            return self.save(true);
        }

        let new_lock = DocumentLock::acquire(&path)?;
        let content = self.line_ending.apply(&self.content);
        let bytes = encode_text(&content, &self.encoding)?;
        ensure_document_size(bytes.len())?;
        write_atomically(&path, &bytes, overwrite_existing)?;

        self.path = Some(path.clone());
        self.lock = Some(new_lock);
        self.file_fingerprint = Some(fingerprint_from_bytes(&path, &bytes));
        self.saved_content.clone_from(&self.content);
        self.dirty = false;
        Ok(())
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| "未命名文档无法从磁盘重新加载".to_owned())?;
        let mut reloaded = Self::open_unlocked(path)?;
        reloaded.lock = self.lock.take();
        *self = reloaded;
        Ok(())
    }

    pub fn recover(
        path: Option<PathBuf>,
        content: String,
        base_content: Option<String>,
        encoding: Option<&str>,
        line_ending: Option<&str>,
        untitled_id: usize,
    ) -> RecoveryOutcome {
        let Some(path) = path else {
            return RecoveryOutcome {
                document: recovered_copy(content, encoding, line_ending, untitled_id),
                conflicts: 0,
                warning: None,
            };
        };

        if path.exists() {
            return match Self::open(&path) {
                Ok(mut document) => {
                    if let Some(base_content) = base_content {
                        let merged =
                            merge::three_way_merge(&base_content, &content, &document.content);
                        document.content = merged.content;
                        document.update_after_edit();
                        RecoveryOutcome {
                            document,
                            conflicts: merged.conflicts,
                            warning: None,
                        }
                    } else if document.content == content {
                        RecoveryOutcome {
                            document,
                            conflicts: 0,
                            warning: None,
                        }
                    } else {
                        RecoveryOutcome {
                            document: recovered_copy(content, encoding, line_ending, untitled_id),
                            conflicts: 0,
                            warning: Some(format!(
                                "旧版恢复快照无法验证磁盘基线，已将 {} 作为未命名副本打开",
                                path.display()
                            )),
                        }
                    }
                }
                Err(error) => RecoveryOutcome {
                    document: recovered_copy(content, encoding, line_ending, untitled_id),
                    conflicts: 0,
                    warning: Some(format!(
                        "无法安全重新关联 {}（{error}），已作为未命名副本打开",
                        path.display()
                    )),
                },
            };
        }

        let mut document = recovered_copy(content, encoding, line_ending, untitled_id);
        match DocumentLock::acquire(&path) {
            Ok(lock) => {
                document.path = Some(path.clone());
                document.saved_content = base_content.unwrap_or_default();
                document.file_fingerprint = None;
                document.lock = Some(lock);
                document.dirty = document.content != document.saved_content;
                RecoveryOutcome {
                    document,
                    conflicts: 0,
                    warning: Some(format!(
                        "原文件 {} 已不存在；保存将重新创建该文件",
                        path.display()
                    )),
                }
            }
            Err(error) => RecoveryOutcome {
                document,
                conflicts: 0,
                warning: Some(format!(
                    "无法锁定恢复目标 {}（{error}），已作为未命名副本打开",
                    path.display()
                )),
            },
        }
    }

    pub(crate) fn recovery_base_content(&self) -> &str {
        &self.saved_content
    }
}

#[derive(Debug)]
struct DocumentLock {
    _file: File,
}

impl DocumentLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        let lock_directory = eframe::storage_dir("RUPORA")
            .map(|directory| directory.join("document-locks"))
            .unwrap_or_else(|| std::env::temp_dir().join("rupora-document-locks"));
        fs::create_dir_all(&lock_directory)
            .map_err(|error| format!("无法创建文档锁目录 {}：{error}", lock_directory.display()))?;
        let lock_path = document_lock_path(&lock_directory, path);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| format!("无法创建文档锁 {}：{error}", lock_path.display()))?;
        fs2::FileExt::try_lock_exclusive(&file)
            .map_err(|_| format!("文档已由另一个 RUPORA 实例编辑：{}", path.display()))?;
        Ok(Self { _file: file })
    }
}

fn next_document_id() -> u64 {
    NEXT_DOCUMENT_ID.fetch_add(1, Ordering::Relaxed)
}

fn canonical_document_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("无法解析文档路径 {}：{error}", path.display()))?;
    Ok(normalize_windows_verbatim_path(canonical))
}

fn canonical_save_target(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return canonical_document_path(path);
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("保存路径缺少文件名：{}", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok(canonical_document_path(parent)?.join(file_name))
}

#[cfg(windows)]
fn normalize_windows_verbatim_path(path: PathBuf) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    const VERBATIM: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: &[u16] = &[b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
    if wide.starts_with(VERBATIM) {
        let suffix = &wide[VERBATIM.len()..];
        if suffix.starts_with(UNC) {
            let mut normalized = vec![b'\\' as u16, b'\\' as u16];
            normalized.extend_from_slice(&suffix[UNC.len()..]);
            return PathBuf::from(OsString::from_wide(&normalized));
        }
        return PathBuf::from(OsString::from_wide(suffix));
    }
    path
}

#[cfg(not(windows))]
fn normalize_windows_verbatim_path(path: PathBuf) -> PathBuf {
    path
}

fn document_lock_path(lock_directory: &Path, path: &Path) -> PathBuf {
    let identity = absolute_path_identity(path);
    let digest = Sha256::digest(path_identity_bytes(&identity));
    lock_directory.join(format!("{digest:x}.lock"))
}

#[cfg(windows)]
fn path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(unix)]
fn path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(any(unix, windows)))]
fn path_identity_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

fn absolute_path_identity(path: &Path) -> PathBuf {
    let absolute = path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    });
    if cfg!(windows) {
        PathBuf::from(absolute.to_string_lossy().to_lowercase())
    } else {
        absolute
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint {
    modified: Option<SystemTime>,
    length: u64,
    content_hash: u64,
}

#[derive(Clone, Debug)]
struct EditTransaction {
    patch: TextPatch,
    before_hash: u64,
    after_hash: u64,
    selection_before: Option<Range<usize>>,
    selection_after: Option<Range<usize>>,
    kind: EditKind,
    created_at: Instant,
}

#[derive(Clone, Debug)]
struct TextPatch {
    start: usize,
    removed: String,
    inserted: String,
}

struct DecodedText {
    text: String,
    encoding: TextEncoding,
}

fn decode_bytes(bytes: &[u8]) -> Result<DecodedText, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok(DecodedText {
            text: String::from_utf8(rest.to_vec())
                .map_err(|_| "UTF-8 BOM 文件包含无效 UTF-8 字节".to_owned())?,
            encoding: TextEncoding::Utf8Bom,
        });
    }

    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return Ok(DecodedText {
            text: decode_utf16(rest, true)?,
            encoding: TextEncoding::Utf16Le,
        });
    }

    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return Ok(DecodedText {
            text: decode_utf16(rest, false)?,
            encoding: TextEncoding::Utf16Be,
        });
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(DecodedText {
            text: text.to_owned(),
            encoding: TextEncoding::Utf8,
        });
    }

    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let detected = detector.guess(None, true);
    // chardetng deliberately reports the GBK superset label for most Chinese
    // text. If the byte stream actually needs GB18030's four-byte sequences,
    // retain GB18030 so saving cannot replace those characters.
    let encoding = if detected == GBK && contains_gb18030_four_byte_sequence(bytes) {
        GB18030
    } else {
        detected
    };
    let (text, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(format!(
            "检测为 {}，但输入包含无法无损解码的字节",
            encoding.name()
        ));
    }
    Ok(DecodedText {
        text: text.into_owned(),
        encoding: TextEncoding::Legacy(encoding),
    })
}

fn contains_gb18030_four_byte_sequence(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|sequence| {
        matches!(sequence[0], 0x81..=0xfe)
            && sequence[1].is_ascii_digit()
            && matches!(sequence[2], 0x81..=0xfe)
            && sequence[3].is_ascii_digit()
    })
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("UTF-16 文件末尾包含不完整的代码单元".to_owned());
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|_| "UTF-16 文件包含未配对的代理项".to_owned())
}

fn ensure_document_size(bytes: usize) -> Result<(), String> {
    if bytes as u64 > MAX_DOCUMENT_BYTES {
        Err(format!(
            "保存内容超过 {} MiB 安全上限",
            MAX_DOCUMENT_BYTES / 1024 / 1024
        ))
    } else {
        Ok(())
    }
}

fn recovered_copy(
    content: String,
    encoding: Option<&str>,
    line_ending: Option<&str>,
    untitled_id: usize,
) -> Document {
    let mut document = Document::untitled(untitled_id);
    document.encoding = recovery_encoding(encoding).unwrap_or(TextEncoding::Utf8);
    document.line_ending = match line_ending {
        Some("CRLF") => LineEnding::CrLf,
        Some("CR") => LineEnding::Cr,
        _ => LineEnding::Lf,
    };
    document.content = content;
    document.update_after_edit();
    document
}

fn recovery_encoding(label: Option<&str>) -> Option<TextEncoding> {
    match label? {
        "UTF-8" => Some(TextEncoding::Utf8),
        "UTF-8 BOM" => Some(TextEncoding::Utf8Bom),
        "UTF-16 LE" => Some(TextEncoding::Utf16Le),
        "UTF-16 BE" => Some(TextEncoding::Utf16Be),
        label => Encoding::for_label(label.as_bytes()).map(TextEncoding::Legacy),
    }
}

fn encode_text(text: &str, encoding: &TextEncoding) -> Result<Vec<u8>, String> {
    match encoding {
        TextEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        TextEncoding::Utf8Bom => {
            let mut bytes = Vec::with_capacity(text.len() + 3);
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            bytes.extend_from_slice(text.as_bytes());
            Ok(bytes)
        }
        TextEncoding::Utf16Le => {
            let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
            bytes.extend_from_slice(&[0xff, 0xfe]);
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(bytes)
        }
        TextEncoding::Utf16Be => {
            let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
            bytes.extend_from_slice(&[0xfe, 0xff]);
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            Ok(bytes)
        }
        TextEncoding::Legacy(encoding) => {
            let (bytes, _, had_errors) = encoding.encode(text);
            if had_errors {
                Err(format!(
                    "内容包含无法用 {} 表示的字符；请另存为 UTF-8",
                    encoding.name()
                ))
            } else {
                Ok(bytes.into_owned())
            }
        }
    }
}

fn detect_line_ending(text: &str) -> LineEnding {
    let crlf = text.match_indices("\r\n").count();
    let cr = text
        .as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\r')
        .count()
        - crlf;
    let lf = text
        .as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        - crlf;

    if crlf >= cr && crlf >= lf && crlf > 0 {
        LineEnding::CrLf
    } else if cr > lf && cr > 0 {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

struct BoundedFile {
    bytes: Vec<u8>,
    metadata: fs::Metadata,
}

fn read_regular_file_bounded(path: &Path, limit: u64) -> Result<Option<BoundedFile>, String> {
    let mut file = match open_readonly_bounded(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法打开 {}：{error}", path.display())),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法检查 {}：{error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("文档不是普通文件：{}", path.display()));
    }
    if metadata.len() > limit {
        return Err(format!(
            "文档超过 {} MiB 安全上限：{}",
            limit / 1024 / 1024,
            path.display()
        ));
    }

    let capacity = usize::try_from(metadata.len().min(limit)).unwrap_or(usize::MAX);
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "文档读取期间增长并超过 {} MiB 安全上限：{}",
            limit / 1024 / 1024,
            path.display()
        ));
    }
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法复核 {}：{error}", path.display()))?;
    Ok(Some(BoundedFile { bytes, metadata }))
}

#[cfg(unix)]
fn open_readonly_bounded(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_readonly_bounded(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

fn fingerprint(path: &Path) -> Result<Option<FileFingerprint>, String> {
    let Some(bounded) = read_regular_file_bounded(path, MAX_DOCUMENT_BYTES)? else {
        return Ok(None);
    };
    Ok(Some(fingerprint_from_metadata(
        &bounded.metadata,
        &bounded.bytes,
    )))
}

fn fingerprint_from_bytes(path: &Path, bytes: &[u8]) -> FileFingerprint {
    let metadata = fs::metadata(path).ok();
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    FileFingerprint {
        modified: metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok()),
        length: bytes.len() as u64,
        content_hash: hasher.finish(),
    }
}

fn fingerprint_from_metadata(metadata: &fs::Metadata, bytes: &[u8]) -> FileFingerprint {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    FileFingerprint {
        modified: metadata.modified().ok(),
        length: bytes.len() as u64,
        content_hash: hasher.finish(),
    }
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn write_atomically(path: &Path, bytes: &[u8], overwrite_existing: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| format!("无法在 {} 创建临时文件：{error}", parent.display()))?;

    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| format!("无法继承 {} 的文件权限：{error}", path.display()))?;
    }

    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| format!("无法写入 {}：{error}", path.display()))?;
    #[cfg(test)]
    if FAIL_BEFORE_ATOMIC_PERSIST.with(|failure| failure.replace(false)) {
        return Err("测试注入：临时文件同步后、原子替换前失败".to_owned());
    }
    #[cfg(test)]
    if CREATE_TARGET_BEFORE_ATOMIC_PERSIST.with(|create| create.replace(false)) {
        fs::write(path, b"concurrent creator")
            .map_err(|error| format!("测试无法创建并发目标 {}：{error}", path.display()))?;
    }
    if overwrite_existing {
        temporary
            .persist(path)
            .map_err(|error| format!("无法替换 {}：{}", path.display(), error.error))?;
    } else {
        temporary.persist_noclobber(path).map_err(|error| {
            format!(
                "目标文件已存在或无法创建 {}：{}",
                path.display(),
                error.error
            )
        })?;
    }
    sync_parent_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), String> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("无法同步目录 {}：{error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static FAIL_BEFORE_ATOMIC_PERSIST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CREATE_TARGET_BEFORE_ATOMIC_PERSIST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn trim_history(history: &mut Vec<EditTransaction>) {
    trim_history_to_budget(history, MAX_HISTORY_ENTRIES, MAX_HISTORY_BYTES);
}

fn trim_history_to_budget(
    history: &mut Vec<EditTransaction>,
    max_entries: usize,
    max_bytes: usize,
) {
    while history.len() > max_entries
        || history
            .iter()
            .map(EditTransaction::memory_cost)
            .fold(0usize, usize::saturating_add)
            > max_bytes
    {
        history.remove(0);
    }
}

impl EditTransaction {
    fn memory_cost(&self) -> usize {
        self.patch.removed.len() + self.patch.inserted.len()
    }
}

impl TextPatch {
    fn between(before: &str, after: &str) -> Self {
        let mut prefix = 0usize;
        for (left, right) in before.chars().zip(after.chars()) {
            if left != right {
                break;
            }
            prefix += left.len_utf8();
        }

        let mut suffix = 0usize;
        for (left, right) in before[prefix..]
            .chars()
            .rev()
            .zip(after[prefix..].chars().rev())
        {
            if left != right
                || prefix + suffix + left.len_utf8() > before.len()
                || prefix + suffix + right.len_utf8() > after.len()
            {
                break;
            }
            suffix += left.len_utf8();
        }

        Self {
            start: prefix,
            removed: before[prefix..before.len() - suffix].to_owned(),
            inserted: after[prefix..after.len() - suffix].to_owned(),
        }
    }

    fn apply_forward(&self, text: &mut String) -> bool {
        let range = self.start..self.start + self.removed.len();
        if text.get(range.clone()) != Some(self.removed.as_str()) {
            return false;
        }
        text.replace_range(range, &self.inserted);
        true
    }

    fn apply_reverse(&self, text: &mut String) -> bool {
        let range = self.start..self.start + self.inserted.len();
        if text.get(range.clone()) != Some(self.inserted.as_str()) {
            return false;
        }
        text.replace_range(range, &self.removed);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_normalizes_crlf() {
        let text = "one\r\ntwo\r\n";
        assert_eq!(detect_line_ending(text), LineEnding::CrLf);
        assert_eq!(normalize_line_endings(text), "one\ntwo\n");
    }

    #[test]
    fn utf16_round_trip_preserves_bom() {
        let original = "你好, RUPORA\n";
        for encoding in [TextEncoding::Utf16Le, TextEncoding::Utf16Be] {
            let bytes = encode_text(original, &encoding).unwrap();
            let decoded = decode_bytes(&bytes).unwrap();
            assert_eq!(decoded.text, original);
            assert_eq!(decoded.encoding, encoding);
        }
    }

    #[test]
    fn gb18030_round_trip() {
        let original = "中文 Markdown";
        let encoding = TextEncoding::Legacy(GB18030);
        let bytes = encode_text(original, &encoding).unwrap();
        let (decoded, _, had_errors) = GB18030.decode(&bytes);
        assert!(!had_errors);
        assert_eq!(decoded, original);
    }

    #[test]
    fn detects_common_gb18030_markdown() {
        let original = "# 中文标题\n\n这是一段用于编码检测的中文内容。";
        let (bytes, _, had_errors) = GB18030.encode(original);
        assert!(!had_errors);

        let decoded = decode_bytes(&bytes).unwrap();
        assert_eq!(decoded.text, original);
        assert_eq!(decoded.encoding, TextEncoding::Legacy(GBK));
    }

    #[test]
    fn distinguishes_four_byte_gb18030_from_gbk() {
        let original = "# 扩展汉字\n\n𠀀";
        let (bytes, _, had_errors) = GB18030.encode(original);
        assert!(!had_errors);

        let decoded = decode_bytes(&bytes).unwrap();
        assert_eq!(decoded.text, original);
        assert_eq!(decoded.encoding, TextEncoding::Legacy(GB18030));
    }

    #[test]
    fn document_save_preserves_encoding_and_line_endings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preserve.md");
        let original = "# 标题\r\n\r\n正文\r\n";
        fs::write(
            &path,
            encode_text(original, &TextEncoding::Utf16Le).unwrap(),
        )
        .unwrap();

        let mut document = Document::open(&path).unwrap();
        assert_eq!(document.encoding, TextEncoding::Utf16Le);
        assert_eq!(document.line_ending, LineEnding::CrLf);
        document.content.push_str("结尾\n");
        document.update_after_edit();
        document.save(false).unwrap();

        let bytes = fs::read(path).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xfe]));
        let decoded = decode_bytes(&bytes).unwrap();
        assert!(decoded.text.contains("正文\r\n结尾\r\n"));
    }

    #[test]
    fn dirty_state_clears_when_content_returns_to_saved_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dirty.md");
        fs::write(&path, "original").unwrap();
        let mut document = Document::open(path).unwrap();

        document.content.push_str(" changed");
        document.update_after_edit();
        assert!(document.dirty);

        document.content = "original".to_owned();
        document.update_after_edit();
        assert!(!document.dirty);
    }

    #[test]
    fn refuses_to_overwrite_an_external_change_without_confirmation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("conflict.md");
        fs::write(&path, "disk version").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.content = "editor version".to_owned();
        document.update_after_edit();
        fs::write(&path, "external version").unwrap();

        assert!(document.has_external_changes().unwrap());
        assert!(document.save(false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external version");

        document.save(true).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "editor version");
    }

    #[test]
    fn failed_save_as_restores_the_original_path() {
        let directory = tempfile::tempdir().unwrap();
        let original_path = directory.path().join("original.md");
        fs::write(&original_path, "content").unwrap();
        let mut document = Document::open(&original_path).unwrap();

        let invalid_target = directory.path().join("target-directory");
        fs::create_dir(&invalid_target).unwrap();
        assert!(document.save_as(invalid_target, true).is_err());
        assert_eq!(document.path.as_deref(), Some(original_path.as_path()));
    }

    #[test]
    fn save_as_without_overwrite_does_not_clobber_a_concurrent_creator() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("created-concurrently.md");
        let mut document = Document::untitled(1);
        document.content = "editor content".to_owned();
        document.update_after_edit();

        CREATE_TARGET_BEFORE_ATOMIC_PERSIST.with(|create| create.set(true));
        assert!(document.save_as(target.clone(), false).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "concurrent creator");
        assert!(document.path.is_none());
        assert!(document.dirty);
    }

    #[test]
    fn injected_atomic_commit_failure_preserves_the_original_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("atomic.md");
        fs::write(&path, "durable original").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.content = "new editor content".to_owned();
        document.update_after_edit();

        FAIL_BEFORE_ATOMIC_PERSIST.with(|failure| failure.set(true));
        assert!(document.save(true).unwrap_err().contains("测试注入"));
        assert_eq!(fs::read_to_string(path).unwrap(), "durable original");
        assert!(document.dirty);
    }

    #[test]
    fn external_change_hint_detects_modified_and_deleted_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("watched.md");
        fs::write(&path, "original").unwrap();
        let document = Document::open(&path).unwrap();
        assert!(!document.external_change_hint().unwrap());

        fs::write(&path, "changed with another length").unwrap();
        assert!(document.external_change_hint().unwrap());

        fs::remove_file(&path).unwrap();
        assert!(document.external_change_hint().unwrap());
    }

    #[test]
    fn coalesces_adjacent_typing_into_one_undo_step() {
        let mut document = Document::untitled(1);
        let before = document.content.clone();
        document.content.push('你');
        document.record_edit(before, Some(0..0), Some(1..1), EditKind::Typing);

        let before = document.content.clone();
        document.content.push('好');
        document.record_edit(before, Some(1..1), Some(2..2), EditKind::Typing);

        assert_eq!(document.content, "你好");
        assert_eq!(document.undo().unwrap().selection, Some(0..0));
        assert_eq!(document.content, "");
        assert!(!document.can_undo());
        assert!(document.can_redo());

        assert_eq!(document.redo().unwrap().selection, Some(2..2));
        assert_eq!(document.content, "你好");
    }

    #[test]
    fn falls_back_to_a_new_transaction_when_coalescing_state_is_inconsistent() {
        let mut document = Document::untitled(1);
        document.content = "a".to_owned();
        document.record_edit(String::new(), None, None, EditKind::Typing);
        document.undo_history.last_mut().unwrap().patch.inserted = "wrong".to_owned();

        document.content = "ab".to_owned();
        document.record_edit("a".to_owned(), None, None, EditKind::Typing);

        assert_eq!(document.undo_history.len(), 2);
        assert!(document.undo().is_some());
        assert_eq!(document.content, "a");
    }

    #[test]
    fn drops_even_a_single_history_entry_when_it_exceeds_the_budget() {
        let mut document = Document::untitled(1);
        document.content = "long edit".to_owned();
        document.record_edit(String::new(), None, None, EditKind::Other);
        assert_eq!(document.undo_history.len(), 1);

        trim_history_to_budget(&mut document.undo_history, 10, 1);
        assert!(document.undo_history.is_empty());
    }

    #[test]
    fn defers_full_markdown_analysis_during_a_typing_burst() {
        let mut document = Document::untitled(1);
        let before = document.content.clone();
        document.content = "# 标题\n\n正文".to_owned();
        document.record_edit(before, None, None, EditKind::Typing);

        assert!(document.derived_state_is_stale());
        assert!(document.analysis.headings.is_empty());
        assert!(!document.refresh_derived_state_if_idle(Duration::from_secs(60)));

        assert!(document.refresh_derived_state_if_idle(Duration::ZERO));
        assert!(!document.derived_state_is_stale());
        assert_eq!(document.analysis.headings[0].text, "标题");
    }

    #[test]
    fn requesting_blocks_refreshes_deferred_derived_state() {
        let mut document = Document::untitled(1);
        let before = document.content.clone();
        document.content = "alpha\n\nbeta".to_owned();
        document.record_edit(before, None, None, EditKind::Typing);

        assert!(document.derived_state_is_stale());
        assert_eq!(document.blocks().len(), 2);
        assert!(!document.derived_state_is_stale());
    }

    #[test]
    fn keeps_formatting_and_replacement_as_separate_transactions() {
        let mut document = Document::untitled(1);
        let before = document.content.clone();
        document.content.push_str("text");
        document.record_edit(before, Some(0..0), Some(4..4), EditKind::Typing);

        let before = document.content.clone();
        document.content = "**text**".to_owned();
        document.record_edit(before, Some(0..4), Some(2..6), EditKind::Format);

        let before = document.content.clone();
        document.content = "**word**".to_owned();
        document.record_edit(before, Some(2..6), Some(6..6), EditKind::Replace);

        document.undo();
        assert_eq!(document.content, "**text**");
        document.undo();
        assert_eq!(document.content, "text");
        document.undo();
        assert_eq!(document.content, "");
    }

    #[test]
    fn new_edit_after_undo_discards_redo_history() {
        let mut document = Document::untitled(1);
        let before = document.content.clone();
        document.content = "first".to_owned();
        document.record_edit(before, None, None, EditKind::Other);
        document.undo();
        assert!(document.can_redo());

        let before = document.content.clone();
        document.content = "different".to_owned();
        document.record_edit(before, None, None, EditKind::Other);
        assert!(!document.can_redo());
    }

    #[test]
    fn undoing_to_saved_content_clears_dirty_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("undo-save.md");
        fs::write(&path, "saved").unwrap();
        let mut document = Document::open(path).unwrap();

        let before = document.content.clone();
        document.content.push('!');
        document.record_edit(before, Some(5..5), Some(6..6), EditKind::Typing);
        assert!(document.dirty);

        document.undo();
        assert!(!document.dirty);
        document.redo();
        assert!(document.dirty);
    }

    #[test]
    fn text_patch_round_trips_unicode_insert_delete_and_replace() {
        for (before, after) in [
            ("你好 world", "你好 brave world"),
            ("emoji 😀 test", "emoji test"),
            ("alpha 中文 omega", "alpha 汉字 omega"),
            ("same suffix suffix", "different suffix"),
        ] {
            let patch = TextPatch::between(before, after);
            let mut text = before.to_owned();
            assert!(patch.apply_forward(&mut text));
            assert_eq!(text, after);
            assert!(patch.apply_reverse(&mut text));
            assert_eq!(text, before);
        }
    }

    #[test]
    fn history_stores_only_the_changed_slice() {
        let before = format!("{}old{}", "a".repeat(100_000), "z".repeat(100_000));
        let after = format!("{}new{}", "a".repeat(100_000), "z".repeat(100_000));
        let patch = TextPatch::between(&before, &after);

        assert_eq!(patch.removed, "old");
        assert_eq!(patch.inserted, "new");
        assert_eq!(patch.removed.len() + patch.inserted.len(), 6);
    }

    #[test]
    fn merges_independent_external_edits_and_refreshes_fingerprint() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("merge.md");
        fs::write(&path, "one\nmiddle\ntwo\n").unwrap();
        let mut document = Document::open(&path).unwrap();
        let before = document.content.clone();
        document.content = "ONE\nmiddle\ntwo\n".to_owned();
        document.record_edit(before, None, None, EditKind::Other);
        fs::write(&path, "one\nmiddle\nTWO\n").unwrap();

        assert_eq!(document.merge_external().unwrap(), 0);
        assert_eq!(document.content, "ONE\nmiddle\nTWO\n");
        assert!(!document.has_external_changes().unwrap());
        assert!(document.dirty);
    }

    #[test]
    fn keeps_both_sides_when_external_merge_conflicts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("conflict-merge.md");
        fs::write(&path, "base\n").unwrap();
        let mut document = Document::open(&path).unwrap();
        let before = document.content.clone();
        document.content = "local\n".to_owned();
        document.record_edit(before, None, None, EditKind::Other);
        fs::write(&path, "external\n").unwrap();

        assert_eq!(document.merge_external().unwrap(), 1);
        assert!(document.content.contains("<<<<<<<"));
        assert!(document.content.contains("local"));
        assert!(document.content.contains("external"));
    }

    #[test]
    fn relinks_a_moved_document_without_losing_local_edits() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.md");
        let moved = directory.path().join("moved.md");
        fs::write(&original, "local target\nmiddle\nexternal target\n").unwrap();
        let mut document = Document::open(&original).unwrap();
        document.content = "local edit\nmiddle\nexternal target\n".to_owned();
        document.update_after_edit();

        fs::rename(&original, &moved).unwrap();
        fs::write(&moved, "local target\nmiddle\nexternal edit\n").unwrap();
        let conflicts = document.relink_external(moved.clone()).unwrap();

        assert_eq!(conflicts, 0);
        assert_eq!(document.path.as_deref(), Some(moved.as_path()));
        assert_eq!(document.content, "local edit\nmiddle\nexternal edit\n");
        assert!(document.dirty);
        assert!(!document.has_external_changes().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn relinking_through_a_symlink_locks_and_saves_the_real_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.md");
        let target = directory.path().join("target.md");
        let link = directory.path().join("target-link.md");
        fs::write(&original, "base").unwrap();
        fs::write(&target, "base").unwrap();
        symlink(&target, &link).unwrap();
        let mut document = Document::open(&original).unwrap();

        document.relink_external(link.clone()).unwrap();
        assert_eq!(document.path.as_deref(), Some(target.as_path()));
        assert!(
            Document::open(&target)
                .unwrap_err()
                .contains("另一个 RUPORA")
        );

        document.content = "updated".to_owned();
        document.update_after_edit();
        document.save(true).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated");
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }

    #[test]
    fn rejects_documents_above_the_resource_limit_before_reading() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("too-large.md");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_DOCUMENT_BYTES + 1).unwrap();
        assert!(Document::open(path).unwrap_err().contains("安全上限"));
    }

    #[test]
    fn rejects_non_regular_documents_and_external_replacements() {
        let directory = tempfile::tempdir().unwrap();
        assert!(Document::open(directory.path()).is_err());

        let path = directory.path().join("replaced.md");
        fs::write(&path, "content").unwrap();
        let document = Document::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(document.has_external_changes().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_character_devices_without_reading_them() {
        assert!(
            Document::open("/dev/null")
                .unwrap_err()
                .contains("普通文件")
        );
    }

    #[test]
    fn prevents_two_editor_instances_from_locking_the_same_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("locked.md");
        fs::write(&path, "content").unwrap();
        let first = Document::open(&path).unwrap();
        assert!(Document::open(&path).unwrap_err().contains("另一个 RUPORA"));
        drop(first);
        assert!(Document::open(path).is_ok());
    }
}
