use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

const REQUEST_VERSION: u32 = 1;
const MAX_INBOX_BYTES: u64 = 1024 * 1024;
const MAX_PATHS_PER_REQUEST: usize = 128;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub enum InstanceRole {
    Primary(InstanceCoordinator),
    Secondary,
}

#[derive(Debug)]
pub struct InstanceCoordinator {
    _lock: File,
    inbox_path: PathBuf,
    last_poll: Instant,
}

#[derive(Serialize, Deserialize)]
struct WireRequest {
    version: u32,
    paths: Vec<PathBuf>,
}

impl InstanceCoordinator {
    pub fn acquire(app_id: &str, paths: &[PathBuf]) -> Result<InstanceRole, String> {
        let directory = eframe::storage_dir(app_id)
            .unwrap_or_else(|| std::env::temp_dir().join("rupora-instance"));
        Self::acquire_at(&directory, paths)
    }

    pub fn poll(&mut self) -> Result<Option<OpenRequest>, String> {
        if self.last_poll.elapsed() < POLL_INTERVAL {
            return Ok(None);
        }
        self.last_poll = Instant::now();
        self.read_inbox()
    }

    fn acquire_at(directory: &Path, paths: &[PathBuf]) -> Result<InstanceRole, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create instance directory: {error}"))?;
        let lock_path = directory.join("instance.lock");
        let inbox_path = directory.join("open-requests.jsonl");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| format!("cannot open instance lock: {error}"))?;

        match lock.try_lock_exclusive() {
            Ok(()) => {
                clear_stale_inbox(&inbox_path)?;
                Ok(InstanceRole::Primary(Self {
                    _lock: lock,
                    inbox_path,
                    last_poll: Instant::now() - POLL_INTERVAL,
                }))
            }
            Err(error) if is_lock_contended(&error) => {
                forward_request(&inbox_path, paths)?;
                Ok(InstanceRole::Secondary)
            }
            Err(error) => Err(format!("cannot acquire instance lock: {error}")),
        }
    }

    fn read_inbox(&self) -> Result<Option<OpenRequest>, String> {
        let mut inbox = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.inbox_path)
            .map_err(|error| format!("cannot open instance inbox: {error}"))?;
        inbox
            .lock_exclusive()
            .map_err(|error| format!("cannot lock instance inbox: {error}"))?;

        let length = inbox
            .metadata()
            .map_err(|error| format!("cannot inspect instance inbox: {error}"))?
            .len();
        if length > MAX_INBOX_BYTES {
            inbox
                .set_len(0)
                .map_err(|error| format!("cannot reset oversized instance inbox: {error}"))?;
            FileExt::unlock(&inbox).ok();
            return Err("instance request inbox exceeded its safety limit".to_owned());
        }

        let mut contents = String::new();
        inbox
            .read_to_string(&mut contents)
            .map_err(|error| format!("cannot read instance inbox: {error}"))?;
        inbox
            .set_len(0)
            .and_then(|()| inbox.seek(SeekFrom::Start(0)).map(|_| ()))
            .map_err(|error| format!("cannot clear instance inbox: {error}"))?;
        FileExt::unlock(&inbox).ok();

        if contents.trim().is_empty() {
            return Ok(None);
        }

        let mut paths = Vec::new();
        for line in contents.lines() {
            let request: WireRequest = serde_json::from_str(line)
                .map_err(|error| format!("invalid instance request: {error}"))?;
            if request.version != REQUEST_VERSION {
                continue;
            }
            paths.extend(
                request
                    .paths
                    .into_iter()
                    .take(MAX_PATHS_PER_REQUEST.saturating_sub(paths.len())),
            );
            if paths.len() == MAX_PATHS_PER_REQUEST {
                break;
            }
        }
        Ok(Some(OpenRequest { paths }))
    }
}

fn is_lock_contended(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    let expected = fs2::lock_contended_error().raw_os_error();
    expected.is_some() && error.raw_os_error() == expected
}

fn clear_stale_inbox(path: &Path) -> Result<(), String> {
    let inbox = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| format!("cannot clear stale instance inbox: {error}"))?;
    inbox
        .sync_all()
        .map_err(|error| format!("cannot sync instance inbox: {error}"))
}

fn forward_request(path: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let mut inbox = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open instance inbox: {error}"))?;
    inbox
        .lock_exclusive()
        .map_err(|error| format!("cannot lock instance inbox: {error}"))?;
    if inbox
        .metadata()
        .map_err(|error| format!("cannot inspect instance inbox: {error}"))?
        .len()
        > MAX_INBOX_BYTES
    {
        inbox
            .set_len(0)
            .map_err(|error| format!("cannot reset instance inbox: {error}"))?;
    }

    let request = WireRequest {
        version: REQUEST_VERSION,
        paths: paths.iter().take(MAX_PATHS_PER_REQUEST).cloned().collect(),
    };
    serde_json::to_writer(&mut inbox, &request)
        .map_err(|error| format!("cannot serialize instance request: {error}"))?;
    inbox
        .write_all(b"\n")
        .and_then(|()| inbox.sync_data())
        .map_err(|error| format!("cannot forward instance request: {error}"))?;
    FileExt::unlock(&inbox).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_files_to_the_primary_instance() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut primary = match InstanceCoordinator::acquire_at(directory.path(), &[])
            .expect("acquire primary")
        {
            InstanceRole::Primary(primary) => primary,
            InstanceRole::Secondary => panic!("first instance must be primary"),
        };
        let expected = vec![PathBuf::from("notes.md"), PathBuf::from("文档.markdown")];

        assert!(matches!(
            InstanceCoordinator::acquire_at(directory.path(), &expected).expect("forward request"),
            InstanceRole::Secondary
        ));
        primary.last_poll = Instant::now() - POLL_INTERVAL;
        let request = primary.poll().expect("poll request").expect("request");

        assert_eq!(request.paths, expected);
    }

    #[test]
    fn a_second_launch_without_files_still_requests_focus() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut primary = match InstanceCoordinator::acquire_at(directory.path(), &[])
            .expect("acquire primary")
        {
            InstanceRole::Primary(primary) => primary,
            InstanceRole::Secondary => panic!("first instance must be primary"),
        };

        InstanceCoordinator::acquire_at(directory.path(), &[]).expect("forward focus");
        primary.last_poll = Instant::now() - POLL_INTERVAL;

        assert_eq!(
            primary.poll().expect("poll request"),
            Some(OpenRequest::default())
        );
    }
}
