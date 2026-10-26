//! Size-capped diagnostic log file under the config directory.
//!
//! `Logs/macplayer.log` receives timestamped lines from the engine and startup
//! paths. When the file grows past [`MAX_BYTES`] it is rotated to `macplayer.log.old`
//! (replacing any previous rotation), so the log can never grow without bound.
//! Logging is best-effort: every failure is swallowed — diagnostics must never
//! take the player down.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Rotate when the active log exceeds this size.
pub const MAX_BYTES: u64 = 512 * 1024;

/// `Logs/macplayer.log` inside the settings directory.
pub fn log_file() -> PathBuf {
    crate::settings::config_dir()
        .join("Logs")
        .join("macplayer.log")
}

/// Append one timestamped line to the default log file.
pub fn write(line: &str) {
    append_to(&log_file(), line);
}

/// Append one timestamped line to `path`, rotating first when it is too large.
pub fn append_to(path: &Path, line: &str) {
    rotate_if_large(path, MAX_BYTES);
    let Some(dir) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    #[cfg(unix)]
    if let Ok(metadata) = std::fs::metadata(dir) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o700);
        let _ = std::fs::set_permissions(dir, permissions);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let Ok(mut file) = options.open(path) else {
        return;
    };
    let stamp = crate::util::format_unix_time(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or(0),
    );
    // One write call keeps concurrent writers from interleaving mid-line.
    let _ = writeln!(file, "{stamp} {line}");
}

/// Replace `path` with `path.old` when the file is larger than `max_bytes`.
fn rotate_if_large(path: &Path, max_bytes: u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < max_bytes {
        return;
    }
    let Some(old) = rotated_path(path) else {
        return;
    };
    let _ = std::fs::rename(path, &old);
}

/// `macplayer.log` → `macplayer.log.old`; `None` for extension-less names.
fn rotated_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some(path.with_file_name(format!("{name}.old")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("macplayer-log-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_creates_the_file_and_rotates_when_large() {
        let dir = test_dir();
        let log = dir.join("macplayer.log");
        append_to(&log, "first line");
        append_to(&log, "second line");
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("first line"));
        assert!(text.contains("second line"));
        // Lines are timestamped by format_unix_time (or the em-dash fallback).
        assert!(text.lines().all(|line| line.len() > 16));

        rotate_if_large(&log, 10);
        let old = rotated_path(&log).unwrap();
        assert!(old.exists(), "rotation must move the oversized log");
        assert!(!log.exists() || std::fs::metadata(&log).unwrap().len() < 10);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn small_files_are_never_rotated() {
        let dir = test_dir();
        let log = dir.join("macplayer.log");
        append_to(&log, "tiny");
        rotate_if_large(&log, MAX_BYTES);
        assert!(log.exists());
        assert!(!rotated_path(&log).unwrap().exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
