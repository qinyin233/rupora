//! Real process-termination tests, using the production save/recovery paths.
//!
//! Allowed states at the acknowledged boundary:
//! | Boundary | Existing target | New target | Recovery rewrite |
//! | --- | --- | --- | --- |
//! | temporary-created / temporary-written / file-synced | old | absent | old checkpoint |
//! | committed / directory-synced | complete new | complete new | complete new checkpoint |
//!
//! A previous checkpoint is recoverable; edits made after it are not promised
//! before a document or recovery commit. An old checkpoint cannot silently
//! replace a newly committed document: recovery must keep both conflicting
//! versions. These are OS process kills, not power cuts or storage-cache tests.
//! On Windows the directory-sync helper is a no-op, so that final boundary is
//! only the end of the save path, not evidence of directory fsync durability.

use std::{
    cell::RefCell,
    fs,
    io::{BufRead as _, BufReader, Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    document::{Document, EditKind},
    recovery::{RecoveryEntry, RecoveryStore},
};

const CHILD_TEST: &str = "interrupted_save_tests::interrupted_save_child";
const SPEC_FILE: &str = ".rupora-interrupted-save-test.json";
const READY: &str = "RUPORA_ATOMIC_WRITE_CHECKPOINT:";
const STAGES: [&str; 5] = [
    "temporary-created",
    "temporary-written",
    "file-synced",
    "committed",
    "directory-synced",
];
const BASE: &str = "header\nbase body\nseparator\nbase footer\nend\n";
const CHECKPOINT: &str = "header\ncheckpoint draft\nseparator\nbase footer\nend\n";
const LATEST: &str = "header\nlatest draft\nseparator\nbase footer\nend\n";
const EXTERNAL: &str = "header\nbase body\nseparator\nexternal footer\nend\n";
const UNTITLED_OLD: &str = "未命名草稿 checkpoint 🙂\n";
const UNTITLED_NEW: &str = "未命名草稿 latest 🦀\n";

thread_local! {
    static INTERRUPT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    static LOCK_DIRECTORY: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub(crate) fn lock_directory() -> Option<PathBuf> {
    LOCK_DIRECTORY.with(|directory| directory.borrow().clone())
}

/// Inert for ordinary tests. Only the explicitly launched child arms a stop.
pub(crate) fn checkpoint(writer: &str, stage: &str) {
    if !INTERRUPT.with(|interrupt| {
        interrupt
            .borrow()
            .as_ref()
            .is_some_and(|expected| expected.0 == writer && expected.1 == stage)
    }) {
        return;
    }
    let mut output = std::io::stdout().lock();
    writeln!(output, "\n{READY}{writer}:{stage}").unwrap();
    output.flush().unwrap();
    drop(output);
    // The parent owns this pipe and never writes a byte. A successful test
    // ends this process with Child::kill, without unwinding or running Drop.
    let mut byte = [0];
    let result = std::io::stdin().read_exact(&mut byte);
    panic!("checkpoint resumed instead of being killed: {result:?}");
}

struct IsolatedLocks(Option<PathBuf>);

impl IsolatedLocks {
    fn in_directory(root: &Path) -> Self {
        Self(LOCK_DIRECTORY.with(|directory| directory.replace(Some(root.join("locks")))))
    }
}

impl Drop for IsolatedLocks {
    fn drop(&mut self) {
        LOCK_DIRECTORY.with(|directory| *directory.borrow_mut() = self.0.take());
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Scenario {
    ExistingDocument,
    NewDocument,
    RecoveryRewrite,
}

impl Scenario {
    fn writer(self) -> &'static str {
        match self {
            Self::ExistingDocument | Self::NewDocument => "document",
            Self::RecoveryRewrite => "recovery",
        }
    }
}

#[derive(Deserialize, Serialize)]
struct RunSpec {
    scenario: Scenario,
    stage: String,
}

fn edit_to(document: &mut Document, content: &str) {
    document.edit(EditKind::Typing, None, |text| {
        content.clone_into(text);
        None
    });
}

#[test]
#[ignore = "only launched by the interruption tests, with an isolated fixture and pipes"]
fn interrupted_save_child() {
    let root = std::env::current_dir().unwrap();
    let spec = match fs::read(root.join(SPEC_FILE)) {
        Ok(spec) => serde_json::from_slice::<RunSpec>(&spec).unwrap(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("cannot read child fixture: {error}"),
    };
    assert!(STAGES.contains(&spec.stage.as_str()));
    let _locks = IsolatedLocks::in_directory(&root);
    let target = root.join("document.md");
    let mut document = match spec.scenario {
        Scenario::NewDocument => Document::untitled(1),
        _ => Document::open(&target).unwrap(),
    };
    edit_to(&mut document, LATEST);
    INTERRUPT.with(|interrupt| {
        *interrupt.borrow_mut() = Some((spec.scenario.writer().to_owned(), spec.stage));
    });
    match spec.scenario {
        Scenario::ExistingDocument => document.save(false).unwrap(),
        Scenario::NewDocument => document.save_as(target, false).unwrap(),
        Scenario::RecoveryRewrite => {
            let mut untitled = Document::untitled(2);
            edit_to(&mut untitled, UNTITLED_NEW);
            RecoveryStore::at(root.join("recovery.json"))
                .save(&[document, untitled])
                .unwrap();
        }
    }
    panic!("the requested checkpoint was never reached");
}

struct InterruptedWriter {
    child: Child,
    reader: Option<thread::JoinHandle<()>>,
}

impl InterruptedWriter {
    fn start(root: &Path, scenario: Scenario, stage: &str) -> Self {
        fs::write(
            root.join(SPEC_FILE),
            serde_json::to_vec(&RunSpec {
                scenario,
                stage: stage.to_owned(),
            })
            .unwrap(),
        )
        .unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                CHILD_TEST,
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        // Install the guard before any operation that can panic or time out.
        let mut process = Self {
            child,
            reader: None,
        };
        let stdout = process.child.stdout.take().unwrap();
        let expected = format!("{READY}{}:{stage}", scenario.writer());
        let (sender, receiver) = mpsc::channel();
        process.reader = Some(thread::spawn(move || {
            // Bound diagnostics as well as the wait; a broken child must not
            // fill memory or leave the test runner blocked on its stdout pipe.
            let mut transcript = String::new();
            for line in BufReader::new(stdout.take(64 * 1024)).lines() {
                match line {
                    Ok(line) if line.ends_with(&expected) => {
                        let _ = sender.send(Ok(()));
                        return;
                    }
                    Ok(line) => {
                        transcript.push_str(&line);
                        transcript.push('\n');
                    }
                    Err(error) => {
                        transcript.push_str(&error.to_string());
                        break;
                    }
                }
            }
            let _ = sender.send(Err(transcript));
        }));
        receiver
            .recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("{scenario:?}/{stage}: checkpoint timeout: {error}"))
            .unwrap_or_else(|output| {
                panic!("{scenario:?}/{stage}: child exited before checkpoint:\n{output}")
            });
        assert!(process.child.try_wait().unwrap().is_none());
        process
    }

    fn kill_and_reap(&mut self) {
        assert!(self.child.try_wait().unwrap().is_none());
        self.child.kill().expect("terminate the stopped writer");
        let status = wait_for_exit(&mut self.child, Duration::from_secs(10))
            .expect("killed writer must be reaped within the deadline");
        assert!(!status.success(), "the child must not exit normally");
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt as _;
            assert_eq!(status.signal(), Some(libc::SIGKILL));
        }
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => return None,
        }
    }
}

impl Drop for InterruptedWriter {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            if wait_for_exit(&mut self.child, Duration::from_secs(10)).is_none() {
                eprintln!(
                    "could not reap interrupted writer {} within 10 seconds",
                    self.child.id()
                );
            }
        }
        // Once the child is gone its pipe closes. Do not make a cleanup path
        // unbounded by joining a reader which has not finished yet.
        if let Some(reader) = self.reader.take().filter(thread::JoinHandle::is_finished) {
            let _ = reader.join();
        }
    }
}

