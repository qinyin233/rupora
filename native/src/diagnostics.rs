use std::{
    backtrace::Backtrace,
    fs::{self, OpenOptions},
    io::Write,
    panic,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 1024 * 1024;
const LOG_FILE_NAME: &str = "rupora.log";

pub fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("unknown panic payload");
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_owned());
        let message = format!(
            "panic at {location}: {payload}\nversion={} os={} arch={}\n{:?}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            Backtrace::force_capture()
        );
        let _ = append_event("PANIC", &message);
        previous(info);
    }));
}

pub fn append_event(level: &str, message: &str) -> Result<(), String> {
    let Some(path) = log_path() else {
        return Ok(());
    };
    append_event_at(&path, level, message)
}

pub fn log_directory() -> Option<PathBuf> {
    eframe::storage_dir("RUPORA").map(|directory| directory.join("logs"))
}

pub fn log_path() -> Option<PathBuf> {
    log_directory().map(|directory| directory.join(LOG_FILE_NAME))
}

fn append_event_at(path: &Path, level: &str, message: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("diagnostic log has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create diagnostic directory: {error}"))?;
    rotate_if_needed(path)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let sanitized_level = level
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("cannot open diagnostic log: {error}"))?;
    writeln!(
        file,
        "[{timestamp}] [{}] {}",
        sanitized_level,
        message.replace('\0', "�")
    )
    .and_then(|()| file.flush())
    .map_err(|error| format!("cannot write diagnostic log: {error}"))
}

fn rotate_if_needed(path: &Path) -> Result<(), String> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() < MAX_LOG_BYTES {
        return Ok(());
    }

    let rotated = path.with_file_name(format!("{LOG_FILE_NAME}.1"));
    if rotated.exists() {
        fs::remove_file(&rotated)
            .map_err(|error| format!("cannot replace rotated diagnostic log: {error}"))?;
    }
    fs::rename(path, rotated).map_err(|error| format!("cannot rotate diagnostic log: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_sanitized_diagnostic_events() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join(LOG_FILE_NAME);

        append_event_at(&path, "INFO\nINJECT", "started\0safely").expect("append event");

        let contents = fs::read_to_string(path).expect("read log");
        assert!(contents.contains("[INFOINJECT]"));
        assert!(contents.contains("started�safely"));
    }

    #[test]
    fn rotates_an_oversized_log() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join(LOG_FILE_NAME);
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).expect("seed oversized log");

        append_event_at(&path, "INFO", "new session").expect("append event");

        assert!(path.with_file_name(format!("{LOG_FILE_NAME}.1")).exists());
        assert!(
            fs::read_to_string(path)
                .expect("read current")
                .contains("new session")
        );
    }
}
