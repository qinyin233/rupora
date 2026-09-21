use std::{
    cell::{Cell, RefCell},
    collections::{HashSet, hash_map::DefaultHasher},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

use crate::document::Document;

const RECOVERY_VERSION: u32 = 4;
const MAX_RECOVERY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECOVERY_DOCUMENTS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryEntry {
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path_native: Option<RecoveryPath>,
    pub content: String,
    #[serde(default)]
    pub base_content: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub line_ending: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "format", content = "data", rename_all = "snake_case")]
enum RecoveryPath {
    UnixBytes(String),
    WindowsUtf16Le(String),
    Utf8(String),
}

impl RecoveryPath {
    fn encode(path: &Path) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            Ok(Self::UnixBytes(BASE64.encode(path.as_os_str().as_bytes())))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt as _;
            let bytes = path
                .as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            Ok(Self::WindowsUtf16Le(BASE64.encode(bytes)))
        }
        #[cfg(not(any(unix, windows)))]
        {
            path.to_str()
                .map(|path| Self::Utf8(path.to_owned()))
                .ok_or_else(|| "当前平台无法编码非 UTF-8 恢复路径".to_owned())
        }
    }

    fn decode(&self) -> Result<PathBuf, String> {
        match self {
            Self::Utf8(path) => Ok(PathBuf::from(path)),
            Self::UnixBytes(encoded) => {
                #[cfg(unix)]
                {
                    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};
                    let bytes = BASE64
                        .decode(encoded)
                        .map_err(|error| format!("无法解码 Unix 恢复路径：{error}"))?;
                    Ok(PathBuf::from(OsString::from_vec(bytes)))
                }
                #[cfg(not(unix))]
                {
                    let _ = encoded;
                    Err("Unix 恢复路径不能在当前平台打开".to_owned())
                }
            }
            Self::WindowsUtf16Le(encoded) => {
                #[cfg(windows)]
                {
                    use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};
                    let bytes = BASE64
                        .decode(encoded)
                        .map_err(|error| format!("无法解码 Windows 恢复路径：{error}"))?;
                    if !bytes.len().is_multiple_of(2) {
                        return Err("Windows 恢复路径包含不完整 UTF-16 单元".to_owned());
                    }
                    let wide = bytes
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>();
                    Ok(PathBuf::from(OsString::from_wide(&wide)))
                }
                #[cfg(not(windows))]
                {
                    let _ = encoded;
                    Err("Windows 恢复路径不能在当前平台打开".to_owned())
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RecoverySnapshot {
    version: u32,
    #[serde(default)]
    saved_at_unix_ms: u128,
    documents: Vec<RecoveryEntry>,
    #[serde(default)]
    checksum: Option<u64>,
    #[serde(default)]
    checksum_sha256: Option<String>,
}

#[derive(Serialize)]
struct RecoveryEntryRef<'a> {
    path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_native: Option<&'a RecoveryPath>,
    content: &'a str,
    base_content: Option<&'a str>,
    encoding: Option<&'a str>,
    line_ending: Option<&'a str>,
}

struct BoundedByteCounter {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}

fn entries_fit_budget(entries: &[RecoveryEntry]) -> Result<bool, String> {
    if entries.len() > MAX_RECOVERY_DOCUMENTS {
        return Ok(false);
    }
    let mut counter = BoundedByteCounter {
        bytes: 0,
        limit: (MAX_RECOVERY_BYTES as usize).saturating_sub(1_024),
        exceeded: false,
    };
    match serde_json::to_writer(&mut counter, entries) {
        Ok(()) => Ok(true),
        Err(_) if counter.exceeded => Ok(false),
        Err(error) => Err(format!("无法估算恢复快照编码大小：{error}")),
    }
}

impl Write for BoundedByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.exceeded = true;
            return Err(std::io::Error::other(
                "serialized recovery entry is too large",
            ));
        };
        if next > self.limit {
            self.exceeded = true;
            return Err(std::io::Error::other(
                "serialized recovery entry is too large",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

enum RecoveryLoadError {
    Corrupt(String),
    Preserve(String),
}

pub struct RecoveryStore {
    path: Option<PathBuf>,
    preserve_blocked: Cell<bool>,
    written_snapshot: RefCell<Option<WrittenSnapshot>>,
}

#[derive(Clone)]
struct WrittenSnapshot {
    checksum: String,
    document_ids: Vec<u64>,
}

impl RecoveryStore {
    pub fn for_app(app_id: &str) -> Self {
        let path = eframe::storage_dir(app_id).map(|directory| directory.join("recovery.json"));
        Self {
            path,
            preserve_blocked: Cell::new(false),
            written_snapshot: RefCell::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            preserve_blocked: Cell::new(false),
            written_snapshot: RefCell::new(None),
        }
    }

    pub fn load(&self) -> Result<Vec<RecoveryEntry>, String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(Vec::new());
        };
        match fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                self.preserve_blocked.set(true);
                return Err(format!("无法检查恢复数据 {}：{error}", path.display()));
            }
        }
        match self.load_snapshot(path) {
            Ok(entries) => {
                self.preserve_blocked.set(false);
                Ok(entries)
            }
            Err(RecoveryLoadError::Preserve(error)) => {
                self.preserve_blocked.set(true);
                Err(error)
            }
            Err(RecoveryLoadError::Corrupt(error)) => {
                let quarantine = quarantine_corrupt_snapshot(path);
                if !matches!(&quarantine, Ok(Some(_))) && path.exists() {
                    self.preserve_blocked.set(true);
                }
                Err(match quarantine {
                    Ok(Some(backup)) => {
                        format!("{error}。损坏快照已隔离到 {}", backup.display())
                    }
                    Ok(None) => error,
                    Err(quarantine_error) => {
                        format!("{error}；同时无法隔离损坏快照：{quarantine_error}")
                    }
                })
            }
        }
    }

    pub fn save(&self, documents: &[Document]) -> Result<(), String> {
        if self.preserve_blocked.get() {
            return Err("恢复快照来自不受支持的版本或暂时不可读，已禁止覆盖".to_owned());
        }
        let mut entries = Vec::new();
        let mut document_ids = Vec::new();
        let mut estimated_bytes = 1_024usize;
        let mut omitted_ids = HashSet::new();
        let dirty_documents = documents.iter().filter(|document| document.dirty).count();
        for document in documents.iter().filter(|document| document.dirty) {
            if entries.len() >= MAX_RECOVERY_DOCUMENTS {
                omitted_ids.insert(document.id());
                continue;
            }
            let base_content = document.recovery_base_content();
            let path_native = document
                .path
                .as_deref()
                .map(RecoveryPath::encode)
                .transpose()?;
            let entry_ref = RecoveryEntryRef {
                path: None,
                path_native: path_native.as_ref(),
                content: &document.content,
                base_content: Some(base_content),
                encoding: Some(document.encoding.label()),
                line_ending: Some(document.line_ending.label()),
            };
            let remaining_budget = (MAX_RECOVERY_BYTES as usize).saturating_sub(estimated_bytes);
            let mut counter = BoundedByteCounter {
                bytes: 0,
                limit: remaining_budget.saturating_sub(1),
                exceeded: false,
            };
            if let Err(error) = serde_json::to_writer(&mut counter, &entry_ref) {
                if !counter.exceeded {
                    return Err(format!("无法估算恢复条目编码大小：{error}"));
                }
                omitted_ids.insert(document.id());
                continue;
            }
            let Some(next_estimate) = estimated_bytes.checked_add(counter.bytes + 1) else {
                omitted_ids.insert(document.id());
                continue;
            };
            if next_estimate as u64 > MAX_RECOVERY_BYTES {
                omitted_ids.insert(document.id());
                continue;
            }
            estimated_bytes = next_estimate;
            document_ids.push(document.id());
            entries.push(RecoveryEntry {
                path: None,
                path_native,
                content: document.content.clone(),
                base_content: Some(base_content.to_owned()),
                encoding: Some(document.encoding.label().to_owned()),
                line_ending: Some(document.line_ending.label().to_owned()),
            });
        }

        if dirty_documents == 0 {
            return self.clear();
        }
        let mut updated_documents = entries.len();
        let mut retained_documents = 0;
        if !omitted_ids.is_empty()
            && let Some(previous) = self.previous_snapshot_for(documents)?
        {
            for (id, entry) in &previous {
                if omitted_ids.contains(id) {
                    document_ids.push(*id);
                    entries.push(entry.clone());
                    retained_documents += 1;
                }
            }
            if !entries_fit_budget(&entries)? {
                // Updating some entries must not evict the only recovery
                // copy of another. If both cannot fit, retain the previous
                // copies, pruning documents explicitly closed or saved.
                document_ids.clear();
                entries.clear();
                for (id, entry) in previous {
                    document_ids.push(id);
                    entries.push(entry);
                }
                updated_documents = 0;
                retained_documents = entries.len();
            }
            if entries.is_empty() {
                // The known old snapshot contained only documents that
                // were closed or saved. Do not resurrect them merely
                // because every remaining draft is too large to store.
                self.clear()?;
            }
        }
        if entries.is_empty() {
            return Err(
                "所有未保存文档都超过恢复资源预算，当前没有可保留的恢复副本；最新修改未保存"
                    .to_owned(),
            );
        }
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建恢复目录 {}：{error}", parent.display()))?;
        }

        let saved_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let checksum = entries_sha256(&entries)?;
        let snapshot = RecoverySnapshot {
            version: RECOVERY_VERSION,
            saved_at_unix_ms,
            checksum: None,
            checksum_sha256: Some(checksum.clone()),
            documents: entries,
        };
        let json = serde_json::to_vec(&snapshot)
            .map_err(|error| format!("无法序列化恢复数据：{error}"))?;
        if json.len() as u64 > MAX_RECOVERY_BYTES {
            return Err("恢复快照编码后超过资源预算；已保留之前的快照".to_owned());
        }
        write_atomically(path, &json)?;
        *self.written_snapshot.borrow_mut() = Some(WrittenSnapshot {
            checksum,
            document_ids,
        });
        if !omitted_ids.is_empty() {
            let unprotected_documents = dirty_documents - updated_documents - retained_documents;
            Err(format!(
                "恢复资源预算不足：已更新 {updated_documents} 个文档，{retained_documents} 个仅保留上次快照，{unprotected_documents} 个无恢复副本；其余最新修改未保存"
            ))
        } else {
            Ok(())
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        if self.preserve_blocked.get() {
            return Err("恢复快照已被保护，不能自动清理".to_owned());
        }
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let result = match fs::remove_file(path) {
            Ok(()) => sync_parent_directory(path.parent().unwrap_or_else(|| Path::new("."))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("无法清理恢复数据 {}：{error}", path.display())),
        };
        if result.is_ok() {
            *self.written_snapshot.borrow_mut() = None;
        }
        result
    }

    fn previous_snapshot_for(
        &self,
        documents: &[Document],
    ) -> Result<Option<Vec<(u64, RecoveryEntry)>>, String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(None);
        };
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "恢复资源预算不足，无法检查旧快照：{error}；本次未写入"
                ));
            }
            Ok(_) => {}
        }
        // Persisted snapshots do not contain session document IDs. Only a
        // snapshot written by this store can be safely matched to untitled
        // documents; path, content, and tab order are not substitutes for ID.
        let Some(written) = self.written_snapshot.borrow().clone() else {
            return Err(
                "恢复资源预算不足，无法确认旧快照的文档身份；本次未写入，已完整保留旧快照"
                    .to_owned(),
            );
        };
        let mut entries = self.load_snapshot(path).map_err(|error| {
            let (RecoveryLoadError::Corrupt(error) | RecoveryLoadError::Preserve(error)) = error;
            format!("无法验证旧恢复快照：{error}；本次未写入")
        })?;
        // load_snapshot exposes decoded native paths to callers. Restore their
        // serialized shape before comparing the exact payload we last wrote.
        for entry in &mut entries {
            if entry.path_native.is_some() {
                entry.path = None;
            }
        }
        if entries.len() != written.document_ids.len()
            || entries_sha256(&entries)? != written.checksum
        {
            return Err(
                "恢复资源预算不足，旧快照已变化，无法确认文档身份；本次未写入，已保留旧快照"
                    .to_owned(),
            );
        }
        let mut retained = Vec::new();
        for (id, mut entry) in written.document_ids.into_iter().zip(entries) {
            let Some(document) = documents
                .iter()
                .find(|document| document.id() == id && document.dirty)
            else {
                continue;
            };
            let previous_path = entry
                .path_native
                .as_ref()
                .map(RecoveryPath::decode)
                .transpose()?
                .or_else(|| entry.path.clone());
            if previous_path != document.path {
                // A save-as or relink can change the file associated with this
                // ID. Preserve old text as a copy, never against the old file.
                entry.path = None;
                entry.path_native = None;
            }
            retained.push((id, entry));
        }
        Ok(Some(retained))
    }

    fn load_snapshot(&self, path: &Path) -> Result<Vec<RecoveryEntry>, RecoveryLoadError> {
        let mut file = open_readonly_bounded(path).map_err(|error| {
            RecoveryLoadError::Preserve(format!("无法打开恢复数据 {}：{error}", path.display()))
        })?;
        let metadata = file.metadata().map_err(|error| {
            RecoveryLoadError::Preserve(format!("无法检查恢复数据 {}：{error}", path.display()))
        })?;
        if !metadata.is_file() {
            return Err(RecoveryLoadError::Preserve(format!(
                "恢复数据不是普通文件：{}",
                path.display()
            )));
        }
        if metadata.len() > MAX_RECOVERY_BYTES {
            return Err(RecoveryLoadError::Corrupt(format!(
                "恢复数据超过 {} MiB 安全上限：{}",
                MAX_RECOVERY_BYTES / 1024 / 1024,
                path.display()
            )));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(MAX_RECOVERY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                RecoveryLoadError::Preserve(format!("无法读取恢复数据 {}：{error}", path.display()))
            })?;
        if bytes.len() as u64 > MAX_RECOVERY_BYTES {
            return Err(RecoveryLoadError::Corrupt(format!(
                "恢复数据读取期间超过 {} MiB 安全上限：{}",
                MAX_RECOVERY_BYTES / 1024 / 1024,
                path.display()
            )));
        }
        // A newer snapshot may use a different schema. Protect it before
        // interpreting its document fields with the current format.
        #[derive(Deserialize)]
        struct VersionHeader {
            version: u32,
        }
        let header: VersionHeader = serde_json::from_slice(&bytes).map_err(|error| {
            RecoveryLoadError::Corrupt(format!("恢复数据格式无效 {}：{error}", path.display()))
        })?;
        if !(1..=RECOVERY_VERSION).contains(&header.version) {
            return Err(RecoveryLoadError::Preserve(format!(
                "恢复数据版本 {} 不受支持（当前版本 {}）",
                header.version, RECOVERY_VERSION
            )));
        }
        let snapshot: RecoverySnapshot = serde_json::from_slice(&bytes).map_err(|error| {
            RecoveryLoadError::Corrupt(format!("恢复数据格式无效 {}：{error}", path.display()))
        })?;
        if snapshot.documents.len() > MAX_RECOVERY_DOCUMENTS {
            return Err(RecoveryLoadError::Corrupt(format!(
                "恢复数据包含过多文档：{}",
                snapshot.documents.len()
            )));
        }
        if snapshot.version == 2 && snapshot.checksum != Some(entries_checksum(&snapshot.documents))
        {
            return Err(RecoveryLoadError::Corrupt(
                "恢复数据校验失败，内容可能已损坏".to_owned(),
            ));
        }
        if snapshot.version >= 3 {
            let expected =
                entries_sha256(&snapshot.documents).map_err(RecoveryLoadError::Corrupt)?;
            if snapshot.checksum_sha256.as_deref() != Some(expected.as_str()) {
                return Err(RecoveryLoadError::Corrupt(
                    "恢复数据 SHA-256 校验失败，内容可能已损坏".to_owned(),
                ));
            }
        }
        let mut documents = snapshot.documents;
        if snapshot.version >= 4 {
            for entry in &mut documents {
                if entry.path.is_some() && entry.path_native.is_some() {
                    return Err(RecoveryLoadError::Corrupt(
                        "恢复条目同时包含旧路径和原生路径".to_owned(),
                    ));
                }
                if let Some(path_native) = entry.path_native.as_ref() {
                    entry.path = Some(path_native.decode().map_err(RecoveryLoadError::Preserve)?);
                }
            }
        }
        Ok(documents)
    }
}

