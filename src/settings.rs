//! Persistent settings, one JSON file under `~/Library/Application Support/MacPlayer`.
//!
//! Defaults are merged over whatever is on disk; saves are atomic (write temp file, then rename).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub const MAX_BOOKMARKS: usize = 100;
pub const MAX_BOOKMARK_PATH_LEN: usize = 4096;
pub const MAX_BOOKMARK_TITLE_LEN: usize = 512;
pub const SETTINGS_SCHEMA_VERSION: u32 = 2;
/// Upper bound for a stored fullscreen monitor index; the UI offers 0..=8.
const MAX_FULLSCREEN_MONITOR: usize = 64;

#[cfg(unix)]
struct SaveFileLock(std::fs::File);

#[cfg(unix)]
impl Drop for SaveFileLock {
    fn drop(&mut self) {
        unlock_file(&self.0);
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn lock_file(file: &std::fs::File) -> bool {
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    unsafe { flock(file.as_raw_fd(), 2) == 0 }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn unlock_file(file: &std::fs::File) {
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    let _ = unsafe { flock(file.as_raw_fd(), 8) };
}

fn default_schema_version() -> u32 {
    SETTINGS_SCHEMA_VERSION
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub path: String,
    pub time: f64,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackPreference {
    pub path: String,
    pub audio: i64,
    pub subtitle: i64,
    pub secondary_subtitle: i64,
    /// Last manually chosen aspect ratio (label + mpv value); restored on load.
    #[serde(default = "default_aspect_label")]
    pub aspect_label: String,
    #[serde(default = "default_aspect_value")]
    pub aspect_value: String,
    /// Last playback speed; `0.0` means "never set, do not restore".
    #[serde(default)]
    pub speed: f64,
}

fn default_aspect_label() -> String {
    "Default".into()
}

fn default_aspect_value() -> String {
    "-1".into()
}

/// One watched-file entry, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: String,
    pub title: String,
    pub position: f64,
    pub duration: f64,
    /// Unix seconds of when the file was (last) opened.
    pub played_at: i64,
}

pub const MAX_HISTORY: usize = 500;

impl Bookmark {
    pub fn new(path: impl Into<String>, time: f64, title: impl Into<String>) -> Option<Self> {
        let path = bounded_text(path.into(), MAX_BOOKMARK_PATH_LEN);
        if path.is_empty() || !time.is_finite() || time < 0.0 {
            return None;
        }
        Some(Self {
            path,
            time,
            title: bounded_text(title.into(), MAX_BOOKMARK_TITLE_LEN),
        })
    }
}

/// Every field the app reads, with its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Version of the persisted settings schema. Older files are migrated on load.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub volume: i64,
    pub mute: bool,
    pub window_w: f64,
    pub window_h: f64,
    pub window_x: Option<f64>,
    pub window_y: Option<f64>,
    pub playlist_visible: bool,
    pub playlist_width: f64,
    /// "never" | "always" | "playing"
    pub on_top: String,
    pub autofit: bool,
    pub autofit_max_pct: i64,
    pub resume: bool,
    pub show_remaining: bool,
    /// "off" | "one" | "all"
    #[serde(rename = "repeat", alias = "repeat_mode")]
    pub repeat_mode: String,
    pub shuffle: bool,
    pub seek_small: f64,
    pub seek_medium: f64,
    pub seek_large: f64,
    pub seek_huge: f64,
    pub volume_step: f64,
    pub hwdec: bool,
    pub sub_font: String,
    pub sub_size: i64,
    pub sub_bold: bool,
    /// Used only when a file is not valid UTF-8.
    pub sub_codepage: String,
    pub sub_auto: String,
    pub sub_pos: i64,
    pub sub_visible: bool,
    pub preferred_sub_lang: String,
    pub preferred_audio_lang: String,
    pub screenshot_dir: String,
    pub last_dir: String,
    pub last_sub_dir: String,
    pub recent: Vec<String>,
    pub playlist_paths: Vec<String>,
    pub playlist_current: Option<String>,
    pub bookmarks: Vec<Bookmark>,
    pub track_preferences: Vec<TrackPreference>,
    /// Resume positions, kept in insertion order so eviction drops the oldest entry
    /// (a plain map would evict by sort order, which is not what the player needs).
    #[serde(with = "ordered_positions")]
    pub positions: Vec<(String, f64)>,
    pub eq: Vec<f64>,
    pub eq_enabled: bool,
    pub first_run: bool,
    pub hdr_tone_mapping: bool,
    pub hdr_tone_mapping_mode: String,
    pub fullscreen_monitor: usize,
    /// UI appearance: "dark" or "light".
    pub theme: String,
    /// Accent color id: "gold", "blue", "green", "rose" or "graphite".
    pub accent: String,
    /// UI language: "en" or "fa".
    pub language: String,
    /// Upper bound of the volume slider, percent (100 = no boost, up to 200).
    pub volume_max: i64,
    /// "none" | "file" | "playlist" — mpv loop-file / loop-playlist.
    pub loop_mode: String,
    /// Keep the audio pitch when changing playback speed (mpv scaletempo).
    pub audio_pitch_correction: bool,
    /// Comma list of bitstream formats passed to the output device
    /// (ac3,dts,eac3,truehd); empty = no passthrough.
    pub audio_spdif: String,
    /// Hold the audio device exclusively while playing.
    pub audio_exclusive: bool,
    /// "auto" | "stereo" | "reverse" | "5.1" | "7.1".
    pub audio_channels: String,
    /// "auto" | "audio" | "display-resync" | "display-desync".
    pub video_sync: String,
    /// Interpolate frames when resyncing to the display.
    pub interpolation: bool,
    /// "bilinear" | "bicubic" | "spline36" | "lanczos" | "ewa_lanczossharp".
    pub scale_profile: String,
    /// Stream cache seconds; 0 keeps mpv's automatic sizing.
    pub cache_secs: i64,
    /// Demuxer cache in MiB; 0 keeps mpv's automatic sizing.
    pub demuxer_max_mb: i64,
    /// User-Agent for HTTP(S) streams.
    pub http_user_agent: String,
    /// Extra HTTP headers, one `Key: Value` per line.
    pub http_headers: String,
    /// Preferred yt-dlp quality: best|2160|1440|1080|720|480|360|audio.
    pub ytdl_quality: String,
    /// Subtitle text color, "#RRGGBB".
    pub sub_color: String,
    /// Subtitle border width in pixels.
    pub sub_border_size: f64,
    /// Subtitle border color, "#RRGGBB".
    pub sub_border_color: String,
    /// Subtitle shadow offset; 0 disables the shadow.
    pub sub_shadow_offset: f64,
    /// Draw a box behind subtitles.
    pub sub_back_enable: bool,
    /// Subtitle background color, "#RRGGBB".
    pub sub_back_color: String,
    /// Audio visualizer for music files: "off" | "wave" | "spectrum".
    pub visualizer: String,
    /// Start the visualizer automatically for audio-only files.
    pub auto_visualizer: bool,
    /// Watched-file history, newest first.
    pub history: Vec<HistoryEntry>,
    /// Show a "Continue watching" card row on the idle/welcome screen.
    pub continue_watching: bool,
    /// Extract and display embedded cover art for audio files.
    pub cover_art: bool,
    /// Publish playback state to the macOS Now Playing (Control Center) surface.
    pub now_playing: bool,
    /// Check GitHub for a newer release at startup.
    pub update_check: bool,
    /// Show hover thumbnails above the seek bar.
    pub hover_thumbnails: bool,
    /// Show video previews on playlist rows.
    pub row_previews: bool,
    /// Custom accent color, "#rrggbb"; used when `accent` == "custom".
    pub accent_custom: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            volume: 100,
            mute: false,
            window_w: 1100.0,
            window_h: 680.0,
            window_x: None,
            window_y: None,
            playlist_visible: true,
            playlist_width: 280.0,
            on_top: "never".into(),
            autofit: true,
            autofit_max_pct: 85,
            resume: true,
            show_remaining: false,
            repeat_mode: "off".into(),
            shuffle: false,
            seek_small: 5.0,
            seek_medium: 30.0,
            seek_large: 60.0,
            seek_huge: 300.0,
            volume_step: 5.0,
            hwdec: true,
            sub_font: String::new(),
            sub_size: 44,
            sub_bold: true,
            sub_codepage: "cp1256".into(),
            sub_auto: "fuzzy".into(),
            sub_pos: 100,
            sub_visible: true,
            preferred_sub_lang: String::new(),
            preferred_audio_lang: String::new(),
            screenshot_dir: String::new(),
            last_dir: String::new(),
            last_sub_dir: String::new(),
            recent: Vec::new(),
            playlist_paths: Vec::new(),
            playlist_current: None,
            bookmarks: Vec::new(),
            track_preferences: Vec::new(),
            positions: Vec::new(),
            eq: vec![0.0; 10],
            eq_enabled: false,
            first_run: true,
            hdr_tone_mapping: false,
            hdr_tone_mapping_mode: "bt.2390".into(),
            fullscreen_monitor: 0,
            theme: "dark".into(),
            accent: "gold".into(),
            language: "en".into(),
            volume_max: 100,
            loop_mode: "none".into(),
            audio_pitch_correction: true,
            audio_spdif: String::new(),
            audio_exclusive: false,
            audio_channels: "auto".into(),
            video_sync: "auto".into(),
            interpolation: false,
            scale_profile: "bilinear".into(),
            cache_secs: 0,
            demuxer_max_mb: 0,
            http_user_agent: String::new(),
            http_headers: String::new(),
            ytdl_quality: "best".into(),
            sub_color: "#ffffff".into(),
            sub_border_size: 3.0,
            sub_border_color: "#000000".into(),
            sub_shadow_offset: 0.0,
            sub_back_enable: false,
            sub_back_color: "#000000".into(),
            visualizer: "off".into(),
            auto_visualizer: true,
            history: Vec::new(),
            continue_watching: true,
            cover_art: true,
            now_playing: true,
            update_check: true,
            hover_thumbnails: true,
            row_previews: true,
            accent_custom: String::new(),
        }
    }
}

