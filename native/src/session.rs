//! Owns document membership and the active tab independently of editor and window state.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::document::{Document, EditKind, SnapshotToken};
use std::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SnapshotApplyError {
    Closed,
    Changed,
}

/// Tab positions are presentation details; every session operation uses document identity.
pub(crate) struct DocumentSession {
    documents: Vec<Document>,
    active_id: Option<u64>,
    next_untitled_number: usize,
}

impl Default for DocumentSession {
    fn default() -> Self {
        Self {
            documents: Vec::new(),
            active_id: None,
            next_untitled_number: 1,
        }
    }
}

impl DocumentSession {
    /// Resolves identity and commits against the exact version used to prepare an edit.
    pub(crate) fn edit_if_current(
        &mut self,
        token: SnapshotToken,
        kind: EditKind,
        selection_before: Option<Range<usize>>,
        edit: impl FnOnce(&mut String) -> Option<Range<usize>>,
    ) -> Result<bool, SnapshotApplyError> {
        self.get_mut(token.document_id())
            .ok_or(SnapshotApplyError::Closed)?
            .edit_if_current(token, kind, selection_before, edit)
            .map_err(|_| SnapshotApplyError::Changed)
    }

    pub(crate) fn documents(&self) -> &[Document] {
        &self.documents
    }

    /// Allows bulk document edits without exposing the backing vector.
    pub(crate) fn documents_mut(&mut self) -> &mut [Document] {
        &mut self.documents
    }

    pub(crate) fn active_id(&self) -> Option<u64> {
        self.active_id
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.active_id.and_then(|id| self.index_of(id))
    }

