//! Background work owns its inputs and reports results without touching documents or UI state.

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use crate::{
    document::{DocumentSnapshot, SnapshotToken},
    extensions::{self, ExtensionInvocation, ExtensionRegistry, ExtensionService},
    updater::{self, UpdateInfo, UpdateStatus},
};

/// The caller snapshots the target before starting work. A tab index is not an identity.
pub(crate) type ExtensionRequest = DocumentSnapshot;

/// The document owner must check both identity and snapshot before applying a replacement.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExtensionJobResult {
    pub token: SnapshotToken,
    pub invocation: ExtensionInvocation,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BackgroundEvent {
    UpdateChecked(Result<UpdateStatus, String>),
    ExtensionFinished(Result<ExtensionJobResult, String>),
}

/// Runs at most one update check and one extension concurrently.
///
/// A task remains busy until `poll` consumes its result or detects a disconnected worker.
/// Polling never blocks. Dropping this module detaches workers, as closing the app did before;
/// their existing updater/extension timeouts still govern the underlying work.
pub(crate) struct BackgroundTasks {
    update_receiver: Option<Receiver<Result<UpdateStatus, String>>>,
    extension_receiver: Option<Receiver<Result<ExtensionJobResult, String>>>,
    extension_registry: ExtensionRegistry,
    available_update: Option<UpdateInfo>,
}

impl BackgroundTasks {
    pub fn new(extension_registry: ExtensionRegistry) -> Self {
        Self {
            update_receiver: None,
            extension_receiver: None,
            extension_registry,
            available_update: None,
        }
    }

    pub fn update_check_running(&self) -> bool {
        self.update_receiver.is_some()
    }

    pub fn extension_running(&self) -> bool {
        self.extension_receiver.is_some()
    }

    pub fn available_update(&self) -> Option<&UpdateInfo> {
        self.available_update.as_ref()
    }

    pub fn extensions(&self) -> &ExtensionRegistry {
        &self.extension_registry
    }

    pub fn reload_extensions(&mut self) -> Result<(), String> {
        self.extension_registry.reload()
    }

    pub fn start_update_check(&mut self) -> Result<(), String> {
        self.start_update_check_with(|| updater::check_for_update(env!("CARGO_PKG_VERSION")))
    }

    /// Returns the selected service name. Reloading configuration cannot change an active job.
    pub fn start_extension(
        &mut self,
        service_index: usize,
        request: ExtensionRequest,
    ) -> Result<String, String> {
        self.start_extension_with(service_index, request, extensions::invoke)
    }