#[cfg(unix)]
fn open_readonly_bounded(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_readonly_bounded(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|error| format!("无法创建恢复临时文件：{error}"))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| format!("无法写入恢复数据：{error}"))?;
    temporary
        .persist(path)
        .map_err(|error| format!("无法提交恢复数据：{}", error.error))?;
    sync_parent_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), String> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("无法同步恢复目录 {}：{error}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<(), String> {
    Ok(())
}

fn entries_checksum(entries: &[RecoveryEntry]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for entry in entries {
        entry.path.hash(&mut hasher);
        entry.content.hash(&mut hasher);
    }
    hasher.finish()
}

fn entries_sha256(entries: &[RecoveryEntry]) -> Result<String, String> {
    let payload =
        serde_json::to_vec(entries).map_err(|error| format!("无法编码恢复校验载荷：{error}"))?;
    Ok(format!("{:x}", Sha256::digest(payload)))
}

fn quarantine_corrupt_snapshot(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let backup = path.with_file_name(format!("recovery.corrupt-{timestamp}.json"));
    #[cfg(test)]
    if FAIL_QUARANTINE.with(|failure| failure.replace(false)) {
        return Err("测试注入：无法隔离恢复快照".to_owned());
    }
    fs::rename(path, &backup).map_err(|error| format!("无法移动 {}：{error}", path.display()))?;
    sync_parent_directory(path.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(Some(backup))
}

#[cfg(test)]
thread_local! {
    static FAIL_QUARANTINE: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_partial_recovery_save_keeps_the_omitted_documents_previous_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let mut documents = vec![Document::untitled(1), Document::untitled(2)];
        documents[0].content = "draft A before growth".to_owned();
        documents[0].update_after_edit();
        documents[1].content = "draft B before update".to_owned();
        documents[1].update_after_edit();
        store.save(&documents).unwrap();

        // The document's allowed text size still fits its own 64 MiB limit,
        // while recovery JSON needs additional room for metadata and a base.
        documents[0].content = "x".repeat(MAX_RECOVERY_BYTES as usize);
        documents[1].content = "draft B latest".to_owned();
        documents[1].update_after_edit();
        let error = store.save(&documents).unwrap_err();
        let recovered = store.load().unwrap();

        assert!(error.contains("预算"), "{error}");
        assert!(
            recovered
                .iter()
                .any(|entry| entry.content == "draft A before growth"),
            "a failed partial save discarded A's previous recovery copy"
        );
        assert!(
            recovered
                .iter()
                .any(|entry| entry.content == "draft B latest"),
            "B still fits the budget and should receive its current snapshot"
        );
    }

    #[test]
    fn audit_partial_recovery_save_with_unknown_ids_preserves_the_complete_old_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        let mut documents = audit_dirty_documents(2);
        RecoveryStore::at(path.clone()).save(&documents).unwrap();
        let original = fs::read(&path).unwrap();
        let new_store = RecoveryStore::at(path.clone());
        assert_eq!(new_store.load().unwrap().len(), 2);
        documents[0].content = "x".repeat(MAX_RECOVERY_BYTES as usize);
        documents[1].content = "newer B".to_owned();

        let error = new_store.save(&documents).unwrap_err();

        assert!(error.contains("身份"), "{error}");
        assert!(error.contains("本次未写入"), "{error}");
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn audit_partial_recovery_save_does_not_resurrect_closed_document_ids() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let mut documents = audit_dirty_documents(3);
        store.save(&documents).unwrap();
        documents.remove(2);
        documents[0].content = "x".repeat(MAX_RECOVERY_BYTES as usize);
        documents[1].content = "newer B".to_owned();

        assert!(store.save(&documents).is_err());
        let entries = store.load().unwrap();

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.content == "draft 0"));
        assert!(entries.iter().any(|entry| entry.content == "newer B"));
        assert!(!entries.iter().any(|entry| entry.content == "draft 2"));

        // Closing the final previously snapshotted documents must remove their
        // entries even when a newly created draft is too large to recover.
        documents.remove(1);
        let mut replacement = Document::untitled(1);
        replacement.content = std::mem::take(&mut documents[0].content);
        replacement.dirty = true;
        let error = store.save(&[replacement]).unwrap_err();
        assert!(error.contains("没有可保留的恢复副本"), "{error}");
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn audit_recovery_count_fallback_retains_old_drafts_and_prunes_closed_ids() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let mut documents = audit_dirty_documents(MAX_RECOVERY_DOCUMENTS);
        store.save(&documents).unwrap();
        documents.remove(0);
        documents.splice(0..0, audit_dirty_documents(2));

        let error = store.save(&documents).unwrap_err();
        let entries = store.load().unwrap();

        assert!(error.contains("已更新 0 个文档"), "{error}");
        assert_eq!(entries.len(), MAX_RECOVERY_DOCUMENTS - 1);
        assert!(entries.iter().any(|entry| entry.content == "draft 99"));
        assert!(!entries.iter().any(|entry| entry.content == "draft 0"));
    }

    #[test]
    fn audit_partial_recovery_save_rejects_a_replaced_snapshot_identity_map() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        let store = RecoveryStore::at(path.clone());
        let mut documents = audit_dirty_documents(MAX_RECOVERY_DOCUMENTS);
        store.save(&documents).unwrap();
        RecoveryStore::at(path.clone())
            .save(&audit_dirty_documents(1))
            .unwrap();
        let replaced = fs::read(&path).unwrap();
        documents.extend(audit_dirty_documents(1));

        let error = store.save(&documents).unwrap_err();

        assert!(error.contains("旧快照已变化"), "{error}");
        assert_eq!(fs::read(path).unwrap(), replaced);
    }

    fn audit_dirty_documents(count: usize) -> Vec<Document> {
        (0..count)
            .map(|index| {
                let mut document = Document::untitled(index + 1);
                document.content = format!("draft {index}");
                document.update_after_edit();
                document
            })
            .collect()
    }

    #[test]
    fn saves_only_dirty_documents_and_loads_them_back() {
        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let clean = Document::untitled(1);
        let mut dirty = Document::untitled(2);
        dirty.content = "recover me".to_owned();
        dirty.update_after_edit();

        store.save(&[clean, dirty]).unwrap();
        let entries = store.load().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "recover me");
    }

    #[test]
    fn clears_snapshot_when_no_dirty_document_remains() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        let store = RecoveryStore::at(path.clone());
        let mut dirty = Document::untitled(1);
        dirty.content = "draft".to_owned();
        dirty.update_after_edit();
        store.save(&[dirty]).unwrap();
        assert!(path.exists());

        store.save(&[Document::untitled(2)]).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn loads_version_one_snapshots_for_migration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        fs::write(
            &path,
            r#"{"version":1,"documents":[{"path":null,"content":"legacy"}]}"#,
        )
        .unwrap();
        let entries = RecoveryStore::at(path).load().unwrap();
        assert_eq!(entries[0].content, "legacy");
    }

    #[test]
    fn quarantines_corrupt_snapshots_instead_of_retrying_forever() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        fs::write(&path, b"not json").unwrap();
        let store = RecoveryStore::at(path.clone());

        let error = store.load().unwrap_err();
        assert!(error.contains("隔离"));
        assert!(!path.exists());
        assert_eq!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("recovery.corrupt-")
                })
                .count(),
            1
        );
    }

    #[test]
    fn protects_a_corrupt_snapshot_when_quarantine_fails() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        let original = b"not json";
        fs::write(&path, original).unwrap();
        let store = RecoveryStore::at(path.clone());
        FAIL_QUARANTINE.with(|failure| failure.set(true));

        assert!(store.load().is_err());
        assert!(store.save(&[Document::untitled(1)]).is_err());
        assert!(store.clear().is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn rejects_tampered_version_two_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        fs::write(
            &path,
            r#"{"version":2,"saved_at_unix_ms":1,"documents":[{"path":null,"content":"changed"}],"checksum":1}"#,
        )
        .unwrap();
        let store = RecoveryStore::at(path.clone());
        assert!(store.load().unwrap_err().contains("校验失败"));
        assert!(!path.exists());
    }

    #[test]
    fn loads_version_three_snapshots_without_changing_the_legacy_checksum_shape() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.json");
        let payload = r#"[{"path":null,"content":"legacy v3","base_content":"base","encoding":"UTF-8","line_ending":"LF"}]"#;
        let checksum = format!("{:x}", Sha256::digest(payload.as_bytes()));
        fs::write(
            &path,
            format!("{{\"version\":3,\"documents\":{payload},\"checksum_sha256\":\"{checksum}\"}}"),
        )
        .unwrap();

        let entries = RecoveryStore::at(path).load().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "legacy v3");
        assert!(entries[0].path.is_none());
    }

    #[test]
    fn preserves_future_snapshots_against_automatic_save_and_clear() {
        for original in [
            r#"{"version":999,"documents":[]}"#,
            r#"{"version":999,"drafts":[{"text":"future draft"}]}"#,
            r#"{"version":999,"documents":{"draft":"future draft"}}"#,
            r#"{"version":999,"documents":[{"content":{"text":"future draft"}}]}"#,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("recovery.json");
            fs::write(&path, original).unwrap();
            let store = RecoveryStore::at(path.clone());

            assert!(store.load().unwrap_err().contains("不受支持"), "{original}");
            assert!(store.save(&[Document::untitled(1)]).is_err());
            assert!(store.clear().is_err());
            assert_eq!(fs::read(path).unwrap(), original.as_bytes());
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn quarantines_invalid_current_snapshot_fields_after_checking_the_version() {
        for original in [
            r#"{"version":4,"drafts":[]}"#,
            r#"{"version":4,"documents":{}}"#,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("recovery.json");
            fs::write(&path, original).unwrap();
            let store = RecoveryStore::at(path.clone());

            assert!(store.load().unwrap_err().contains("隔离"));
            assert!(!path.exists());
            let backup = fs::read_dir(directory.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            assert_eq!(fs::read(backup.path()).unwrap(), original.as_bytes());
            assert!(store.load().unwrap().is_empty());
            store.save(&[Document::untitled(1)]).unwrap();
            store.clear().unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn round_trips_non_utf8_document_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let mut document = Document::untitled(1);
        let path = PathBuf::from(OsString::from_vec(b"draft-\xff.md".to_vec()));
        document.path = Some(path.clone());
        document.content = "dirty".to_owned();
        document.update_after_edit();

        store.save(&[document]).unwrap();
        assert_eq!(store.load().unwrap()[0].path.as_ref(), Some(&path));
    }

    #[cfg(windows)]
    #[test]
    fn round_trips_unpaired_utf16_document_paths() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};

        let directory = tempfile::tempdir().unwrap();
        let store = RecoveryStore::at(directory.path().join("recovery.json"));
        let mut document = Document::untitled(1);
        let path = PathBuf::from(OsString::from_wide(&[b'x' as u16, 0xd800]));
        document.path = Some(path.clone());
        document.content = "dirty".to_owned();
        document.update_after_edit();

        store.save(&[document]).unwrap();
        assert_eq!(store.load().unwrap()[0].path.as_ref(), Some(&path));
    }
}
