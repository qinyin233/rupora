use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::document::Document;

const RECOVERY_VERSION: u32 = 2;
const MAX_RECOVERY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECOVERY_DOCUMENTS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub path: Option<PathBuf>,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RecoverySnapshot {
    version: u32,
    #[serde(default)]
    saved_at_unix_ms: u128,
    documents: Vec<RecoveryEntry>,
    #[serde(default)]
    checksum: Option<u64>,
}

pub struct RecoveryStore {
    path: Option<PathBuf>,
}

impl RecoveryStore {
    pub fn for_app(app_id: &str) -> Self {
        let path = eframe::storage_dir(app_id).map(|directory| directory.join("recovery.json"));
        Self { path }
    }

    #[cfg(test)]
    fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn load(&self) -> Result<Vec<RecoveryEntry>, String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(Vec::new());
        };
        if !path.exists() {
            return Ok(Vec::new());
        }
        match self.load_snapshot(path) {
            Ok(entries) => Ok(entries),
            Err(error) => {
                let quarantine = quarantine_corrupt_snapshot(path);
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
        let entries = documents
            .iter()
            .filter(|document| document.dirty)
            .map(|document| RecoveryEntry {
                path: document.path.clone(),
                content: document.content.clone(),
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return self.clear();
        }
        if entries.len() > MAX_RECOVERY_DOCUMENTS {
            return Err(format!(
                "未保存文档超过恢复上限：{} > {}",
                entries.len(),
                MAX_RECOVERY_DOCUMENTS
            ));
        }
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建恢复目录 {}：{error}", parent.display()))?;
        }

        let snapshot = RecoverySnapshot {
            version: RECOVERY_VERSION,
            saved_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            checksum: Some(entries_checksum(&entries)),
            documents: entries,
        };
        let json = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| format!("无法序列化恢复数据：{error}"))?;
        if json.len() as u64 > MAX_RECOVERY_BYTES {
            return Err(format!(
                "恢复数据超过 {} MiB 安全上限",
                MAX_RECOVERY_BYTES / 1024 / 1024
            ));
        }
        write_atomically(path, &json)
    }

    pub fn clear(&self) -> Result<(), String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("无法清理恢复数据 {}：{error}", path.display())),
        }
    }

    fn load_snapshot(&self, path: &Path) -> Result<Vec<RecoveryEntry>, String> {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("无法检查恢复数据 {}：{error}", path.display()))?;
        if metadata.len() > MAX_RECOVERY_BYTES {
            return Err(format!(
                "恢复数据超过 {} MiB 安全上限：{}",
                MAX_RECOVERY_BYTES / 1024 / 1024,
                path.display()
            ));
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("无法读取恢复数据 {}：{error}", path.display()))?;
        let snapshot: RecoverySnapshot = serde_json::from_slice(&bytes)
            .map_err(|error| format!("恢复数据格式无效 {}：{error}", path.display()))?;
        if !matches!(snapshot.version, 1 | RECOVERY_VERSION) {
            return Err(format!(
                "恢复数据版本 {} 不受支持（当前版本 {}）",
                snapshot.version, RECOVERY_VERSION
            ));
        }
        if snapshot.documents.len() > MAX_RECOVERY_DOCUMENTS {
            return Err(format!(
                "恢复数据包含过多文档：{}",
                snapshot.documents.len()
            ));
        }
        if snapshot.version >= 2 && snapshot.checksum != Some(entries_checksum(&snapshot.documents))
        {
            return Err("恢复数据校验失败，内容可能已损坏".to_owned());
        }
        Ok(snapshot.documents)
    }
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

fn quarantine_corrupt_snapshot(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let backup = path.with_file_name(format!("recovery.corrupt-{timestamp}.json"));
    fs::rename(path, &backup).map_err(|error| format!("无法移动 {}：{error}", path.display()))?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