fn fixture(scenario: Scenario) -> tempfile::TempDir {
    let directory = tempfile::Builder::new()
        .prefix("rupora-process-interruption-")
        .tempdir()
        .unwrap();
    let _locks = IsolatedLocks::in_directory(directory.path());
    let target = directory.path().join("document.md");
    let mut document = match scenario {
        Scenario::NewDocument => Document::untitled(1),
        _ => {
            fs::write(&target, BASE).unwrap();
            Document::open(&target).unwrap()
        }
    };
    edit_to(&mut document, CHECKPOINT);
    let mut documents = vec![document];
    if matches!(scenario, Scenario::RecoveryRewrite) {
        let mut untitled = Document::untitled(2);
        edit_to(&mut untitled, UNTITLED_OLD);
        documents.push(untitled);
    }
    RecoveryStore::at(directory.path().join("recovery.json"))
        .save(&documents)
        .unwrap();
    directory
}

fn assert_lock_is_held(target: &Path) {
    if target.exists() {
        assert!(
            Document::open(target)
                .unwrap_err()
                .contains("另一个 RUPORA")
        );
    } else {
        let probe = Document::recover(
            Some(target.to_owned()),
            "probe".into(),
            Some(String::new()),
            None,
            None,
            99,
        );
        assert!(probe.document.path.is_none());
        assert!(probe.warning.unwrap().contains("无法锁定恢复目标"));
    }
}