    pub(crate) fn active(&self) -> Option<&Document> {
        self.active_id.and_then(|id| self.get(id))
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut Document> {
        let id = self.active_id?;
        self.get_mut(id)
    }

    pub(crate) fn get(&self, id: u64) -> Option<&Document> {
        self.documents.iter().find(|document| document.id() == id)
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Document> {
        self.documents
            .iter_mut()
            .find(|document| document.id() == id)
    }

    pub(crate) fn index_of(&self, id: u64) -> Option<usize> {
        self.documents
            .iter()
            .position(|document| document.id() == id)
    }

    /// Inserts an already opened or recovered document and makes it active.
    pub(crate) fn insert(&mut self, document: Document) -> u64 {
        let id = document.id();
        self.documents.push(document);
        self.active_id = Some(id);
        id
    }

    /// Returns whether the active document changed. Unknown IDs leave the session intact.
    pub(crate) fn activate(&mut self, id: u64) -> bool {
        if self.active_id == Some(id) || self.get(id).is_none() {
            return false;
        }
        self.active_id = Some(id);
        true
    }

    /// Removes a document after the caller has handled any save/discard decision.
    ///
    /// Closing the active tab selects the next tab at its former position, or the
    /// preceding tab when it was last. Closing another tab preserves active identity.
    /// An empty session restarts untitled numbering, as closing the final tab does.
    pub(crate) fn remove(&mut self, id: u64) -> Option<Document> {
        let index = self.index_of(id)?;
        let document = self.documents.remove(index);
        if self.documents.is_empty() {
            self.active_id = None;
            self.next_untitled_number = 1;
        } else if self.active_id == Some(id) {
            let next = index.min(self.documents.len() - 1);
            self.active_id = Some(self.documents[next].id());
        }
        Some(document)
    }

    /// Allocates a display number for both new documents and recovered untitled copies.
    pub(crate) fn next_untitled_number(&mut self) -> usize {
        let number = self.next_untitled_number;
        self.next_untitled_number += 1;
        number
    }

    pub(crate) fn new_document(&mut self) -> u64 {
        let number = self.next_untitled_number();
        self.insert(Document::untitled(number))
    }

    pub(crate) fn document_id_for_path(&self, path: &Path) -> Option<u64> {
        let path = canonical_path(path);
        self.documents
            .iter()
            .find(|document| {
                document
                    .path
                    .as_deref()
                    .is_some_and(|open_path| canonical_path(open_path) == path)
            })
            .map(Document::id)
    }

    /// Returns existing, unopened files once each, preserving candidate order.
    /// The caller retains responsibility for opening files and reporting failures.
    pub(crate) fn restorable_files(
        &self,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Vec<PathBuf> {
        let mut seen = self
            .documents
            .iter()
            .filter_map(|document| document.path.as_deref())
            .map(canonical_path)
            .collect::<HashSet<_>>();
        paths
            .into_iter()
            .filter(|path| path.is_file() && seen.insert(canonical_path(path)))
            .collect()
    }
}

fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl std::ops::Index<usize> for DocumentSession {
    type Output = Document;
    fn index(&self, index: usize) -> &Document {
        &self.documents()[index]
    }
}
impl std::ops::IndexMut<usize> for DocumentSession {
    fn index_mut(&mut self, index: usize) -> &mut Document {
        &mut self.documents_mut()[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_edits_follow_identity_and_commit_one_unicode_history_entry() {
        let mut session = DocumentSession::default();
        let first = session.new_document();
        let target = session.new_document();
        session
            .get_mut(target)
            .unwrap()
            .edit(EditKind::Other, None, |text| {
                *text = "原文🙂".to_owned();
                None
            });
        let snapshot = session.get(target).unwrap().snapshot();
        let active = session.new_document();
        session.remove(first);
        assert_eq!(
            session.edit_if_current(snapshot.token(), EditKind::Format, Some(0..3), |text| {
                *text = "改写🦀".to_owned();
                Some(0..3)
            }),
            Ok(true)
        );
        assert_eq!(session.active_id(), Some(active));
        let document = session.get_mut(target).unwrap();
        assert_eq!(document.content, "改写🦀");
        assert_eq!(document.undo().unwrap().selection, Some(0..3));
        assert_eq!(document.content, snapshot.text());
        assert_eq!(document.redo().unwrap().selection, Some(0..3));
        assert_eq!(document.content, "改写🦀");
    }

    #[test]
    fn conditional_edits_reject_closed_or_reverted_documents_before_running_the_edit() {
        let mut session = DocumentSession::default();
        let id = session.new_document();
        let token = session.get(id).unwrap().snapshot_token();
        session
            .get_mut(id)
            .unwrap()
            .edit(EditKind::Other, None, |text| {
                text.push('🙂');
                None
            });
        session.get_mut(id).unwrap().undo().unwrap();
        assert_eq!(
            session.edit_if_current(token, EditKind::Other, None, |_| panic!(
                "stale closure ran"
            )),
            Err(SnapshotApplyError::Changed)
        );
        assert!(session.get(id).unwrap().can_redo());
        session.remove(id);
        assert_eq!(
            session.edit_if_current(token, EditKind::Other, None, |_| panic!(
                "closed closure ran"
            )),
            Err(SnapshotApplyError::Closed)
        );
    }

    #[test]
    fn reloading_a_file_preserves_active_identity_and_future_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reload.md");
        std::fs::write(&path, "before").unwrap();
        let mut session = DocumentSession::default();
        let id = session.insert(Document::open(&path).unwrap());
        std::fs::write(&path, "after 中文🙂").unwrap();
        session.get_mut(id).unwrap().reload().unwrap();
        assert_eq!(session.active_id(), Some(id));
        assert_eq!(session.active_index(), Some(0));
        let document = session
            .active_mut()
            .expect("reload must keep an active document");
        assert_eq!(document.id(), id);
        assert_eq!(document.content, "after 中文🙂");
        document.content.push('!');
        document.update_after_edit();
        document.save(false).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "after 中文🙂!");
    }

    #[test]
    fn closing_a_left_tab_preserves_the_active_document_identity() {
        let mut session = DocumentSession::default();
        let left = session.new_document();
        let middle = session.new_document();
        let right = session.new_document();
        session.active_mut().unwrap().content = "保留🙂".to_owned();

        assert_eq!(session.remove(left).unwrap().id(), left);
        assert_eq!(session.active_id(), Some(right));
        assert_eq!(session.active_index(), Some(1));
        assert_eq!(session.active().unwrap().content, "保留🙂");
        assert_eq!(session.index_of(middle), Some(0));
        assert!(session.get(left).is_none());
    }

    #[test]
    fn unknown_ids_and_reactivating_the_active_tab_are_noops() {
        let mut session = DocumentSession::default();
        let id = session.new_document();
        let unknown = Document::untitled(99).id();

        assert!(!session.activate(unknown));
        assert!(!session.activate(id));
        assert!(session.remove(unknown).is_none());
        assert!(session.get(unknown).is_none());
        assert!(session.get_mut(unknown).is_none());
        assert_eq!(session.active_id(), Some(id));
        assert_eq!(session.documents().len(), 1);
    }

    #[test]
    fn removing_the_active_tab_selects_its_next_then_previous_neighbor() {
        let mut session = DocumentSession::default();
        let first = session.new_document();
        let middle = session.new_document();
        let last = session.new_document();

        assert!(session.activate(middle));
        session.remove(middle).unwrap();
        assert_eq!(session.active_id(), Some(last));
        session.remove(last).unwrap();
        assert_eq!(session.active_id(), Some(first));
        session.remove(first).unwrap();
        assert_eq!(session.active_id(), None);
        assert_eq!(session.active_index(), None);
        assert!(session.active().is_none());
        assert!(session.active_mut().is_none());
        assert!(session.documents().is_empty());
    }

    #[test]
    fn restoration_skips_open_missing_and_duplicate_files_in_candidate_order() {
        let directory = tempfile::tempdir().unwrap();
        let open = directory.path().join("open.md");
        let first = directory.path().join("first.md");
        let second = directory.path().join("second.md");
        for path in [&open, &first, &second] {
            std::fs::write(path, "# 文档").unwrap();
        }
        let mut session = DocumentSession::default();
        let open_id = session.insert(Document::open(&open).unwrap());
        let open_alias = directory.path().join(".").join("open.md");
        let first_alias = directory.path().join(".").join("first.md");

        assert_eq!(session.document_id_for_path(&open_alias), Some(open_id));
        assert!(session.document_id_for_path(&first).is_none());
        assert_eq!(
            session.restorable_files([
                open_alias,
                first.clone(),
                directory.path().join("missing.md"),
                first_alias,
                second.clone(),
                directory.path().to_path_buf(),
                second.clone(),
            ]),
            vec![first, second]
        );
        assert_eq!(session.active_id(), Some(open_id));
        assert_eq!(session.documents().len(), 1);
    }

    #[test]
    fn recovery_and_new_documents_share_untitled_numbering() {
        let mut session = DocumentSession::default();
        let recovered_number = session.next_untitled_number();
        assert_eq!(recovered_number, 1);
        let recovered = session.insert(Document::untitled(recovered_number));
        let next = session.new_document();

        assert_eq!(session.get(recovered).unwrap().title(), "未命名.md");
        assert_eq!(session.get(next).unwrap().title(), "未命名-2.md");
        session.remove(recovered).unwrap();
        let third = session.new_document();
        assert_eq!(session.get(third).unwrap().title(), "未命名-3.md");
        session.remove(next).unwrap();
        session.remove(third).unwrap();
        let restarted = session.new_document();
        assert_eq!(session.get(restarted).unwrap().title(), "未命名.md");
        assert_ne!(restarted, recovered);
    }

    #[test]
    fn mutable_document_access_keeps_session_membership_and_identity() {
        let mut session = DocumentSession::default();
        let first = session.new_document();
        let second = session.new_document();
        session.get_mut(first).unwrap().content = "第一篇".to_owned();
        session.documents_mut()[1].content = "第二篇".to_owned();

        assert_eq!(session.documents().len(), 2);
        assert_eq!(session.get(first).unwrap().content, "第一篇");
        assert_eq!(session.active().unwrap().content, "第二篇");
        assert_eq!(session.active_id(), Some(second));
    }
}
