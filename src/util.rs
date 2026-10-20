//! Small helpers: time formatting, media file detection, natural sorting.
//!
//! Path, media, playlist and time helpers used by MacPlayer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const VIDEO_EXT: &[&str] = &[
    ".mkv", ".mp4", ".m4v", ".avi", ".mov", ".wmv", ".flv", ".webm", ".ts", ".m2ts", ".mts",
    ".mpg", ".mpeg", ".vob", ".3gp", ".ogv", ".rm", ".rmvb", ".asf", ".divx", ".f4v", ".mxf",
    ".dat", ".tp", ".trp", ".wtv", ".y4m", ".gif",
];

pub const AUDIO_EXT: &[&str] = &[
    ".mp3", ".flac", ".m4a", ".aac", ".ogg", ".oga", ".opus", ".wav", ".wma", ".ac3", ".dts",
    ".ape", ".mka", ".aiff", ".alac", ".amr", ".wv", ".tta", ".mpc",
];

pub const SUB_EXT: &[&str] = &[
    ".srt", ".ass", ".ssa", ".sub", ".vtt", ".smi", ".idx", ".sup", ".txt", ".lrc", ".ttml",
];

pub const PLAYLIST_EXT: &[&str] = &[".m3u", ".m3u8", ".pls", ".cue"];

