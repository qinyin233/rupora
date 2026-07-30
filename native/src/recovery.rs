use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::document::Document;

const RECOVERY_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub path: Option<PathBuf>,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RecoverySnapshot {
    version: u32,
    documents: Vec<RecoveryEntry>,
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
        let bytes = fs::read(path)
            .map_err(|error| format!("无法读取恢复数据 {}：{error}", path.display()))?;
        let snapshot: RecoverySnapshot = serde_json::from_slice(&bytes)
            .map_err(|error| format!("恢复数据格式无效 {}：{error}", path.display()))?;
        if snapshot.version != RECOVERY_VERSION {
            return Err(format!(
                "恢复数据版本 {} 不受支持（当前版本 {}）",
                snapshot.version, RECOVERY_VERSION
            ));
        }
        Ok(snapshot.documents)
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
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建恢复目录 {}：{error}", parent.display()))?;
        }

        let snapshot = RecoverySnapshot {
            version: RECOVERY_VERSION,
            documents: entries,
        };
        let json = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| format!("无法序列化恢复数据：{error}"))?;
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
}
