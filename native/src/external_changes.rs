//! Coordinates external-file changes by document identity, independently of dialogs and views.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::session::DocumentSession;

const SCAN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExternalEvent {
    Reloaded { document_id: u64, path: PathBuf },
    Conflict { document_id: u64, path: PathBuf },
    Error { document_id: u64, error: String },
}

pub(crate) enum ExternalResolution {
    Reload,
    Merge,
    Relink(PathBuf),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExternalResolutionResult {
    pub document_id: u64,
    pub previous_path: Option<PathBuf>,
    pub path: PathBuf,
    pub conflicts: usize,
}

pub(crate) struct ExternalChanges {
    conflicts: HashSet<u64>,
    reported_errors: HashMap<u64, String>,
    last_scan: Instant,
}

impl ExternalChanges {
    pub fn new(now: Instant) -> Self {
        Self {
            conflicts: HashSet::new(),
            reported_errors: HashMap::new(),
            last_scan: now,
        }
    }

    pub fn has_conflict(&self, document_id: u64) -> bool {
        self.conflicts.contains(&document_id)
    }

    pub fn mark_conflict(&mut self, document_id: u64) {
        self.conflicts.insert(document_id);
    }

    /// Call after a document is closed or saved, including a successful Save As.
    pub fn forget(&mut self, document_id: u64) {
        self.conflicts.remove(&document_id);
        self.reported_errors.remove(&document_id);
    }

    /// Clean documents reload automatically; dirty documents retain their local content.
    ///
    /// Each unchanged failure is reported once for its own document. Recovery clears that
    /// document's error so a subsequent failure can be reported again. Closed or untitled
    /// documents lose tracked state even when the disk scan is not due yet.
    pub fn scan(&mut self, session: &mut DocumentSession, now: Instant) -> Vec<ExternalEvent> {
        let tracked = session
            .documents()
            .iter()
            .filter(|document| document.path.is_some())
            .map(|document| document.id())
            .collect::<HashSet<_>>();
        self.conflicts.retain(|id| tracked.contains(id));
        self.reported_errors.retain(|id, _| tracked.contains(id));
        if now.saturating_duration_since(self.last_scan) < SCAN_INTERVAL {
            return Vec::new();
        }
        self.last_scan = now;

        let mut events = Vec::new();
        for document in session.documents_mut() {
            let Some(path) = document.path.clone() else {
                continue;
            };
            let document_id = document.id();
            match document.external_change_hint() {
                Ok(false) => self.forget(document_id),
                Ok(true) if document.dirty => {
                    self.reported_errors.remove(&document_id);
                    self.record_conflict(document_id, path, &mut events);
                }
                Ok(true) => match document.reload() {
                    Ok(()) => {
                        self.forget(document_id);
                        events.push(ExternalEvent::Reloaded { document_id, path });
                    }
                    Err(error) => {
                        self.record_conflict(document_id, path, &mut events);
                        self.record_error(document_id, error, &mut events);
                    }
                },
                Err(error) => self.record_error(document_id, error, &mut events),
            }
        }
        events
    }

    /// The caller has already collected any confirmation required to discard local edits.
    /// Document performs the disk operation and commits its state only after it succeeds.
    pub fn resolve(
        &mut self,
        session: &mut DocumentSession,
        document_id: u64,
        resolution: ExternalResolution,
    ) -> Result<ExternalResolutionResult, String> {
        let Some(document) = session.get_mut(document_id) else {
            self.forget(document_id);
            return Err("目标文档已关闭".to_owned());
        };
        let previous_path = document.path.clone();
        let result = match resolution {
            ExternalResolution::Reload => document.reload().map(|()| 0),
            ExternalResolution::Merge => document.merge_external(),
            ExternalResolution::Relink(path) => document.relink_external(path),
        };
        match result {
            Ok(conflicts) => {
                let path = document
                    .path
                    .clone()
                    .expect("successful external resolution has a document path");
                self.forget(document_id);
                Ok(ExternalResolutionResult {
                    document_id,
                    previous_path,
                    path,
                    conflicts,
                })
            }
            Err(error) => {
                if document.path.is_some() {
                    self.mark_conflict(document_id);
                }
                Err(error)
            }
        }
    }

    fn record_conflict(
        &mut self,
        document_id: u64,
        path: PathBuf,
        events: &mut Vec<ExternalEvent>,
    ) {
        if self.conflicts.insert(document_id) {
            events.push(ExternalEvent::Conflict { document_id, path });
        }
    }