fn assert_lock_is_released(target: &Path) {
    if target.exists() {
        drop(Document::open(target).expect("process termination must release the document lock"));
    } else {
        let probe = Document::recover(
            Some(target.to_owned()),
            "probe".into(),
            Some(String::new()),
            None,
            None,
            99,
        );
        let reopened = probe
            .document
            .path
            .as_deref()
            .expect("reacquire the absent target lock");
        assert_eq!(reopened.file_name(), target.file_name());
        assert_eq!(
            reopened.parent().unwrap().canonicalize().unwrap(),
            target.parent().unwrap().canonicalize().unwrap(),
        );
    }
}

fn recover(entry: RecoveryEntry) -> crate::document::RecoveryOutcome {
    Document::recover(
        entry.path,
        entry.content,
        entry.base_content,
        entry.encoding.as_deref(),
        entry.line_ending.as_deref(),
        10,
    )
}

fn is_committed(stage: &str) -> bool {
    matches!(stage, "committed" | "directory-synced")
}

fn read_document(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("cannot inspect document {}: {error}", path.display()),
    }
}

#[test]
fn process_termination_during_document_save_preserves_disk_and_checkpoint() {
    for scenario in [Scenario::ExistingDocument, Scenario::NewDocument] {
        for stage in STAGES {
            let directory = fixture(scenario);
            let root = directory.path();
            let _locks = IsolatedLocks::in_directory(root);
            let target = root.join("document.md");
            let snapshot = root.join("recovery.json");
            let old_snapshot = fs::read(&snapshot).unwrap();
            eprintln!(
                "process kill: {scenario:?}/{stage} on {}-{} at {} (filesystem not queried by harness)",
                std::env::consts::OS,
                std::env::consts::ARCH,
                root.display()
            );
            let mut writer = InterruptedWriter::start(root, scenario, stage);
            assert_lock_is_held(&target);
            writer.kill_and_reap();
            assert_lock_is_released(&target);

            let disk = read_document(&target);
            let expected_disk = if is_committed(stage) {
                Some(LATEST)
            } else if matches!(scenario, Scenario::ExistingDocument) {
                Some(BASE)
            } else {
                None
            };
            assert_eq!(disk.as_deref(), expected_disk, "{scenario:?}/{stage}");
            assert_eq!(fs::read(&snapshot).unwrap(), old_snapshot);
            let mut entries = RecoveryStore::at(snapshot).load().unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].content, CHECKPOINT);
            let recovered = recover(entries.remove(0));
            assert!(recovered.warning.is_none());
            assert!(recovered.document.dirty);
            if matches!(scenario, Scenario::ExistingDocument) && is_committed(stage) {
                // The previous checkpoint and the newer committed document
                // changed the same line. Never silently substitute the old draft.
                assert!(recovered.conflicts > 0);
                assert!(recovered.document.content.contains("checkpoint draft"));
                assert!(recovered.document.content.contains("latest draft"));
                assert_eq!(recovered.document.recovery_base_content(), LATEST);
            } else {
                assert_eq!(recovered.conflicts, 0);
                assert_eq!(recovered.document.content, CHECKPOINT);
            }
            if matches!(scenario, Scenario::NewDocument) {
                assert!(
                    recovered.document.path.is_none(),
                    "the older untitled checkpoint stays separate from Save As"
                );
            } else {
                assert_eq!(
                    recovered
                        .document
                        .path
                        .as_ref()
                        .unwrap()
                        .canonicalize()
                        .unwrap(),
                    target.canonicalize().unwrap(),
                );
            }
            assert_eq!(
                read_document(&target),
                disk,
                "recovery must never write over the disk version"
            );
        }
    }
}

