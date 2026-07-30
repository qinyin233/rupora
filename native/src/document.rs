use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use encoding_rs::{Encoding, GB18030, GBK};
use tempfile::NamedTempFile;

use crate::markdown::{BlockIndex, MarkdownAnalysis, MarkdownBlock, analyze};

const MAX_HISTORY_ENTRIES: usize = 256;
const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;
const TYPING_COALESCE_WINDOW: Duration = Duration::from_millis(900);

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

#[derive(Clone, Debug)]
pub struct Document {
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
}

impl Document {
    pub fn untitled(id: usize) -> Self {
        let content = String::new();
        let block_index = BlockIndex::new(&content);
        Self {
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
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes =
            fs::read(path).map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
        let decoded = decode_bytes(&bytes);
        let line_ending = detect_line_ending(&decoded.text);
        let content = normalize_line_endings(&decoded.text);
        let block_index = BlockIndex::new(&content);
        let file_fingerprint = fingerprint_from_bytes(path, &bytes);
        Ok(Self {
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

    pub fn update_after_edit(&mut self) {
        self.dirty = self.content != self.saved_content;
        self.analysis = analyze(&self.content);
        self.block_index.update(&self.content);
    }

    pub fn blocks(&self) -> &[MarkdownBlock] {
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
        let after = self.content.clone();
        let before_hash = text_hash(&before);
        let after_hash = text_hash(&after);
        let can_coalesce = kind == EditKind::Typing
            && self.undo_history.last().is_some_and(|previous| {
                previous.kind == EditKind::Typing
                    && previous.after_hash == before_hash
                    && previous.selection_after == selection_before
                    && now.duration_since(previous.created_at) <= TYPING_COALESCE_WINDOW
            });

        if can_coalesce {
            let previous = self
                .undo_history
                .last_mut()
                .expect("coalescing requires an existing transaction");
            let mut original_before = before;
            let reversed = previous.patch.apply_reverse(&mut original_before);
            debug_assert!(
                reversed,
                "coalesced transaction must match its prior result"
            );
            previous.patch = TextPatch::between(&original_before, &after);
            previous.selection_after = selection_after;
            previous.created_at = now;
            previous.after_hash = after_hash;
        } else {
            self.undo_history.push(EditTransaction {
                patch: TextPatch::between(&before, &after),
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
        self.update_after_edit();
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
        self.update_after_edit();
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
        self.update_after_edit();
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
        write_atomically(path, &bytes)?;
        self.file_fingerprint = Some(fingerprint_from_bytes(path, &bytes));
        self.saved_content.clone_from(&self.content);
        self.dirty = false;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf, overwrite_existing: bool) -> Result<(), String> {
        if path.exists() && !overwrite_existing {
            return Err(format!("目标文件已存在：{}", path.display()));
        }

        let previous_path = self.path.replace(path);
        let previous_fingerprint = self.file_fingerprint.take();
        match self.save(true) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.path = previous_path;
                self.file_fingerprint = previous_fingerprint;
                Err(error)
            }
        }
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| "未命名文档无法从磁盘重新加载".to_owned())?;
        let reloaded = Self::open(path)?;
        *self = reloaded;
        Ok(())
    }

    pub fn recover(path: Option<PathBuf>, content: String, untitled_id: usize) -> Self {
        let mut document = path
            .as_deref()
            .filter(|path| path.exists())
            .and_then(|path| Self::open(path).ok())
            .unwrap_or_else(|| {
                let mut document = Self::untitled(untitled_id);
                document.path = path;
                document
            });
        document.content = content;
        document.update_after_edit();
        document
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

fn decode_bytes(bytes: &[u8]) -> DecodedText {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return DecodedText {
            text: String::from_utf8_lossy(rest).into_owned(),
            encoding: TextEncoding::Utf8Bom,
        };
    }

    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return DecodedText {
            text: decode_utf16(rest, true),
            encoding: TextEncoding::Utf16Le,
        };
    }

    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return DecodedText {
            text: decode_utf16(rest, false),
            encoding: TextEncoding::Utf16Be,
        };
    }

    if let Ok(text) = std::str::from_utf8(bytes) {
        return DecodedText {
            text: text.to_owned(),
            encoding: TextEncoding::Utf8,
        };
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
    let (text, _, _) = encoding.decode(bytes);
    DecodedText {
        text: text.into_owned(),
        encoding: TextEncoding::Legacy(encoding),
    }
}

fn contains_gb18030_four_byte_sequence(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|sequence| {
        matches!(sequence[0], 0x81..=0xfe)
            && sequence[1].is_ascii_digit()
            && matches!(sequence[2], 0x81..=0xfe)
            && sequence[3].is_ascii_digit()
    })
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    char::decode_utf16(units)
        .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
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

fn fingerprint(path: &Path) -> Result<Option<FileFingerprint>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| format!("无法检查 {}：{error}", path.display()))?;
    Ok(Some(fingerprint_from_bytes(path, &bytes)))
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

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
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
    temporary
        .persist(path)
        .map_err(|error| format!("无法替换 {}：{}", path.display(), error.error))?;
    Ok(())
}

fn trim_history(history: &mut Vec<EditTransaction>) {
    while history.len() > MAX_HISTORY_ENTRIES
        || (history.len() > 1
            && history
                .iter()
                .map(EditTransaction::memory_cost)
                .sum::<usize>()
                > MAX_HISTORY_BYTES)
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
            let decoded = decode_bytes(&bytes);
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

        let decoded = decode_bytes(&bytes);
        assert_eq!(decoded.text, original);
        assert_eq!(decoded.encoding, TextEncoding::Legacy(GBK));
    }

    #[test]
    fn distinguishes_four_byte_gb18030_from_gbk() {
        let original = "# 扩展汉字\n\n𠀀";
        let (bytes, _, had_errors) = GB18030.encode(original);
        assert!(!had_errors);

        let decoded = decode_bytes(&bytes);
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
        let decoded = decode_bytes(&bytes);
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
}
