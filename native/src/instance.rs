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
            Ok(()) => Ok(InstanceRole::Primary(Self {
                _lock: lock,
                inbox_path,
                last_poll: Instant::now() - POLL_INTERVAL,
            })),
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

        let mut contents = Vec::with_capacity(length as usize);
        inbox
            .read_to_end(&mut contents)
            .map_err(|error| format!("cannot read instance inbox: {error}"))?;
        inbox
            .set_len(0)
            .and_then(|()| inbox.seek(SeekFrom::Start(0)).map(|_| ()))
            .map_err(|error| format!("cannot clear instance inbox: {error}"))?;
        FileExt::unlock(&inbox).ok();

        let mut paths = Vec::new();
        let mut found_request = false;
        for line in contents.split(|byte| *byte == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let Ok(request) = serde_json::from_slice::<WireRequest>(line) else {
                continue;
            };
            if request.version != REQUEST_VERSION {
                continue;
            }
            found_request = true;
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
        Ok(found_request.then_some(OpenRequest { paths }))
    }
}

fn is_lock_contended(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    let expected = fs2::lock_contended_error().raw_os_error();
    expected.is_some() && error.raw_os_error() == expected
}

fn forward_request(path: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let paths = absolute_forwarded_paths(paths)?;
    let request = WireRequest {
        version: REQUEST_VERSION,
        paths,
    };
    let mut encoded = serde_json::to_vec(&request)
        .map_err(|error| format!("cannot serialize instance request: {error}"))?;
    encoded.push(b'\n');

    let mut inbox = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open instance inbox: {error}"))?;
    inbox
        .lock_exclusive()
        .map_err(|error| format!("cannot lock instance inbox: {error}"))?;
    let old_length = inbox
        .metadata()
        .map_err(|error| format!("cannot inspect instance inbox: {error}"))?
        .len();

    let needs_separator = if old_length == 0 {
        false
    } else {
        inbox
            .seek(SeekFrom::End(-1))
            .and_then(|_| {
                let mut last = [0];
                inbox.read_exact(&mut last).map(|()| last[0] != b'\n')
            })
            .map_err(|error| format!("cannot inspect instance inbox ending: {error}"))?
    };

    let added_length = u64::try_from(encoded.len())
        .unwrap_or(u64::MAX)
        .saturating_add(u64::from(needs_separator));
    if old_length.saturating_add(added_length) > MAX_INBOX_BYTES {
        return Err("instance request inbox exceeded its safety limit".to_owned());
    }

    if needs_separator {
        inbox
            .write_all(b"\n")
            .map_err(|error| format!("cannot separate partial instance request: {error}"))?;
    }
    inbox
        .write_all(&encoded)
        .and_then(|()| inbox.sync_data())
        .map_err(|error| format!("cannot forward instance request: {error}"))?;
    FileExt::unlock(&inbox).ok();
    Ok(())
}

fn absolute_forwarded_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let paths = paths.iter().take(MAX_PATHS_PER_REQUEST);
    let current_directory = if paths.clone().any(|path| path.is_relative()) {
        Some(
            std::env::current_dir()
                .map_err(|error| format!("cannot resolve forwarded file paths: {error}"))?,
        )
    } else {
        None
    };

    Ok(paths
        .map(|path| match &current_directory {
            Some(directory) if path.is_relative() => directory.join(path),
            _ => path.clone(),
        })
        .collect())
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

        let current_directory = std::env::current_dir().expect("current directory");
        assert_eq!(
            request.paths,
            expected
                .iter()
                .map(|path| current_directory.join(path))
                .collect::<Vec<_>>()
        );
        assert!(request.paths.iter().all(|path| path.is_absolute()));
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

    #[test]
    fn preserves_requests_that_existed_when_the_primary_started() {
        let directory = tempfile::tempdir().expect("temp directory");
        let inbox_path = directory.path().join("open-requests.jsonl");
        let expected = directory.path().join("pending.md");
        let mut encoded = serde_json::to_vec(&WireRequest {
            version: REQUEST_VERSION,
            paths: vec![expected.clone()],
        })
        .expect("serialize request");
        encoded.push(b'\n');
        fs::write(&inbox_path, encoded).expect("write pending request");

        let mut primary = match InstanceCoordinator::acquire_at(directory.path(), &[])
            .expect("acquire primary")
        {
            InstanceRole::Primary(primary) => primary,
            InstanceRole::Secondary => panic!("first instance must be primary"),
        };
        primary.last_poll = Instant::now() - POLL_INTERVAL;

        assert_eq!(
            primary.poll().expect("poll pending request"),
            Some(OpenRequest {
                paths: vec![expected]
            })
        );
    }

    #[test]
    fn malformed_and_partial_lines_do_not_poison_later_requests() {
        let directory = tempfile::tempdir().expect("temp directory");
        let mut primary = match InstanceCoordinator::acquire_at(directory.path(), &[])
            .expect("acquire primary")
        {
            InstanceRole::Primary(primary) => primary,
            InstanceRole::Secondary => panic!("first instance must be primary"),
        };
        fs::write(
            &primary.inbox_path,
            b"not JSON\n{\"version\":1,\"paths\":[\"unfinished",
        )
        .expect("write damaged inbox");

        let relative = PathBuf::from("forwarded-after-damage.md");
        assert!(matches!(
            InstanceCoordinator::acquire_at(directory.path(), std::slice::from_ref(&relative))
                .expect("forward request after partial line"),
            InstanceRole::Secondary
        ));
        primary.last_poll = Instant::now() - POLL_INTERVAL;

        let request = primary.poll().expect("poll request").expect("request");
        assert_eq!(
            request.paths,
            vec![
                std::env::current_dir()
                    .expect("current directory")
                    .join(relative)
            ]
        );
    }

    #[test]
    fn rejects_a_request_when_existing_and_new_bytes_exceed_the_limit() {
        let directory = tempfile::tempdir().expect("temp directory");
        let inbox_path = directory.path().join("open-requests.jsonl");
        let original = vec![b'x'; MAX_INBOX_BYTES as usize - 1];
        fs::write(&inbox_path, &original).expect("fill inbox");

        let error = forward_request(&inbox_path, &[]).expect_err("inbox must remain bounded");

        assert!(error.contains("safety limit"));
        assert_eq!(fs::read(inbox_path).expect("read inbox"), original);
    }
}