    fn record_error(&mut self, document_id: u64, error: String, events: &mut Vec<ExternalEvent>) {
        if self.reported_errors.get(&document_id) != Some(&error) {
            self.reported_errors.insert(document_id, error.clone());
            events.push(ExternalEvent::Error { document_id, error });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use crate::document::{Document, EditKind};

    use super::*;

    fn open(session: &mut DocumentSession, path: &Path, content: &str) -> u64 {
        fs::write(path, content).unwrap();
        session.insert(Document::open(path).unwrap())
    }

    fn edit(session: &mut DocumentSession, id: u64, content: &str) {
        let document = session.get_mut(id).unwrap();
        let before = std::mem::replace(&mut document.content, content.to_owned());
        assert!(document.record_edit(before, None, None, EditKind::Other));
    }

    fn error_ids(events: &[ExternalEvent]) -> Vec<u64> {
        let mut ids = events
            .iter()
            .filter_map(|event| match event {
                ExternalEvent::Error { document_id, .. } => Some(*document_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn failures_are_deduplicated_per_document_regardless_of_healthy_tab_order() {
        for healthy_first in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut session = DocumentSession::default();
            if healthy_first {
                open(
                    &mut session,
                    &directory.path().join("healthy.md"),
                    "healthy",
                );
            }
            let first_path = directory.path().join("first.md");
            let first = open(&mut session, &first_path, "first");
            let second_path = directory.path().join("second.md");
            let second = open(&mut session, &second_path, "second");
            if !healthy_first {
                open(
                    &mut session,
                    &directory.path().join("healthy.md"),
                    "healthy",
                );
            }
            fs::remove_file(&first_path).unwrap();
            fs::remove_file(&second_path).unwrap();
            let now = Instant::now();
            let mut changes = ExternalChanges::new(now);
            let mut expected = vec![first, second];
            expected.sort_unstable();
            assert_eq!(
                error_ids(&changes.scan(&mut session, now + SCAN_INTERVAL)),
                expected
            );
            assert!(changes.has_conflict(first));
            assert!(changes.has_conflict(second));
            assert!(
                changes
                    .scan(&mut session, now + SCAN_INTERVAL * 2)
                    .is_empty()
            );

            fs::write(&first_path, "first is available again").unwrap();
            let recovered = changes.scan(&mut session, now + SCAN_INTERVAL * 3);
            assert!(
                matches!(recovered.as_slice(), [ExternalEvent::Reloaded { document_id, .. }] if *document_id == first)
            );
            assert!(!changes.has_conflict(first));
            assert!(changes.has_conflict(second));
            fs::remove_file(first_path).unwrap();
            assert_eq!(
                error_ids(&changes.scan(&mut session, now + SCAN_INTERVAL * 4)),
                vec![first],
                "recovery permits a fresh report without repeating another document's failure"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn metadata_errors_do_not_reset_each_other_when_a_healthy_file_is_scanned() {
        use std::os::unix::fs::symlink;

        for healthy_first in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut session = DocumentSession::default();
            if healthy_first {
                open(
                    &mut session,
                    &directory.path().join("healthy.md"),
                    "healthy",
                );
            }
            let path = directory.path().join("loop.md");
            let id = open(&mut session, &path, "original");
            if !healthy_first {
                open(
                    &mut session,
                    &directory.path().join("healthy.md"),
                    "healthy",
                );
            }
            fs::remove_file(&path).unwrap();
            symlink(&path, &path).unwrap();
            assert!(session.get(id).unwrap().external_change_hint().is_err());
            let now = Instant::now();
            let mut changes = ExternalChanges::new(now);
            assert_eq!(
                error_ids(&changes.scan(&mut session, now + SCAN_INTERVAL)),
                vec![id]
            );
            assert!(
                changes
                    .scan(&mut session, now + SCAN_INTERVAL * 2)
                    .is_empty()
            );
            assert_eq!(session.get(id).unwrap().content, "original");
        }
    }

    #[test]
    fn scans_wait_until_due_and_reload_clean_documents_without_changing_active_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = DocumentSession::default();
        let path = directory.path().join("background.md");
        let id = open(&mut session, &path, "before");
        let document_path = session.get(id).unwrap().path.clone().unwrap();
        let active = open(&mut session, &directory.path().join("active.md"), "active");
        let now = Instant::now();
        let mut changes = ExternalChanges::new(now);
        fs::write(&path, "after 中文🙂").unwrap();
        assert!(
            changes
                .scan(&mut session, now + SCAN_INTERVAL - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(session.get(id).unwrap().content, "before");
        assert_eq!(
            changes.scan(&mut session, now + SCAN_INTERVAL),
            vec![ExternalEvent::Reloaded {
                document_id: id,
                path: document_path
            }]
        );
        assert_eq!(session.get(id).unwrap().content, "after 中文🙂");
        assert_eq!(session.active_id(), Some(active));
        assert!(changes.scan(&mut session, now + SCAN_INTERVAL).is_empty());
    }

    #[test]
    fn dirty_conflicts_preserve_local_edits_until_a_successful_merge() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = DocumentSession::default();
        let path = directory.path().join("merge.md");
        let id = open(&mut session, &path, "one\nmiddle\ntwo\n");
        let document_path = session.get(id).unwrap().path.clone().unwrap();
        edit(&mut session, id, "LOCAL\nmiddle\ntwo\n");
        fs::write(&path, "one\nmiddle\nEXTERNAL\n").unwrap();
        let now = Instant::now();
        let mut changes = ExternalChanges::new(now);
        assert_eq!(
            changes.scan(&mut session, now + SCAN_INTERVAL),
            vec![ExternalEvent::Conflict {
                document_id: id,
                path: document_path
            }]
        );
        assert_eq!(session.get(id).unwrap().content, "LOCAL\nmiddle\ntwo\n");
        assert!(changes.has_conflict(id));
        assert!(
            changes
                .scan(&mut session, now + SCAN_INTERVAL * 2)
                .is_empty()
        );
        let result = changes
            .resolve(&mut session, id, ExternalResolution::Merge)
            .unwrap();
        assert_eq!(result.document_id, id);
        assert_eq!(result.conflicts, 0);
        assert!(!changes.has_conflict(id));
        let document = session.get_mut(id).unwrap();
        assert_eq!(document.content, "LOCAL\nmiddle\nEXTERNAL\n");
        assert!(document.dirty);
        assert!(!document.has_external_changes().unwrap());
        document.undo().unwrap();
        assert_eq!(document.content, "LOCAL\nmiddle\ntwo\n");
    }

    #[test]
    fn failed_resolutions_keep_content_history_path_lock_and_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = DocumentSession::default();
        let path = directory.path().join("missing.md");
        let id = open(&mut session, &path, "saved");
        edit(&mut session, id, "local change");
        let original_path = session.get(id).unwrap().path.clone();
        fs::remove_file(&path).unwrap();
        let mut changes = ExternalChanges::new(Instant::now());
        changes.mark_conflict(id);
        for action in [
            ExternalResolution::Reload,
            ExternalResolution::Merge,
            ExternalResolution::Relink(directory.path().join("also-missing.md")),
        ] {
            assert!(changes.resolve(&mut session, id, action).is_err());
            assert!(changes.has_conflict(id));
            assert_eq!(session.get(id).unwrap().content, "local change");
            assert_eq!(session.get(id).unwrap().path, original_path);
            assert_eq!(session.active_id(), Some(id));
            assert!(session.get(id).unwrap().dirty);
        }
        fs::write(&path, "saved").unwrap();
        assert!(
            Document::open(&path).is_err(),
            "failed resolution must retain the original document lock"
        );
        session.get_mut(id).unwrap().undo().unwrap();
        assert_eq!(session.get(id).unwrap().content, "saved");
    }

    #[test]
    fn successful_relink_and_reload_clear_conflicts_and_preserve_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = DocumentSession::default();
        let old = directory.path().join("old.md");
        let id = open(&mut session, &old, "saved\n");
        edit(&mut session, id, "local\n");
        let replacement = directory.path().join("moved.md");
        fs::write(&replacement, "saved\n").unwrap();
        let previous_path = session.get(id).unwrap().path.clone();
        let mut changes = ExternalChanges::new(Instant::now());
        changes.mark_conflict(id);
        let result = changes
            .resolve(
                &mut session,
                id,
                ExternalResolution::Relink(replacement.clone()),
            )
            .unwrap();
        assert_eq!(result.previous_path, previous_path);
        assert_eq!(
            result.path.canonicalize().unwrap(),
            replacement.canonicalize().unwrap()
        );
        assert_eq!(result.conflicts, 0);
        assert_eq!(session.get(id).unwrap().content, "local\n");
        assert_eq!(session.active_id(), Some(id));
        assert!(!changes.has_conflict(id));
        changes.mark_conflict(id);
        let result = changes
            .resolve(&mut session, id, ExternalResolution::Reload)
            .unwrap();
        assert_eq!(result.document_id, id);
        assert_eq!(session.get(id).unwrap().content, "saved\n");
        assert!(!session.get(id).unwrap().dirty);
        assert!(!changes.has_conflict(id));
        assert_eq!(session.active_id(), Some(id));
    }

    #[test]
    fn closed_documents_are_forgotten_even_before_the_next_disk_scan() {
        let mut session = DocumentSession::default();
        let directory = tempfile::tempdir().unwrap();
        let closed = open(&mut session, &directory.path().join("closed.md"), "closed");
        let active = open(&mut session, &directory.path().join("active.md"), "active");
        let now = Instant::now();
        let mut changes = ExternalChanges::new(now);
        changes.mark_conflict(closed);
        changes.mark_conflict(active);
        session.remove(closed).unwrap();
        assert!(changes.scan(&mut session, now).is_empty());
        assert!(!changes.has_conflict(closed));
        assert!(changes.has_conflict(active));
        assert_eq!(session.active_id(), Some(active));
        assert!(
            changes
                .resolve(&mut session, closed, ExternalResolution::Reload)
                .is_err()
        );
        changes.forget(active);
        assert!(!changes.has_conflict(active));
    }
}