/// Platform-appropriate configuration directory.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("MacPlayer");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("macplayer");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join("macplayer");
        }
    }
    PathBuf::from("macplayer")
}

pub fn config_file() -> PathBuf {
    config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        let path = config_file();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(text) = fs::read_to_string(path) else {
            return settings;
        };
        let Ok(Json::Object(map)) = serde_json::from_str::<Json>(&text) else {
            return settings;
        };
        settings.merge(map);
        settings
    }

    /// Overlay a JSON object on top of the defaults.  Invalid individual values
    /// are ignored so one damaged setting does not discard every valid setting.
    fn merge(&mut self, mut map: serde_json::Map<String, Json>) {
        // Older Rust builds called this field `repeat_mode`; keep those files
        // usable while writing the canonical name on the next save.
        // Removing the alias before deserializing also avoids serde treating
        // both spellings as duplicate values when a hand-edited file contains
        // both.
        if let Some(value) = map.remove("repeat_mode")
            && !map.contains_key("repeat")
        {
            map.insert("repeat".into(), value);
        }
        // Fast path: overlay everything at once. Only a file that fails to
        // deserialize as a whole pays for the per-key pass below.
        if let Some(merged) = Self::try_merge(&map) {
            *self = merged;
            self.normalize();
            return;
        }
        let mut current = match serde_json::to_value(Self::default()) {
            Ok(Json::Object(existing)) => existing,
            _ => return,
        };
        for (key, value) in map {
            let previous = current.insert(key.clone(), value);
            if serde_json::from_value::<Settings>(Json::Object(current.clone())).is_err() {
                if let Some(previous) = previous {
                    current.insert(key, previous);
                } else {
                    current.remove(&key);
                }
            }
        }
        if let Ok(loaded) = serde_json::from_value::<Settings>(Json::Object(current)) {
            *self = loaded;
        }
        self.normalize();
    }

    /// One-shot overlay of `map` on the defaults; `None` when the result does
    /// not deserialize (the caller then falls back to per-key reconciliation).
    fn try_merge(map: &serde_json::Map<String, Json>) -> Option<Settings> {
        let mut current = serde_json::to_value(Self::default()).ok()?;
        if let Json::Object(existing) = &mut current {
            for (key, value) in map {
                existing.insert(key.clone(), value.clone());
            }
        }
        serde_json::from_value::<Settings>(current).ok()
    }

    /// Apply a JSON object patch (the CLI `settings set` path). Returns false
    /// when the key is unknown or the value type is wrong; the merge pipeline
    /// (per-key reconciliation + normalize) keeps everything else valid.
    pub fn apply_patch(&mut self, map: serde_json::Map<String, Json>) -> bool {
        let Some((key, requested)) = map.into_iter().next() else {
            return false;
        };
        let single = serde_json::Map::from_iter([(key.clone(), requested.clone())]);
        let mut probe = self.clone();
        probe.merge(single);
        let applied = serde_json::to_value(&probe)
            .ok()
            .and_then(|value| value.get(&key).cloned())
            .is_some_and(|after| after == requested);
        if applied {
            *self = probe;
        }
        applied
    }

    pub fn try_save(&self) -> Result<(), String> {
        self.try_save_to(&config_file())
    }

    pub fn try_save_to(&self, path: &Path) -> Result<(), String> {
        let mut saved = self.clone();
        saved.normalize();
        let text = serde_json::to_string_pretty(&saved)
            .map_err(|error| format!("could not encode settings: {error}"))?;
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir)
            .map_err(|error| format!("could not create settings directory: {error}"))?;
        #[cfg(unix)]
        if let Ok(metadata) = fs::metadata(dir) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o700);
            let _ = fs::set_permissions(dir, permissions);
        }
        #[cfg(unix)]
        let _save_lock = {
            let lock_path = dir.join(".settings.lock");
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            options.mode(0o600);
            let lock_handle = options
                .open(&lock_path)
                .map_err(|error| format!("could not open settings lock: {error}"))?;
            if !lock_file(&lock_handle) {
                return Err("could not lock settings file".into());
            }
            SaveFileLock(lock_handle)
        };
        let (tmp, mut file) = loop {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let candidate = dir.join(format!(".settings-{}-{id}.tmp", std::process::id()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(&candidate) {
                Ok(file) => break (candidate, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!("could not create temporary settings file: {error}"));
                }
            }
        };
        if let Err(error) = file
            .write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&tmp);
            return Err(format!("could not write settings: {error}"));
        }
        drop(file);
        if let Err(error) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("could not replace settings file: {error}"));
        }
        // Sync the directory entry too, so a crash right after the rename cannot
        // leave the new settings file unrecorded on disk.
        if let Ok(dir_file) = fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
        Ok(())
    }

    // -- recent files and resume positions ------------------------------------------------------

    pub fn add_recent(&mut self, path: &str) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_string());
        self.recent.truncate(20);
    }

    pub fn set_position(&mut self, path: &str, seconds: Option<f64>) {
        self.positions.retain(|(key, _)| key != path);
        if let Some(seconds) = seconds.filter(|seconds| seconds.is_finite()) {
            self.positions
                .push((path.to_string(), round_half_even(seconds, 1)));
            while self.positions.len() > 300 {
                self.positions.remove(0);
            }
        }
    }

    pub fn get_position(&self, path: &str) -> Option<f64> {
        self.positions
            .iter()
            .find(|(key, _)| key == path)
            .map(|(_, seconds)| *seconds)
    }

    pub fn add_bookmark(&mut self, path: &str, time: f64, title: &str) -> bool {
        let Some(bookmark) = Bookmark::new(path, time, title) else {
            return false;
        };
        self.bookmarks
            .retain(|item| item.path != bookmark.path || item.time != bookmark.time);
        self.bookmarks.push(bookmark);
        while self.bookmarks.len() > MAX_BOOKMARKS {
            self.bookmarks.remove(0);
        }
        true
    }

    pub fn remove_bookmark(&mut self, path: &str, time: f64) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks
            .retain(|item| item.path != path || item.time != time);
        before != self.bookmarks.len()
    }

    pub fn add_history(&mut self, path: &str, title: &str) {
        if path.is_empty() {
            return;
        }
        self.history.retain(|entry| entry.path != path);
        self.history.insert(
            0,
            HistoryEntry {
                path: path.to_string(),
                title: title_of_file(title),
                position: 0.0,
                duration: 0.0,
                played_at: unix_now(),
            },
        );
        self.history.truncate(MAX_HISTORY);
    }

    /// Refresh the position/duration of the newest entry for `path`.
    pub fn update_history(&mut self, path: &str, position: f64, duration: f64) {
        if let Some(entry) = self.history.iter_mut().find(|entry| entry.path == path) {
            if position.is_finite() {
                entry.position = position.max(0.0);
            }
            if duration.is_finite() && duration > 0.0 {
                entry.duration = duration;
            }
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn track_preference(&self, path: &str) -> Option<&TrackPreference> {
        self.track_preferences.iter().find(|item| item.path == path)
    }

    /// Patch the aspect/speed part of a file's preference, creating the entry
    /// (with neutral track codes) when it does not exist yet.
    pub fn set_media_preference(
        &mut self,
        path: &str,
        aspect_label: &str,
        aspect_value: &str,
        speed: f64,
    ) {
        if path.is_empty() {
            return;
        }
        let mut entry = self
            .track_preferences
            .iter()
            .find(|item| item.path == path)
            .cloned()
            .unwrap_or(TrackPreference {
                path: path.to_string(),
                audio: -2,
                subtitle: -2,
                secondary_subtitle: -1,
                aspect_label: default_aspect_label(),
                aspect_value: default_aspect_value(),
                speed: 0.0,
            });
        entry.aspect_label = aspect_label.to_string();
        entry.aspect_value = aspect_value.to_string();
        if speed.is_finite() && speed > 0.0 {
            entry.speed = speed;
        }
        self.track_preferences.retain(|item| item.path != path);
        self.track_preferences.push(entry);
        while self.track_preferences.len() > 300 {
            self.track_preferences.remove(0);
        }
    }

    pub fn set_track_preference(
        &mut self,
        path: &str,
        audio: i64,
        subtitle: i64,
        secondary_subtitle: i64,
    ) {
        if path.is_empty() {
            return;
        }
        // Preserve the aspect/speed part of an existing entry while replacing
        // the track selection.
        let media = self
            .track_preferences
            .iter()
            .find(|item| item.path == path)
            .map(|item| {
                (
                    item.aspect_label.clone(),
                    item.aspect_value.clone(),
                    item.speed,
                )
            });
        let (aspect_label, aspect_value, speed) =
            media.unwrap_or_else(|| (default_aspect_label(), default_aspect_value(), 0.0));
        self.track_preferences.retain(|item| item.path != path);
        self.track_preferences.push(TrackPreference {
            path: path.to_string(),
            audio,
            subtitle,
            secondary_subtitle,
            aspect_label,
            aspect_value,
            speed,
        });
        while self.track_preferences.len() > 300 {
            self.track_preferences.remove(0);
        }
    }

    fn normalize(&mut self) {
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        self.volume_max = self.volume_max.clamp(100, 200);
        self.volume = self.volume.clamp(0, self.volume_max);
        self.window_w = sane_float(self.window_w, 1100.0, 320.0, 10000.0);
        self.window_h = sane_float(self.window_h, 680.0, 200.0, 10000.0);
        self.window_x = self.window_x.filter(|value| value.is_finite());
        self.window_y = self.window_y.filter(|value| value.is_finite());
        self.playlist_width = sane_float(self.playlist_width, 280.0, 160.0, 1000.0);
        if !matches!(self.on_top.as_str(), "never" | "always" | "playing") {
            self.on_top = "never".into();
        }
        self.autofit_max_pct = self.autofit_max_pct.clamp(30, 100);
        if !matches!(self.repeat_mode.as_str(), "off" | "one" | "all") {
            self.repeat_mode = "off".into();
        }
        self.seek_small = sane_float(self.seek_small, 5.0, 0.1, 3600.0);
        self.seek_medium = sane_float(self.seek_medium, 30.0, 0.1, 3600.0);
        self.seek_large = sane_float(self.seek_large, 60.0, 0.1, 3600.0);
        self.seek_huge = sane_float(self.seek_huge, 300.0, 0.1, 3600.0);
        self.volume_step = sane_float(self.volume_step, 5.0, 1.0, 20.0);
        self.sub_size = self.sub_size.clamp(10, 150);
        self.sub_pos = self.sub_pos.clamp(0, 100);
        if !matches!(self.sub_auto.as_str(), "exact" | "fuzzy" | "all" | "no") {
            self.sub_auto = "fuzzy".into();
        }
        self.sub_codepage = if self.sub_codepage.trim().is_empty() {
            "auto".into()
        } else {
            self.sub_codepage.trim().trim_start_matches('+').to_string()
        };
        self.recent.truncate(20);
        self.bookmarks.retain(|bookmark| {
            !bookmark.path.trim().is_empty() && bookmark.time.is_finite() && bookmark.time >= 0.0
        });
        for bookmark in &mut self.bookmarks {
            bookmark.path = bounded_text(bookmark.path.clone(), MAX_BOOKMARK_PATH_LEN);
            bookmark.title = bounded_text(bookmark.title.clone(), MAX_BOOKMARK_TITLE_LEN);
        }
        while self.bookmarks.len() > MAX_BOOKMARKS {
            self.bookmarks.remove(0);
        }
        self.track_preferences.retain(|item| !item.path.is_empty());
        self.track_preferences.truncate(300);
        self.positions
            .retain(|(path, seconds)| !path.is_empty() && seconds.is_finite());
        while self.positions.len() > 300 {
            self.positions.remove(0);
        }
        self.eq.retain(|gain| gain.is_finite());
        self.eq.truncate(10);
        self.eq.resize(10, 0.0);
        for gain in &mut self.eq {
            *gain = gain.clamp(-12.0, 12.0);
        }
        if !matches!(
            self.hdr_tone_mapping_mode.as_str(),
            "bt.2390" | "reinhard" | "mobius" | "hable"
        ) {
            self.hdr_tone_mapping_mode = "bt.2390".into();
        }
        // The preferences UI exposes monitors 0..=8; accept a little headroom for
        // larger multi-monitor setups but reject absurd indices from a corrupt file.
        self.fullscreen_monitor = self.fullscreen_monitor.min(MAX_FULLSCREEN_MONITOR);
        if !matches!(self.theme.as_str(), "dark" | "light" | "auto") {
            self.theme = "dark".into();
        }
        if !matches!(
            self.accent.as_str(),
            "gold" | "blue" | "green" | "rose" | "graphite"
        ) {
            self.accent = "gold".into();
        }
        if !matches!(self.language.as_str(), "en" | "fa") {
            self.language = "en".into();
        }
        if !matches!(self.loop_mode.as_str(), "none" | "file" | "playlist") {
            self.loop_mode = "none".into();
        }
        // Keep the list inside the formats mpv knows how to bitstream.
        self.audio_spdif = self
            .audio_spdif
            .split(',')
            .map(str::trim)
            .filter(|token| matches!(*token, "ac3" | "dts" | "eac3" | "truehd" | "dts-hd"))
            .collect::<Vec<_>>()
            .join(",");
        if !matches!(
            self.audio_channels.as_str(),
            "auto" | "stereo" | "reverse" | "5.1" | "7.1"
        ) {
            self.audio_channels = "auto".into();
        }
        if !matches!(
            self.video_sync.as_str(),
            "auto" | "audio" | "display-resync" | "display-desync"
        ) {
            self.video_sync = "auto".into();
        }
        if !matches!(
            self.scale_profile.as_str(),
            "bilinear" | "bicubic" | "spline36" | "lanczos" | "ewa_lanczossharp"
        ) {
            self.scale_profile = "bilinear".into();
        }
        self.cache_secs = self.cache_secs.clamp(0, 600);
        self.demuxer_max_mb = self.demuxer_max_mb.clamp(0, 4096);
        if !matches!(
            self.ytdl_quality.as_str(),
            "best" | "2160" | "1440" | "1080" | "720" | "480" | "360" | "audio"
        ) {
            self.ytdl_quality = "best".into();
        }
        self.sub_color = sane_hex_color(&self.sub_color, "#ffffff");
        self.sub_border_color = sane_hex_color(&self.sub_border_color, "#000000");
        self.sub_back_color = sane_hex_color(&self.sub_back_color, "#000000");
        self.sub_border_size = sane_float(self.sub_border_size, 3.0, 0.0, 10.0);
        self.sub_shadow_offset = sane_float(self.sub_shadow_offset, 0.0, 0.0, 10.0);
        if !matches!(self.visualizer.as_str(), "off" | "wave" | "spectrum") {
            self.visualizer = "off".into();
        }
        self.history.truncate(MAX_HISTORY);
        self.history.retain(|entry| !entry.path.is_empty());
        self.accent_custom = sane_hex_color(&self.accent_custom, "");
    }
}

/// Clamp a "#rrggbb" string back to a valid color or fall back to `fallback`.
fn sane_hex_color(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    let valid = trimmed.len() == 7
        && trimmed.starts_with('#')
        && trimmed[1..].chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        trimmed.to_lowercase()
    } else {
        fallback.to_string()
    }
}