    /// Returns each completion once, in update-then-extension order when both are ready.
    /// A failed update check preserves the last successfully discovered release.
    pub fn poll(&mut self) -> Vec<BackgroundEvent> {
        let mut events = Vec::new();
        let update = match self.update_receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Empty)) | None => None,
            Some(Err(TryRecvError::Disconnected)) => Some(Err("更新检查线程意外结束".to_owned())),
        };
        if let Some(result) = update {
            self.update_receiver = None;
            match &result {
                Ok(UpdateStatus::Current { .. }) => self.available_update = None,
                Ok(UpdateStatus::Available(info)) => self.available_update = Some(info.clone()),
                Err(_) => {}
            }
            events.push(BackgroundEvent::UpdateChecked(result));
        }

        let extension = match self.extension_receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Empty)) | None => None,
            Some(Err(TryRecvError::Disconnected)) => Some(Err("扩展工作线程意外结束".to_owned())),
        };
        if let Some(result) = extension {
            self.extension_receiver = None;
            events.push(BackgroundEvent::ExtensionFinished(result));
        }
        events
    }

    // Only the blocking I/O operation varies in tests; task lifecycle and polling stay real.
    fn start_update_check_with(
        &mut self,
        check: impl FnOnce() -> Result<UpdateStatus, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.update_check_running() {
            return Err("正在检查更新…".to_owned());
        }
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("rupora-update".to_owned())
            .spawn(move || {
                let _ = sender.send(check());
            })
            .map_err(|error| format!("无法启动更新检查线程：{error}"))?;
        self.update_receiver = Some(receiver);
        Ok(())
    }

    fn start_extension_with(
        &mut self,
        service_index: usize,
        request: ExtensionRequest,
        invoke: impl FnOnce(
            &ExtensionService,
            &str,
            Option<&Path>,
        ) -> Result<ExtensionInvocation, String>
        + Send
        + 'static,
    ) -> Result<String, String> {
        if self.extension_running() {
            return Err("已有扩展服务正在运行".to_owned());
        }
        let service = self
            .extension_registry
            .services()
            .get(service_index)
            .cloned()
            .ok_or_else(|| "扩展服务不存在或扩展功能已关闭".to_owned())?;
        let name = service.name.clone();
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("rupora-extension".to_owned())
            .spawn(move || {
                let invocation = invoke(&service, request.text(), request.path());
                let result = invocation.map(|invocation| ExtensionJobResult {
                    token: request.token(),
                    invocation,
                });
                let _ = sender.send(result);
            })
            .map_err(|error| format!("无法启动扩展工作线程：{error}"))?;
        self.extension_receiver = Some(receiver);
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use super::*;

    fn configured_tasks() -> (tempfile::TempDir, BackgroundTasks) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("extensions.json");
        let config = serde_json::json!({
            "enabled": true,
            "services": [{
                "name": "test extension",
                "program": directory.path().join("extension"),
                "permissions": ["read_document", "replace_document"]
            }]
        });
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let tasks = BackgroundTasks::new(ExtensionRegistry::load(path).unwrap());
        (directory, tasks)
    }

    fn request(document_id: u64) -> ExtensionRequest {
        let mut document = crate::document::Document::untitled(document_id as usize);
        document.content = "原文 🦀".to_owned();
        document.snapshot()
    }

    fn completions(tasks: &mut BackgroundTasks, count: usize) -> Vec<BackgroundEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while events.len() < count {
            events.extend(tasks.poll());
            assert!(
                Instant::now() < deadline,
                "background worker did not finish"
            );
            thread::sleep(Duration::from_millis(1));
        }
        events
    }

    #[test]
    fn independent_jobs_reject_duplicates_and_bind_the_result_to_the_original_document() {
        let (directory, mut tasks) = configured_tasks();
        let (finish_update, wait_update) = mpsc::channel();
        let (finish_extension, wait_extension) = mpsc::channel();
        tasks
            .start_update_check_with(move || {
                wait_update.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(UpdateStatus::Current {
                    latest: semver::Version::new(2, 0, 0),
                })
            })
            .unwrap();
        let path = directory.path().join("原文.md");
        fs::write(&path, "原文 🦀").unwrap();
        let document = crate::document::Document::open(&path).unwrap();
        let target = document.snapshot();
        let token = target.token();
        let expected_path = document.path.clone().unwrap();
        assert_eq!(
            tasks
                .start_extension_with(0, target, move |service, before, path| {
                    assert_eq!(service.name, "test extension");
                    assert_eq!(before, "原文 🦀");
                    assert_eq!(path, Some(expected_path.as_path()));
                    wait_extension.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(ExtensionInvocation {
                        replacement: Some("改写".to_owned()),
                        message: None,
                    })
                })
                .unwrap(),
            "test extension"
        );

        assert!(tasks.update_check_running());
        assert!(tasks.extension_running());
        assert!(tasks.poll().is_empty());
        assert_eq!(tasks.start_update_check().unwrap_err(), "正在检查更新…");
        assert_eq!(
            tasks.start_extension(0, request(99)).unwrap_err(),
            "已有扩展服务正在运行"
        );

        finish_extension.send(()).unwrap();
        assert_eq!(
            completions(&mut tasks, 1),
            vec![BackgroundEvent::ExtensionFinished(Ok(ExtensionJobResult {
                token,
                invocation: ExtensionInvocation {
                    replacement: Some("改写".to_owned()),
                    message: None
                },
            }))]
        );
        assert!(tasks.update_check_running());
        assert!(!tasks.extension_running());
        finish_update.send(()).unwrap();
        assert!(matches!(
            completions(&mut tasks, 1).as_slice(),
            [BackgroundEvent::UpdateChecked(Ok(
                UpdateStatus::Current { .. }
            ))]
        ));
        assert!(!tasks.update_check_running());
        assert!(tasks.poll().is_empty());
    }

    #[test]
    fn disconnected_workers_report_once_and_release_both_job_slots() {
        let (_directory, mut tasks) = configured_tasks();
        tasks
            .start_update_check_with(|| panic!("simulated update worker failure"))
            .unwrap();
        tasks
            .start_extension_with(0, request(42), |_, _, _| {
                panic!("simulated extension worker failure")
            })
            .unwrap();
        let events = completions(&mut tasks, 2);
        assert!(events.contains(&BackgroundEvent::UpdateChecked(Err(
            "更新检查线程意外结束".to_owned()
        ))));
        assert!(events.contains(&BackgroundEvent::ExtensionFinished(Err(
            "扩展工作线程意外结束".to_owned()
        ))));
        assert!(!tasks.update_check_running());
        assert!(!tasks.extension_running());
        assert!(tasks.poll().is_empty());

        tasks
            .start_update_check_with(|| Err("retry update".to_owned()))
            .unwrap();
        tasks
            .start_extension_with(0, request(43), |_, _, _| Err("retry extension".to_owned()))
            .unwrap();
        let events = completions(&mut tasks, 2);
        assert!(events.contains(&BackgroundEvent::UpdateChecked(Err(
            "retry update".to_owned()
        ))));
        assert!(events.contains(&BackgroundEvent::ExtensionFinished(Err(
            "retry extension".to_owned()
        ))));
        assert!(tasks.poll().is_empty());
    }

    #[test]
    fn successful_checks_own_the_available_release_and_failures_preserve_it() {
        let (_directory, mut tasks) = configured_tasks();
        let info = UpdateInfo {
            version: semver::Version::new(2, 1, 0),
            page_url: updater::RELEASES_URL.to_owned(),
            notes: "release notes".to_owned(),
            target: "test-target".to_owned(),
            artifacts: Vec::new(),
        };
        let discovered = info.clone();
        tasks
            .start_update_check_with(move || Ok(UpdateStatus::Available(discovered)))
            .unwrap();
        assert_eq!(
            completions(&mut tasks, 1),
            vec![BackgroundEvent::UpdateChecked(Ok(UpdateStatus::Available(
                info.clone()
            )))]
        );
        assert_eq!(tasks.available_update(), Some(&info));

        tasks
            .start_update_check_with(|| Err("offline".to_owned()))
            .unwrap();
        assert_eq!(
            completions(&mut tasks, 1),
            vec![BackgroundEvent::UpdateChecked(Err("offline".to_owned()))]
        );
        assert_eq!(tasks.available_update(), Some(&info));

        tasks
            .start_update_check_with(|| {
                Ok(UpdateStatus::Current {
                    latest: semver::Version::new(2, 1, 0),
                })
            })
            .unwrap();
        completions(&mut tasks, 1);
        assert_eq!(tasks.available_update(), None);
    }

    #[test]
    fn disabled_or_missing_services_do_not_create_jobs_and_invalid_reload_keeps_configuration() {
        let (directory, mut tasks) = configured_tasks();
        assert!(tasks.start_extension(1, request(42)).is_err());
        assert!(!tasks.extension_running());
        let path = directory.path().join("extensions.json");
        fs::write(&path, b"not json").unwrap();
        assert!(tasks.reload_extensions().is_err());
        assert!(tasks.extensions().is_enabled());
        assert_eq!(tasks.extensions().services().len(), 1);

        fs::write(&path, br#"{"enabled":false}"#).unwrap();
        tasks.reload_extensions().unwrap();
        assert!(!tasks.extensions().is_enabled());
        assert_eq!(
            tasks.start_extension(0, request(42)).unwrap_err(),
            "扩展服务不存在或扩展功能已关闭"
        );
        assert!(!tasks.extension_running());
        assert!(tasks.poll().is_empty());
    }
}
