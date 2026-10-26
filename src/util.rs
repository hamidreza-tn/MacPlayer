//! Small helpers: time formatting, media file detection, natural sorting.
//!
//! Path, media, playlist and time helpers used by MacPlayer.

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

pub const PLAYLIST_EXT: &[&str] = &[".m3u", ".m3u8", ".pls", ".cue", ".xspf"];

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
        return Some(percent.clamp(0.0, 100.0) / 100.0 * duration);
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

/// Folders become their media files recursively; nested symlinked folders are
/// deliberately skipped so selecting a directory cannot unexpectedly traverse
/// outside it or recurse through a large external volume.
pub fn expand_paths(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for path in paths {
        if is_url(path) {
            out.push(path.clone());
        } else if Path::new(path).is_dir() {
            let mut visited = std::collections::HashSet::new();
            collect_media_recursive(Path::new(path), &mut out, &mut visited);
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

fn collect_media_recursive(
    dir: &Path,
    out: &mut Vec<String>,
    visited: &mut std::collections::HashSet<PathBuf>,
) {
    // Canonicalized path as the cycle key: a symlink pointing back at an ancestor
    // (or at any already-walked directory) must not loop the walk forever.
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if !visited.insert(canonical) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut directories: Vec<(String, PathBuf)> = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
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
        collect_media_recursive(&path, out, visited);
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

/// Numbered siblings of `path` (ep1/ep2, track 01/02, ...): files of the same
/// kind whose name skeleton — digits collapsed to `#` — matches `path`'s.
pub fn similar_files(path: &str) -> Vec<String> {
    let directory = Path::new(path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    let name = title_of(path);
    let extension = ext(&name);
    let kind = if VIDEO_EXT.contains(&extension.as_str()) {
        VIDEO_EXT
    } else if AUDIO_EXT.contains(&extension.as_str()) {
        AUDIO_EXT
    } else {
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
            // Same skeleton rule for audio and video alike: numbered siblings
            // group together, un-numbered names match only themselves.
            if target.contains('#') {
                skeleton(candidate) == target
            } else {
                *candidate == name
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
    let is_xspf = ext(path) == ".xspf";
    if is_xspf {
        for location in text.split("<location>").skip(1) {
            let Some((location, _)) = location.split_once("</location>") else {
                continue;
            };
            let line = xml_unescape(location.trim());
            if line.is_empty() {
                continue;
            }
            if !is_url(&line) && !Path::new(&line).is_absolute() {
                entries.push(base.join(&line).to_string_lossy().into_owned());
            } else {
                entries.push(line);
            }
        }
        return entries;
    }
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
            line = line
                .split_once('=')
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
        }
        if !is_url(&line) && !Path::new(&line).is_absolute() {
            line = base.join(&line).to_string_lossy().into_owned();
        }
        entries.push(line);
    }
    entries
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Render an extended `.m3u8` playlist: one `#EXTINF` line (duration + title)
/// per item, then the path. Returns the number of bytes written.
pub fn write_m3u(path: &str, items: &[(String, String, Option<f64>)]) -> Result<usize, String> {
    let mut text = String::from("#EXTM3U\n");
    for (item_path, title, duration) in items {
        let safe_title = title.replace(['\r', '\n'], " ");
        let safe_path = item_path.replace(['\r', '\n'], " ");
        match duration {
            Some(seconds) if seconds.is_finite() && *seconds > 0.0 => {
                text.push_str(&format!("#EXTINF:{:.0},{}\n", seconds, safe_title));
            }
            _ => text.push_str(&format!("#EXTINF:-1,{}\n", safe_title)),
        }
        text.push_str(&safe_path);
        text.push('\n');
    }
    std::fs::write(path, text.as_bytes())
        .map(|_| text.len())
        .map_err(|error| format!("could not write playlist: {error}"))
}

/// Local "YYYY-MM-DD HH:MM" for a unix timestamp (no calendar crate needed).
pub fn format_unix_time(secs: i64) -> String {
    if secs <= 0 {
        return "—".into();
    }
    #[cfg(unix)]
    if let Some((year, month, day, hour, minute)) = local_components(secs) {
        return format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}");
    }
    format_unix_time_utc(secs)
}

fn format_unix_time_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

#[cfg(unix)]
#[repr(C)]
struct NativeTm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: isize,
    tm_zone: *const std::os::raw::c_char,
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn local_components(secs: i64) -> Option<(i64, u32, u32, u32, u32)> {
    unsafe extern "C" {
        fn localtime_r(timep: *const i64, result: *mut NativeTm) -> *mut NativeTm;
    }
    let mut tm = NativeTm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    let result = unsafe { localtime_r(&secs, &mut tm) };
    (!result.is_null()
        && (1..=12).contains(&(tm.tm_mon + 1))
        && (1..=31).contains(&tm.tm_mday)
        && (0..=23).contains(&tm.tm_hour)
        && (0..=59).contains(&tm.tm_min))
    .then_some((
        i64::from(tm.tm_year) + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
    ))
}

/// Locate an executable on PATH, if it exists as a regular file.
pub fn which(program: &str) -> Option<String> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
            .map(|candidate| candidate.to_string_lossy().into_owned())
    })
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (y, m, d).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_roundtrip_keeps_titles_and_durations() {
        let dir = std::env::temp_dir().join(format!("macplayer-m3u-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.m3u8");
        let items = vec![
            ("/abs/a.mkv".to_string(), "a".to_string(), Some(61.4)),
            ("https://example.test/v".to_string(), "v".to_string(), None),
        ];
        write_m3u(&path.to_string_lossy(), &items).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("#EXTM3U\n"));
        assert!(text.contains("#EXTINF:61,a\n"));
        assert!(text.contains("#EXTINF:-1,v\n"));
        // The local absolute path survives; the URL passes through untouched.
        let read_back = read_playlist_file(&path.to_string_lossy());
        assert!(read_back.contains(&"/abs/a.mkv".to_string()));
        assert!(read_back.contains(&"https://example.test/v".to_string()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn xspf_locations_are_loaded_relative_to_the_playlist() {
        let dir = std::env::temp_dir().join(format!("macplayer-xspf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("list.xspf");
        std::fs::write(
            &path,
            br#"<playlist><trackList><track><location>clip&amp;one.mkv</location></track></trackList></playlist>"#,
        )
        .unwrap();
        let entries = read_playlist_file(&path.to_string_lossy());
        assert_eq!(
            entries,
            vec![dir.join("clip&one.mkv").to_string_lossy().into_owned()]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn m3u_export_keeps_one_record_per_item() {
        let dir = std::env::temp_dir().join(format!("macplayer-m3u-safe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.m3u8");
        write_m3u(
            &path.to_string_lossy(),
            &[("/tmp/a\nb.mkv".into(), "title\r\nnext".into(), None)],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("title\nnext"));
        assert!(!text.contains("/tmp/a\nb.mkv"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unix_time_formats_as_civil_datetime() {
        assert_eq!(format_unix_time(0), "—");
        // 1792800000 = 2026-10-24 00:00 UTC (a whole number of days).
        assert_eq!(format_unix_time_utc(1_792_800_000), "2026-10-24 00:00");
        // Leap day: 2000-02-29 12:34 UTC.
        assert_eq!(format_unix_time_utc(951_827_640), "2000-02-29 12:34");
        assert_eq!(format_unix_time(1_792_800_000).len(), 16);
    }

    #[cfg(unix)]
    #[test]
    fn folder_expansion_walks_symlinks_without_looping() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!("macplayer-walk-{}", std::process::id()));
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.join("root.mkv"), b"x").unwrap();
        std::fs::write(nested.join("inner.mp4"), b"x").unwrap();
        // Nested symlinked folders are intentionally skipped...
        symlink(&nested, dir.join("link")).unwrap();
        // ...including cycles pointing back at the parent.
        symlink(&dir, nested.join("loop")).unwrap();

        let mut found = expand_paths(&[dir.to_string_lossy().into_owned()]);
        found.sort();
        let names: Vec<String> = found
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["inner.mp4", "root.mkv"]);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