/// File name part of a path (history titles are shown without directories).
fn title_of_file(title: &str) -> String {
    std::path::Path::new(title)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| title.to_string())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

fn sane_float(value: f64, default: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

fn bounded_text(value: String, max_len: usize) -> String {
    value.trim().chars().take(max_len).collect()
}

/// Round half to even for stable cross-platform settings files.
/// Rust's `f64::round()` rounds halves away from zero, which disagrees on ties.
fn round_half_even(value: f64, digits: i32) -> f64 {
    if !value.is_finite() || value == 0.0 || !(0..=18).contains(&digits) {
        // `set_position` only calls this with one decimal place.  Retain a
        // sensible fallback for the private helper's other inputs without
        // letting non-finite values turn into invalid JSON.
        if !value.is_finite() || value == 0.0 {
            return value;
        }
        let factor = 10f64.powi(digits);
        return (value * factor).round() / factor;
    }

    // Multiplying by ten first loses the direction of values near a decimal
    // tie (for example, 0.05 becomes exactly 0.5 as an f64, while 1.15 also
    // becomes exactly 11.5).  Compare the original binary float with
    // the decimal midpoint.  Keep that comparison exact for the non-negative
    // precisions used by settings, using the f64 significand and powers of two.
    let negative = value.is_sign_negative();
    let bits = value.abs().to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if exponent_bits == 0 {
        (fraction as u128, -1022 - 52)
    } else {
        (((1u64 << 52) | fraction) as u128, exponent_bits - 1023 - 52)
    };
    let scale = 5u128.pow(digits as u32);
    let numerator = mantissa * scale;
    let binary_exponent = exponent + digits;

    // A non-negative binary exponent means the scaled value is already an
    // integer.  With a very small subnormal, the scaled value is below one
    // half and rounds to signed zero.
    let (quotient, remainder, denominator) = if binary_exponent >= 0 {
        return value;
    } else {
        let shift = (-binary_exponent) as u32;
        if shift >= 128 {
            return if negative { -0.0 } else { 0.0 };
        }
        let denominator = 1u128 << shift;
        (
            numerator / denominator,
            numerator % denominator,
            denominator,
        )
    };
    let rounded = quotient
        + u128::from(
            remainder > denominator / 2 || (remainder * 2 == denominator && quotient % 2 == 1),
        );
    let result = rounded as f64 / 10f64.powi(digits);
    if negative { -result } else { result }
}

/// Serialise `positions` as a JSON object while keeping insertion order in memory.
mod ordered_positions {
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(
        positions: &[(String, f64)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(positions.len()))?;
        for (key, seconds) in positions {
            map.serialize_entry(key, seconds)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<(String, f64)>, D::Error> {
        // Accept both the current object form and the historical array-of-pairs form.
        let value = Value::deserialize(deserializer)?;
        let mut positions = Vec::new();
        match value {
            Value::Object(map) => {
                for (key, seconds) in map {
                    if let Some(seconds) = seconds.as_f64() {
                        positions.push((key, seconds));
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    if let Value::Array(pair) = item
                        && let (Some(key), Some(seconds)) = (pair.first(), pair.get(1))
                        && let (Some(key), Some(seconds)) = (key.as_str(), seconds.as_f64())
                    {
                        positions.push((key.to_string(), seconds));
                    }
                }
            }
            _ => {}
        }
        Ok(positions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("macplayer-settings-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn history_moves_replayed_files_to_the_front_and_is_capped() {
        let mut settings = Settings::default();
        settings.add_history("/tmp/a.mkv", "/tmp/a.mkv");
        settings.add_history("/tmp/b.mkv", "/tmp/b.mkv");
        settings.add_history("/tmp/a.mkv", "/tmp/a.mkv");
        assert_eq!(settings.history[0].path, "/tmp/a.mkv");
        assert_eq!(settings.history.len(), 2);
        settings.update_history("/tmp/b.mkv", 12.5, 120.0);
        assert_eq!(settings.history[1].position, 12.5);
        assert_eq!(settings.history[1].duration, 120.0);
        for i in 0..(MAX_HISTORY + 10) {
            settings.add_history(&format!("/tmp/{i}.mkv"), &format!("/tmp/{i}.mkv"));
        }
        assert_eq!(settings.history.len(), MAX_HISTORY);
        settings.clear_history();
        assert!(settings.history.is_empty());
    }

    #[test]
    fn media_preference_roundtrip_preserves_track_selection() {
        let mut settings = Settings::default();
        settings.set_track_preference("/tmp/a.mkv", 2, 3, -1);
        settings.set_media_preference("/tmp/a.mkv", "16:9", "16:9", 1.25);
        let preference = settings.track_preference("/tmp/a.mkv").unwrap();
        assert_eq!(preference.audio, 2);
        assert_eq!(preference.subtitle, 3);
        assert_eq!(preference.aspect_label, "16:9");
        assert_eq!(preference.aspect_value, "16:9");
        assert_eq!(preference.speed, 1.25);
    }

    #[test]
    fn old_track_preference_files_still_load() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            br#"{"track_preferences":[{"path":"/tmp/a.mkv","audio":2,"subtitle":3,
                 "secondary_subtitle":-1}]}"#,
        )
        .unwrap();
        let settings = Settings::load_from(&path);
        let preference = settings.track_preference("/tmp/a.mkv").unwrap();
        assert_eq!(preference.aspect_label, "Default");
        assert_eq!(preference.aspect_value, "-1");
        assert_eq!(preference.speed, 0.0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_audio_and_network_settings_are_normalized() {
        let mut settings = Settings {
            volume_max: 500,
            audio_spdif: "ac3, bogus, dts".into(),
            audio_channels: "quad".into(),
            video_sync: "weird".into(),
            scale_profile: "nnedi".into(),
            ytdl_quality: "9999".into(),
            sub_color: "red".into(),
            cache_secs: 999_999,
            http_headers: "Referer: x, Cookie: y".into(),
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.volume_max, 200);
        assert_eq!(settings.audio_spdif, "ac3,dts");
        assert_eq!(settings.audio_channels, "auto");
        assert_eq!(settings.video_sync, "auto");
        assert_eq!(settings.scale_profile, "bilinear");
        assert_eq!(settings.ytdl_quality, "best");
        assert_eq!(settings.sub_color, "#ffffff");
        assert_eq!(settings.cache_secs, 600);
        assert_eq!(settings.http_headers, "Referer: x, Cookie: y");
    }

    #[test]
    fn volume_boost_is_preserved_within_the_configured_ceiling() {
        let mut settings = Settings {
            volume: 175,
            volume_max: 200,
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.volume, 175);
        assert_eq!(settings.volume_max, 200);
    }

    #[test]
    fn defaults_roundtrip() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, loaded);
    }

    #[test]
    fn settings_use_current_schema_version() {
        let settings = Settings::default();
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["schema_version"], SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn recent_is_capped() {
        let mut settings = Settings::default();
        for i in 0..30 {
            settings.add_recent(&format!("/tmp/{i}.mkv"));
        }
        assert_eq!(settings.recent.len(), 20);
        assert_eq!(settings.recent[0], "/tmp/29.mkv");
    }

    #[test]
    fn positions_are_capped() {
        let mut settings = Settings::default();
        for i in 0..400 {
            settings.set_position(&format!("/tmp/{i}.mkv"), Some(1.25));
        }
        assert_eq!(settings.positions.len(), 300);
        assert_eq!(settings.get_position("/tmp/100.mkv"), Some(1.2));
    }

    #[test]
    fn position_rounding_handles_decimal_ties() {
        let mut settings = Settings::default();
        settings.set_position("above", Some(0.05));
        settings.set_position("below", Some(1.15));
        settings.set_position("negative", Some(-0.05));
        assert_eq!(settings.get_position("above"), Some(0.1));
        assert_eq!(settings.get_position("below"), Some(1.1));
        assert_eq!(settings.get_position("negative"), Some(-0.1));
    }

    #[test]
    fn non_finite_positions_are_removed_instead_of_poisoning_json() {
        let mut settings = Settings::default();
        settings.set_position("clip", Some(12.0));
        settings.set_position("clip", Some(f64::NAN));
        assert_eq!(settings.get_position("clip"), None);
        assert!(serde_json::to_string(&settings).is_ok());
    }

    #[test]
    fn codepage_is_normalised() {
        let mut settings = Settings::default();
        let map =
            serde_json::from_str::<serde_json::Map<String, Json>>(r#"{"sub_codepage": "+1256"}"#)
                .unwrap();
        settings.merge(map);
        assert_eq!(settings.sub_codepage, "1256");
    }

    #[test]
    fn loads_repeat_key() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{"repeat":"all","volume":42}"#).unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.repeat_mode, "all");
        assert_eq!(settings.volume, 42);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_repeat_with_canonical_key() {
        let json = serde_json::to_value(Settings {
            repeat_mode: "all".into(),
            ..Settings::default()
        })
        .unwrap();
        assert_eq!(json["repeat"], "all");
        assert!(json.get("repeat_mode").is_none());
    }

    #[test]
    fn invalid_setting_does_not_discard_valid_values() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{"volume":42,"mute":"invalid"}"#).unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.volume, 42);
        assert!(!settings.mute);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_numeric_ranges_are_normalized() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            br#"{"volume":999,"window_w":-1,"window_h":null,"eq":[1.0]}"#,
        )
        .unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.volume, 100);
        assert_eq!(settings.window_w, 320.0);
        assert_eq!(settings.window_h, 680.0);
        assert_eq!(settings.eq.len(), 10);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fullscreen_monitor_index_is_bounded() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{"fullscreen_monitor":4000000}"#).unwrap();
        let settings = Settings::load_from(&path);
        assert!(settings.fullscreen_monitor <= 64);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn appearance_settings_survive_corrupt_values() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            br#"{"theme":"neon","accent":"purple","language":"de"}"#,
        )
        .unwrap();
        let settings = Settings::load_from(&path);
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.accent, "gold");
        assert_eq!(settings.language, "en");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn positions_are_saved_as_ordered_object() {
        let mut settings = Settings::default();
        settings.set_position("/tmp/first.mkv", Some(1.25));
        settings.set_position("/tmp/second.mkv", Some(2.35));
        let json = serde_json::to_value(&settings).unwrap();
        let positions = json.get("positions").unwrap();
        assert!(positions.is_object());
        assert_eq!(positions["/tmp/first.mkv"], 1.2);
        assert_eq!(positions["/tmp/second.mkv"], 2.4);

        let loaded: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.get_position("/tmp/first.mkv"), Some(1.2));
        assert_eq!(loaded.get_position("/tmp/second.mkv"), Some(2.4));
    }

    #[test]
    fn loaded_position_order_is_kept_for_oldest_eviction() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            br#"{"positions":{"/tmp/z.mkv":1.0,"/tmp/a.mkv":2.0}}"#,
        )
        .unwrap();
        let mut settings = Settings::load_from(&path);
        settings.set_position("/tmp/new.mkv", Some(3.0));
        assert_eq!(settings.positions[0].0, "/tmp/z.mkv");
        assert_eq!(settings.positions[1].0, "/tmp/a.mkv");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_to_uses_unique_atomic_temp_file() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let settings = Settings {
            volume: 37,
            ..Default::default()
        };
        settings.try_save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path).volume, 37);
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".settings-"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn try_save_to_reports_success() {
        let dir = test_dir();
        let path = dir.join("settings.json");
        let result = Settings::default().try_save_to(&path);
        assert!(result.is_ok());
        assert!(path.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bookmarks_are_capped_and_keep_the_newest_entries() {
        let mut settings = Settings::default();
        for index in 0..(MAX_BOOKMARKS + 2) {
            assert!(settings.add_bookmark("/tmp/movie.mkv", index as f64, "Scene"));
        }
        assert_eq!(settings.bookmarks.len(), MAX_BOOKMARKS);
        assert_eq!(settings.bookmarks.first().unwrap().time, 2.0);
        assert_eq!(
            settings.bookmarks.last().unwrap().time,
            (MAX_BOOKMARKS + 1) as f64
        );
    }

    #[test]
    fn bookmarks_replace_by_path_and_time_and_roundtrip() {
        let mut settings = Settings::default();
        assert!(settings.add_bookmark(" /tmp/movie.mkv ", 12.5, "Old"));
        assert!(settings.add_bookmark("/tmp/movie.mkv", 12.5, "New"));
        assert_eq!(settings.bookmarks.len(), 1);
        assert_eq!(settings.bookmarks[0].title, "New");
        let json = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.bookmarks, settings.bookmarks);
    }

    #[test]
    fn invalid_bookmarks_are_rejected_and_removed_on_load() {
        let mut settings = Settings::default();
        assert!(!settings.add_bookmark("", 1.0, "Scene"));
        assert!(!settings.add_bookmark("/tmp/movie.mkv", f64::NAN, "Scene"));
        let dir = test_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            br#"{"bookmarks":[{"path":"/tmp/movie.mkv","time":-1.0,"title":"Bad"}]}"#,
        )
        .unwrap();
        assert!(Settings::load_from(&path).bookmarks.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