/// Lower-cased extension **with** the leading dot.
pub fn ext(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// `true` when the path has a valid URI scheme prefix.
/// A Windows drive path like `C:/foo` is deliberately not a URL.
pub fn is_url(path: &str) -> bool {
    let Some((scheme, _rest)) = path.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

pub fn is_media(path: &str) -> bool {
    let extension = ext(path);
    VIDEO_EXT.contains(&extension.as_str()) || AUDIO_EXT.contains(&extension.as_str())
}

pub fn is_video(path: &str) -> bool {
    VIDEO_EXT.contains(&ext(path).as_str())
}

pub fn is_audio(path: &str) -> bool {
    AUDIO_EXT.contains(&ext(path).as_str())
}

pub fn is_sub(path: &str) -> bool {
    let extension = ext(path);
    SUB_EXT.contains(&extension.as_str()) && extension != ".txt"
}

/// Split into digit/non-digit runs so "ep2" sorts before "ep10".
pub fn natural_key(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_digits = false;
    for ch in text.chars() {
        let digits = ch.is_ascii_digit();
        if !current.is_empty() && digits != in_digits {
            parts.push(std::mem::take(&mut current));
        }
        in_digits = digits;
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let key_a = natural_key(a);
    let key_b = natural_key(b);
    for (part_a, part_b) in key_a.iter().zip(key_b.iter()) {
        let ordering = if part_a.bytes().all(|byte| byte.is_ascii_digit())
            && part_b.bytes().all(|byte| byte.is_ascii_digit())
        {
            // Do not parse numeric runs as i64: filenames can contain episode or
            // timestamp numbers larger than the integer range.  Comparing the
            // Compare significant lengths first without imposing an integer size limit.
            let significant_a = part_a.trim_start_matches('0');
            let significant_b = part_b.trim_start_matches('0');
            let significant_a = if significant_a.is_empty() {
                "0"
            } else {
                significant_a
            };
            let significant_b = if significant_b.is_empty() {
                "0"
            } else {
                significant_b
            };
            significant_a
                .len()
                .cmp(&significant_b.len())
                .then_with(|| significant_a.cmp(significant_b))
        } else {
            part_a.to_lowercase().cmp(&part_b.to_lowercase())
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    key_a.len().cmp(&key_b.len())
}

/// `HH:MM:SS` always.
pub fn fmt_time(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds as i64
    } else {
        0
    };
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

pub fn fmt_signed(seconds: f64, digits: usize) -> String {
    format!("{:+.*} sec", digits, seconds)
}

/// Accepts `HH:MM:SS(.mmm)`, `MM:SS`, plain seconds or `NN%`. Returns seconds.
pub fn parse_time(text: &str, duration: Option<f64>) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(percent) = text.strip_suffix('%') {
        let duration = duration?;
        if !duration.is_finite() || duration < 0.0 {
            return None;
        }
        let percent: f64 = percent.trim().parse().ok()?;
        if !percent.is_finite() {
            return None;
        }
        return Some((percent.max(0.0).min(100.0)) / 100.0 * duration);
    }
    let mut value = 0.0;
    let mut count = 0;
    for part in text.split(':') {
        let number: f64 = part.trim().parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        value = value * 60.0 + number;
        count += 1;
        if count > 3 {
            return None;
        }
    }
    value.is_finite().then_some(value)
}

pub fn title_of(path: &str) -> String {
    if is_url(path) {
        return path.to_string();
    }
    let path_obj = Path::new(path);
    // Preserve the original path when it ends in a separator.
    let trailing_separator =
        path.ends_with(std::path::MAIN_SEPARATOR) || (cfg!(windows) && path.ends_with('/'));
    if trailing_separator {
        return path.to_string();
    }
    path_obj
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_string())
}

pub fn folder_media(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| is_media(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    names.into_iter().map(|name| folder.join(name)).collect()
}

/// Folders become their media files (recursively); files and URLs pass through.
pub fn expand_paths(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for path in paths {
        if is_url(path) {
            out.push(path.clone());
        } else if Path::new(path).is_dir() {
            collect_media_recursive(Path::new(path), &mut out);
        } else {
            // Make relative paths absolute without resolving symlinks.
            out.push(
                absolute_path(Path::new(path))
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    out
}

fn collect_media_recursive(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut directories: Vec<(String, PathBuf)> = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
        {
            directories.push((name, path));
        } else {
            files.push((name, path));
        }
    }
    // os.walk yields files in a directory before descending into its sorted
    // child directories.  Sorting one mixed list would put a directory named
    // "01" ahead of a file named "02.mkv" and change playback order.
    files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    directories.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    for (_, path) in files {
        if is_media(&path.to_string_lossy()) {
            out.push(path.to_string_lossy().into_owned());
        }
    }
    for (_, path) in directories {
        collect_media_recursive(&path, out);
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

pub fn human_size(bytes: Option<f64>) -> String {
    let Some(mut num) = bytes else {
        return String::new();
    };
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if num < 1024.0 || unit == "TB" {
            return if unit == "B" {
                // Truncate fractional byte counts toward zero for stable display.
                format!("{} {unit}", num as i64)
            } else {
                format!("{num:.1} {unit}")
            };
        }
        num /= 1024.0;
    }
    String::new()
}

/// Files that belong with `path`, such as matching sidecar media files.
pub fn similar_files(path: &str) -> Vec<String> {
    let directory = Path::new(path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let name = title_of(path);
    let extension = ext(&name);
    let kind: Option<&[&str]> = if VIDEO_EXT.contains(&extension.as_str()) {
        Some(VIDEO_EXT)
    } else if AUDIO_EXT.contains(&extension.as_str()) {
        Some(AUDIO_EXT)
    } else {
        None
    };
    let Some(kind) = kind else {
        return vec![path.to_string()];
    };

    let skeleton = |filename: &str| -> String {
        let stem = Path::new(filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mut out = String::new();
        let mut digits = false;
        for ch in stem.chars() {
            if ch.is_ascii_digit() {
                if !digits {
                    out.push('#');
                }
                digits = true;
            } else {
                digits = false;
                out.push(ch);
            }
        }
        out
    };

    let target = skeleton(&name);
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return vec![path.to_string()];
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| kind.contains(&ext(&entry.file_name().to_string_lossy()).as_str()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|candidate| {
            if kind == VIDEO_EXT {
                if target.contains('#') {
                    skeleton(candidate) == target
                } else {
                    *candidate == name
                }
            } else {
                true
            }
        })
        .collect();
    if !names.contains(&name) {
        names.push(name.clone());
    }
    names.sort_by(|a, b| natural_cmp(a, b));
    names
        .into_iter()
        .map(|n| directory.join(n).to_string_lossy().into_owned())
        .collect()
}

/// Minimal `.m3u`/`.m3u8`/`.pls` reader; relative entries resolve against the playlist's folder.
pub fn read_playlist_file(path: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let Ok(bytes) = std::fs::read(path) else {
        return entries;
    };
    // Replacement decoding keeps one invalid byte from making a playlist unusable.
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(text.as_ref());
    let base = absolute_path(Path::new(path))
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let is_pls = ext(path) == ".pls";
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let mut line = line.to_string();
        if is_pls {
            if !line.to_lowercase().starts_with("file") || !line.contains('=') {
                continue;
            }
            line = line.splitn(2, '=').nth(1).unwrap_or("").trim().to_string();
        }
        if !is_url(&line) && !Path::new(&line).is_absolute() {
            line = base.join(&line).to_string_lossy().into_owned();
        }
        entries.push(line);
    }
    entries
}

/// Extensions for the native open dialogs.
pub fn media_filter_pattern() -> String {
    let mut all: HashSet<&str> = HashSet::new();
    for extension in VIDEO_EXT.iter().chain(AUDIO_EXT).chain(PLAYLIST_EXT) {
        all.insert(extension);
    }
    let mut sorted: Vec<&&str> = all.iter().collect();
    sorted.sort();
    sorted
        .iter()
        .map(|e| format!("*{e}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn sub_filter_pattern() -> String {
    let mut sorted: Vec<&&str> = SUB_EXT.iter().collect();
    sorted.sort();
    sorted
        .iter()
        .map(|e| format!("*{e}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("macplayer-util-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn time_formatting() {
        assert_eq!(fmt_time(0.0), "00:00:00");
        assert_eq!(fmt_time(3723.7), "01:02:03");
        assert_eq!(fmt_signed(-1.25, 1), "-1.2 sec");
    }

    #[test]
    fn time_parsing() {
        assert_eq!(parse_time("01:02:03", None), Some(3723.0));
        assert_eq!(parse_time("02:03.5", None), Some(123.5));
        assert_eq!(parse_time("90", None), Some(90.0));
        assert_eq!(parse_time("50%", Some(200.0)), Some(100.0));
        assert_eq!(parse_time("abc", None), None);
        assert_eq!(parse_time("nan", None), None);
        assert_eq!(parse_time("inf", None), None);
        assert_eq!(parse_time("1:2:3:4", None), None);
    }

    #[test]
    fn natural_sorting() {
        let mut names = vec!["ep10.mkv", "ep2.mkv", "ep1.mkv"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["ep1.mkv", "ep2.mkv", "ep10.mkv"]);
    }

    #[test]
    fn natural_sorting_handles_numbers_beyond_i64() {
        let mut names = vec!["part10000000000000000000.mkv", "part2.mkv"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["part2.mkv", "part10000000000000000000.mkv"]);
    }

    #[test]
    fn url_detection() {
        assert!(is_url("https://example.com/a.mp4"));
        assert!(is_url("ytdl://youtube.com/watch?v=x"));
        assert!(!is_url("/plain/path.mkv"));
        assert!(!is_url("C:/windows/path.mkv"));
    }

    #[test]
    fn sub_filter_skips_plaintext() {
        assert!(is_sub("movie.srt"));
        assert!(!is_sub("notes.txt"));
    }

    #[test]
    fn title_of_preserves_trailing_separator() {
        assert_eq!(title_of("/tmp/media/"), "/tmp/media/");
    }

    #[test]
    fn expand_paths_matches_os_walk_file_order() {
        let dir = test_dir();
        let child = dir.join("01-child");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(dir.join("02-root.mkv"), []).unwrap();
        std::fs::write(child.join("01-child.mkv"), []).unwrap();

        let input = vec![dir.to_string_lossy().into_owned()];
        assert_eq!(
            expand_paths(&input),
            vec![
                dir.join("02-root.mkv").to_string_lossy().into_owned(),
                child.join("01-child.mkv").to_string_lossy().into_owned(),
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn expand_paths_absolutizes_missing_files_without_resolving_them() {
        let relative = format!("macplayer-missing-{}-file.mkv", std::process::id());
        assert_eq!(
            expand_paths(std::slice::from_ref(&relative)),
            vec![
                absolute_path(Path::new(&relative))
                    .to_string_lossy()
                    .into_owned()
            ]
        );
    }

    #[test]
    fn playlist_reader_replaces_invalid_utf8() {
        let dir = test_dir();
        let playlist = dir.join("items.m3u");
        std::fs::write(&playlist, b"\xef\xbb\xbfclip\xff.mp4\n").unwrap();
        assert_eq!(
            read_playlist_file(&playlist.to_string_lossy()),
            vec![dir.join("clip�.mp4").to_string_lossy().into_owned()]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn human_size_truncates_fractional_bytes() {
        assert_eq!(human_size(Some(1.9)), "1 B");
        assert_eq!(human_size(Some(-1.9)), "-1 B");
    }
}
