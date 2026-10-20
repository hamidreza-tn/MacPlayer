//! Persistent settings, one JSON file under `~/Library/Application Support/MacPlayer`.
//!
//! Defaults are merged over whatever is on disk; saves are atomic (write temp file, then rename).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub const MAX_BOOKMARKS: usize = 100;
pub const MAX_BOOKMARK_PATH_LEN: usize = 4096;
pub const MAX_BOOKMARK_TITLE_LEN: usize = 512;
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
}

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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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
            bookmarks: Vec::new(),
            track_preferences: Vec::new(),
            positions: Vec::new(),
            eq: vec![0.0; 10],
            eq_enabled: false,
            first_run: true,
            hdr_tone_mapping: false,
            hdr_tone_mapping_mode: "bt.2390".into(),
            fullscreen_monitor: 0,
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
        let Ok(mut current) = serde_json::to_value(Self::default()) else {
            return;
        };
        if let Json::Object(existing) = &mut current {
            for (key, value) in map {
                let previous = existing.insert(key.clone(), value);
                if serde_json::from_value::<Settings>(Json::Object(existing.clone())).is_err() {
                    if let Some(previous) = previous {
                        existing.insert(key, previous);
                    } else {
                        existing.remove(&key);
                    }
                }
            }
        }
        if let Ok(loaded) = serde_json::from_value::<Settings>(current) {
            *self = loaded;
        }
        self.normalize();
    }

    pub fn save(&self) {
        self.save_to(&config_file());
    }

    pub fn save_to(&self, path: &Path) {
        let mut saved = self.clone();
        saved.normalize();
        let Ok(text) = serde_json::to_string_pretty(&saved) else {
            return;
        };
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let (tmp, mut file) = loop {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let candidate = dir.join(format!(".settings-{}-{id}.tmp", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(file) => break (candidate, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return,
            }
        };
        if file.write_all(text.as_bytes()).is_err() || file.sync_all().is_err() {
            let _ = fs::remove_file(&tmp);
            return;
        }
        drop(file);
        if fs::rename(&tmp, path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
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

    pub fn bookmarks_for(&self, path: &str) -> impl Iterator<Item = &Bookmark> {
        self.bookmarks.iter().filter(move |item| item.path == path)
    }

    pub fn track_preference(&self, path: &str) -> Option<&TrackPreference> {
        self.track_preferences.iter().find(|item| item.path == path)
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
        self.track_preferences.retain(|item| item.path != path);
        self.track_preferences.push(TrackPreference {
            path: path.to_string(),
            audio,
            subtitle,
            secondary_subtitle,
        });
        while self.track_preferences.len() > 300 {
            self.track_preferences.remove(0);
        }
    }

    fn normalize(&mut self) {
        self.volume = self.volume.clamp(0, 100);
        self.window_w = sane_float(self.window_w, 806.0, 320.0, 10000.0);
        self.window_h = sane_float(self.window_h, 546.0, 200.0, 10000.0);
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
        self.sub_pos = self.sub_pos.clamp(0, 150);
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
    }
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
                    if let Value::Array(pair) = item {
                        if let (Some(key), Some(seconds)) = (pair.first(), pair.get(1)) {
                            if let (Some(key), Some(seconds)) = (key.as_str(), seconds.as_f64()) {
                                positions.push((key.to_string(), seconds));
                            }
                        }
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
    fn defaults_roundtrip() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, loaded);
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
        let mut settings = Settings::default();
        settings.volume = 37;
        settings.save_to(&path);
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