#[test]
fn process_termination_during_recovery_save_preserves_complete_drafts_and_newer_disk() {
    for stage in STAGES {
        let directory = fixture(Scenario::RecoveryRewrite);
        let root = directory.path();
        let _locks = IsolatedLocks::in_directory(root);
        let target = root.join("document.md");
        let snapshot = root.join("recovery.json");
        let old_snapshot = fs::read(&snapshot).unwrap();
        eprintln!(
            "process kill: RecoveryRewrite/{stage} on {}-{} at {} (filesystem not queried by harness)",
            std::env::consts::OS,
            std::env::consts::ARCH,
            root.display()
        );
        let mut writer = InterruptedWriter::start(root, Scenario::RecoveryRewrite, stage);
        assert_lock_is_held(&target);
        writer.kill_and_reap();
        assert_lock_is_released(&target);
        assert_eq!(fs::read_to_string(&target).unwrap(), BASE);
        if !is_committed(stage) {
            assert_eq!(fs::read(&snapshot).unwrap(), old_snapshot);
        }
        // Model an external edit after termination but before restart. The
        // available local checkpoint must merge into it without writing to disk.
        fs::write(&target, EXTERNAL).unwrap();
        let mut entries = RecoveryStore::at(snapshot).load().unwrap();
        assert_eq!(
            entries.len(),
            2,
            "{stage}: no draft may disappear or be torn"
        );
        let local = if is_committed(stage) {
            LATEST
        } else {
            CHECKPOINT
        };
        let untitled = if is_committed(stage) {
            UNTITLED_NEW
        } else {
            UNTITLED_OLD
        };
        let entry = entries.remove(
            entries
                .iter()
                .position(|entry| entry.path.is_some())
                .unwrap(),
        );
        assert_eq!(entry.content, local);
        assert_eq!(entry.base_content.as_deref(), Some(BASE));
        let recovered = recover(entry);
        assert_eq!(recovered.conflicts, 0);
        assert!(recovered.warning.is_none());
        assert!(recovered.document.dirty);
        assert_eq!(
            recovered
                .document
                .path
                .as_ref()
                .unwrap()
                .canonicalize()
                .unwrap(),
            target.canonicalize().unwrap(),
        );
        assert_eq!(recovered.document.recovery_base_content(), EXTERNAL);
        assert_eq!(
            recovered.document.content,
            local.replace("base footer", "external footer")
        );
        let recovered_untitled = recover(entries.remove(0));
        assert!(recovered_untitled.document.path.is_none());
        assert!(recovered_untitled.document.dirty);
        assert_eq!(recovered_untitled.document.content, untitled);
        assert_eq!(fs::read_to_string(&target).unwrap(), EXTERNAL);
    }
}

#[test]
fn process_termination_guard_releases_child_locks_during_parent_unwind() {
    let directory = fixture(Scenario::ExistingDocument);
    let root = directory.path();
    let _locks = IsolatedLocks::in_directory(root);
    let target = root.join("document.md");
    let snapshot = root.join("recovery.json");
    let old_snapshot = fs::read(&snapshot).unwrap();
    let result = std::panic::catch_unwind(|| {
        let _writer = InterruptedWriter::start(root, Scenario::ExistingDocument, "file-synced");
        assert_lock_is_held(&target);
        panic!("simulate a failed parent assertion while its writer is stopped");
    });
    let failure = result.expect_err("the parent must deliberately unwind at the checkpoint");
    assert_eq!(
        failure.downcast_ref::<&str>(),
        Some(&"simulate a failed parent assertion while its writer is stopped"),
        "an earlier setup or lock-check panic must not count as cleanup coverage",
    );
    assert_lock_is_released(&target);
    assert_eq!(fs::read_to_string(target).unwrap(), BASE);
    assert_eq!(fs::read(snapshot).unwrap(), old_snapshot);
}
