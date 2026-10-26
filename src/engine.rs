//! One mpv core: creation, property/command facade and the event thread.
//!
//! All messages leave mpv through a channel that the UI drains once per frame, so no mpv
//! callback ever touches egui state directly.
#![allow(unsafe_code)]

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_void};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::ffi::{
    self, Api, MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_LOG_MESSAGE, MPV_EVENT_NONE,
    MPV_EVENT_PROPERTY_CHANGE, MPV_EVENT_SHUTDOWN, MPV_FORMAT_NODE, MPV_FORMAT_NONE, mpv_event,
    mpv_event_end_file, mpv_event_log_message, mpv_event_property, mpv_handle, mpv_node,
};
use crate::value::Value;

const EVENT_WAIT_TIMEOUT_SECS: f64 = 0.1;

fn claim_shutdown(alive: &AtomicBool) -> bool {
    alive.swap(false, Ordering::AcqRel)
}

/// Properties the UI subscribes to.
pub const OBSERVED: &[&str] = &[
    "time-pos",
    "duration",
    "pause",
    "volume",
    "mute",
    "speed",
    "track-list",
    "aid",
    "sid",
    "chapter-list",
    "media-title",
    "path",
    "idle-active",
    "eof-reached",
    "hwdec-current",
    "video-params",
    "sub-delay",
    "audio-delay",
    "sub-visibility",
    "paused-for-cache",
    "cache-buffering-state",
    "demuxer-cache-time",
    "ab-loop-a",
    "ab-loop-b",
    "seeking",
    "secondary-sid",
    "deinterlace",
    "video-rotate",
    "audio-device",
    "audio-device-list",
    "chapter",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSelection {
    Auto,
    Id(i64),
    None,
}

impl TrackSelection {
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Int(id) if *id > 0 => Self::Id(*id),
            Value::Float(id) if id.is_finite() && *id > 0.0 => Self::Id(*id as i64),
            Value::Str(value) if value.eq_ignore_ascii_case("auto") => Self::Auto,
            Value::Str(value) if value.eq_ignore_ascii_case("no") => Self::None,
            _ => Self::None,
        }
    }

    pub fn as_value(self) -> Value {
        match self {
            Self::Auto => Value::Str("auto".into()),
            Self::Id(id) if id > 0 => Value::Int(id),
            Self::Id(_) | Self::None => Value::Str("no".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrackState {
    pub primary: TrackSelection,
    pub primary_volume: f64,
    pub primary_delay: f64,
}

impl Default for AudioTrackState {
    fn default() -> Self {
        Self {
            primary: TrackSelection::Auto,
            primary_volume: 100.0,
            primary_delay: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTrackState {
    pub audio: TrackSelection,
    pub subtitle: TrackSelection,
    pub secondary_subtitle: TrackSelection,
}

impl Default for FileTrackState {
    fn default() -> Self {
        Self {
            audio: TrackSelection::Auto,
            subtitle: TrackSelection::Auto,
            secondary_subtitle: TrackSelection::None,
        }
    }
}

impl FileTrackState {
    pub fn from_engine(engine: &Engine) -> Self {
        Self {
            audio: engine
                .get("aid")
                .map(|value| TrackSelection::from_value(&value))
                .unwrap_or(TrackSelection::Auto),
            subtitle: engine
                .get("sid")
                .map(|value| TrackSelection::from_value(&value))
                .unwrap_or(TrackSelection::Auto),
            secondary_subtitle: engine
                .get("secondary-sid")
                .map(|value| TrackSelection::from_value(&value))
                .unwrap_or(TrackSelection::None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTrackInfo {
    pub id: i64,
    pub title: String,
    pub language: String,
    pub selected: bool,
    pub external: bool,
}

/// Messages leaving mpv for the UI.
#[derive(Debug, Clone)]
pub enum EngineMsg {
    /// `observe_property` notification; `None` means the property is currently unavailable.
    Prop(String, Value),
    /// Lifecycle events the playlist logic reacts to.
    FileLoaded,
    EndFile {
        reason: i32,
        error: i32,
        entry_id: i64,
    },
}

/// `mpv` raw handle wrapper. The client API is thread-safe, only a handful of functions
/// (documented per symbol) must stay on one thread; we never call those from two places.
pub struct Engine {
    mpv: mpv_handle,
    api: Api,
    rx: Receiver<EngineMsg>,
    log_lines: Arc<Mutex<VecDeque<String>>>,
    alive: Arc<AtomicBool>,
    _event_thread: Option<std::thread::JoinHandle<()>>,
}

/// Newtype that lets the raw `mpv_handle` cross into the event thread.
///
/// SAFETY: mpv's client API documents the handle as usable from any thread for the
/// functions we call there (`mpv_wait_event`), so this is sound.
struct SendHandle(mpv_handle);
unsafe impl Send for SendHandle {}

// SAFETY: mpv_handle is documented as usable from any thread; the event thread only calls
// mpv_wait_event, everything else happens on the UI thread or through mpv's own queues.
unsafe impl Send for Engine {}

impl Engine {
    pub fn new(
        settings: &crate::settings::Settings,
        silent: bool,
        verbose: bool,
    ) -> Result<Self, String> {
        let (_lib, api) = ffi::load()?;

        let mpv = unsafe { (api.create)() };
        if mpv.is_null() {
            return Err("mpv_create returned NULL".into());
        }

        let set_option = |name: &str, value: &str| -> Result<bool, String> {
            let c_name = CString::new(name)
                .map_err(|_| format!("mpv option name contains NUL: {name:?}"))?;
            let c_value =
                CString::new(value).map_err(|_| format!("mpv option {name:?} contains NUL"))?;
            Ok(unsafe { (api.set_option_string)(mpv, c_name.as_ptr(), c_value.as_ptr()) >= 0 })
        };

        let mut options: Vec<(&str, String)> = vec![
            ("vo", "libmpv".into()),
            ("idle", "yes".into()),
            ("keep-open", "yes".into()),
            ("osc", "no".into()),
            ("osd-level", "0".into()),
            ("input-default-bindings", "no".into()),
            ("input-vo-keyboard", "no".into()),
            ("hwdec", Self::hwdec_value(settings.hwdec).into()),
            ("hr-seek", "yes".into()),
            (
                "volume-max",
                settings.volume_max.clamp(100, 200).to_string(),
            ),
            (
                "volume",
                settings
                    .volume
                    .clamp(0, settings.volume_max.clamp(100, 200))
                    .to_string(),
            ),
            ("mute", yes_no(settings.mute)),
            ("audio-client-name", "MacPlayer".into()),
            ("screenshot-format", "png".into()),
            ("sub-auto", settings.sub_auto.clone()),
            (
                "sub-file-paths",
                "sub:subs:Sub:Subs:subtitles:Subtitles".into(),
            ),
            ("sub-codepage", settings.sub_codepage.clone()),
            ("sub-font-size", settings.sub_size.to_string()),
            ("sub-bold", yes_no(settings.sub_bold)),
            ("sub-pos", settings.sub_pos.to_string()),
            ("sub-visibility", yes_no(settings.sub_visible)),
            ("sub-fix-timing", "yes".into()),
            ("slang", settings.preferred_sub_lang.clone()),
            ("alang", settings.preferred_audio_lang.clone()),
            ("audio-display", "embedded-first".into()),
            (
                "reset-on-next-file",
                "sub-delay,audio-delay,video-zoom,video-pan-x,video-pan-y,\
                 video-rotate,video-aspect-override,ab-loop-a,ab-loop-b"
                    .into(),
            ),
            ("ytdl", "yes".into()),
            (
                "ytdl-format",
                Self::ytdl_format(&settings.ytdl_quality).into(),
            ),
            (
                "audio-pitch-correction",
                yes_no(settings.audio_pitch_correction),
            ),
            ("audio-channels", settings.audio_channels.clone()),
            (
                "video-sync",
                Self::video_sync_value(&settings.video_sync).into(),
            ),
            ("scale", settings.scale_profile.clone()),
        ];
        if settings.interpolation {
            options.extend([
                ("interpolation", "yes".into()),
                ("tscale", "oversample".into()),
            ]);
        }
        if !settings.audio_spdif.is_empty() {
            options.push(("audio-spdif", settings.audio_spdif.clone()));
        }
        if settings.audio_exclusive {
            options.push(("audio-exclusive-mode", "yes".into()));
        }
        if settings.cache_secs > 0 {
            options.extend([
                ("cache", "yes".into()),
                ("cache-secs", settings.cache_secs.to_string()),
            ]);
        }
        if settings.demuxer_max_mb > 0 {
            options.push((
                "demuxer-max-bytes",
                format!("{}MiB", settings.demuxer_max_mb),
            ));
        }
        if !settings.http_user_agent.is_empty() {
            options.push(("user-agent", settings.http_user_agent.clone()));
        }
        let headers = http_header_fields(&settings.http_headers);
        if !headers.is_empty() {
            options.push(("http-header-fields", headers));
        }
        if !settings.sub_font.is_empty() {
            options.push(("sub-font", settings.sub_font.clone()));
        }
        if settings.hdr_tone_mapping {
            options.extend([
                ("target-trc", "srgb".into()),
                ("target-prim", "bt.709".into()),
                ("target-peak", "203".into()),
                ("tone-mapping", settings.hdr_tone_mapping_mode.clone()),
                ("hdr-compute-peak", "yes".into()),
            ]);
        }
        if silent {
            options.push(("ao", "null".into()));
        }

        // Builds without Lua or yt-dlp support reject "osc"/"ytdl"; retry without them.
        let apply_options = |skip_optional: bool| -> Result<bool, String> {
            let mut all_ok = true;
            for (name, value) in &options {
                if skip_optional && (*name == "osc" || *name == "ytdl") {
                    continue;
                }
                if !set_option(name, value)? {
                    all_ok = false;
                }
            }
            Ok(all_ok)
        };
        let first_pass = match apply_options(false) {
            Ok(ok) => ok,
            Err(error) => {
                unsafe { (api.terminate_destroy)(mpv) };
                return Err(error);
            }
        };
        if !first_pass {
            let second_pass = match apply_options(true) {
                Ok(ok) => ok,
                Err(error) => {
                    unsafe { (api.terminate_destroy)(mpv) };
                    return Err(error);
                }
            };
            if !second_pass {
                unsafe { (api.terminate_destroy)(mpv) };
                return Err("mpv rejected one or more required startup options".into());
            }
        }

        let log_level = std::env::var("MACPLAYER_LOGLEVEL")
            .unwrap_or_else(|_| if verbose { "v".into() } else { "warn".into() });
        let c_level = match CString::new(log_level.as_str()) {
            Ok(level) => level,
            Err(_) => {
                unsafe { (api.terminate_destroy)(mpv) };
                return Err("MACPLAYER_LOGLEVEL contains NUL".into());
            }
        };
        unsafe { (api.request_log_messages)(mpv, c_level.as_ptr()) };

        let rc = unsafe { (api.initialize)(mpv) };
        if rc < 0 {
            let msg = unsafe { CStr::from_ptr((api.error_string)(rc)) }
                .to_string_lossy()
                .into_owned();
            unsafe { (api.terminate_destroy)(mpv) };
            return Err(format!("mpv_initialize failed: {msg}"));
        }

        let log_lines: Arc<Mutex<VecDeque<String>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(200)));
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = channel::<EngineMsg>();

        for (index, name) in OBSERVED.iter().enumerate() {
            let Ok(c_name) = CString::new(*name) else {
                continue;
            };
            unsafe {
                (api.observe_property)(mpv, (index + 1) as u64, c_name.as_ptr(), MPV_FORMAT_NODE)
            };
        }

        let thread_log = Arc::clone(&log_lines);
        let thread_alive = Arc::clone(&alive);
        let event_handle = SendHandle(mpv);
        let event_verbose = verbose;
        let event_thread =
            match std::thread::Builder::new()
                .name("mpv-events".into())
                .spawn(move || {
                    event_loop(
                        event_handle,
                        api,
                        tx,
                        thread_log,
                        thread_alive,
                        event_verbose,
                    )
                }) {
                Ok(handle) => handle,
                Err(error) => {
                    unsafe { (api.terminate_destroy)(mpv) };
                    return Err(format!("spawn event thread: {error}"));
                }
            };

        Ok(Self {
            mpv,
            api,
            rx,
            log_lines,
            alive,
            _event_thread: Some(event_thread),
        })
    }

    /// On macOS the plain `videotoolbox` entry resolves to a direct
    /// VideoToolbox→GL interop, which skips the per-frame GPU→RAM→GPU
    /// round-trip of the `-copy` variants — a large CPU and battery saving at
    /// 4K. The `-copy` mode and `auto-safe` stay in the chain as per-file
    /// fallbacks for formats whose direct interop is unsupported; mpv then
    /// falls back on its own, worst case to software decoding.
    #[cfg(target_os = "macos")]
    pub fn hwdec_value(enabled: bool) -> &'static str {
        if !enabled {
            "no"
        } else {
            "videotoolbox,videotoolbox-copy,auto-safe"
        }
    }

    /// Linux does not expose VideoToolbox; keep the platform-specific decoder
    /// out of the fallback list so mpv accepts the option cleanly.
    #[cfg(not(target_os = "macos"))]
    pub fn hwdec_value(enabled: bool) -> &'static str {
        if !enabled {
            "no"
        } else {
            "vaapi-copy,nvdec-copy,auto-copy-safe"
        }
    }

    /// Map the settings vocabulary onto values mpv's `video-sync` option accepts.
    /// mpv has no "auto": its default clock is `audio`; the resync/desync ids map
    /// onto their real option names.
    pub fn video_sync_value(mode: &str) -> &'static str {
        match mode {
            "display-resync" => "display-resample",
            "display-desync" => "display-desync",
            _ => "audio",
        }
    }

    /// The raw mpv handle, for constructing the render context.
    pub fn handle(&self) -> mpv_handle {
        self.mpv
    }

    /// The resolved libmpv entry points, shared with the renderer.
    pub fn api(&self) -> Api {
        self.api
    }

    /// Drain pending messages; call once per UI frame.
    pub fn poll(&self) -> Vec<EngineMsg> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.push(msg);
        }
        out
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if !self.alive.load(Ordering::Relaxed) {
            return None;
        }
        let c_name = CString::new(name).ok()?;
        let mut node = mpv_node::none();
        let rc = unsafe {
            (self.api.get_property)(
                self.mpv,
                c_name.as_ptr(),
                MPV_FORMAT_NODE,
                &mut node as *mut mpv_node as *mut c_void,
            )
        };
        if rc < 0 {
            return None;
        }
        let value = unsafe { Value::from_mpv_node(&node) };
        unsafe { (self.api.free_node_contents)(&mut node) };
        Some(value)
    }

    pub fn get_f64(&self, name: &str) -> f64 {
        self.get(name).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }

    pub fn audio_tracks(&self) -> Vec<AudioTrackInfo> {
        self.get("track-list")
            .map(|value| {
                value
                    .as_array()
                    .iter()
                    .filter(|track| track.get("type").as_str() == Some("audio"))
                    .filter_map(|track| {
                        let id = track.get("id").as_i64().filter(|id| *id > 0)?;
                        Some(AudioTrackInfo {
                            id,
                            title: track.get("title").as_str().unwrap_or_default().to_string(),
                            language: track.get("lang").as_str().unwrap_or_default().to_string(),
                            selected: track.get("selected").as_bool(),
                            external: track.get("external").as_bool(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn file_track_state(&self) -> FileTrackState {
        FileTrackState::from_engine(self)
    }

    pub fn audio_track_state(&self) -> AudioTrackState {
        AudioTrackState {
            primary: self
                .get("aid")
                .map(|value| TrackSelection::from_value(&value))
                .unwrap_or(TrackSelection::Auto),
            primary_volume: self.get_f64("volume").clamp(0.0, 200.0),
            primary_delay: self.get_f64("audio-delay"),
        }
    }

    /// Apply stream-related options as properties immediately before a URL load.
    /// This avoids embedding user-controlled values in mpv's comma-separated
    /// `loadfile` option list.
    pub fn set_stream_options(&self, user_agent: &str, headers: &str) -> bool {
        let mut ok = self.set("user-agent", &Value::Str(user_agent.to_string()));
        let header_value = http_header_fields(headers);
        ok &= self.set("http-header-fields", &Value::Str(header_value));
        ok
    }

    pub fn set_audio_track(&self, selection: TrackSelection) -> bool {
        self.set("aid", &selection.as_value())
    }

    pub fn set_subtitle_track(&self, selection: TrackSelection) -> bool {
        self.set("sid", &selection.as_value())
    }

    pub fn set_secondary_subtitle(&self, selection: TrackSelection) -> bool {
        self.set("secondary-sid", &selection.as_value())
    }

    pub fn apply_file_track_state(&self, state: &FileTrackState) -> bool {
        self.set_audio_track(state.audio)
            & self.set_subtitle_track(state.subtitle)
            & self.set_secondary_subtitle(state.secondary_subtitle)
    }

    pub fn set(&self, name: &str, value: &Value) -> bool {
        if !self.alive.load(Ordering::Relaxed) {
            return false;
        }
        let c_name = match CString::new(name) {
            Ok(v) => v,
            Err(_) => return false,
        };
        // Simple scalars go through the typed setters; anything else as a string.
        let rc = unsafe {
            match value {
                Value::Bool(flag) => {
                    let mut int: i32 = if *flag { 1 } else { 0 };
                    (self.api.set_property)(
                        self.mpv,
                        c_name.as_ptr(),
                        ffi::MPV_FORMAT_FLAG,
                        &mut int as *mut i32 as *mut c_void,
                    )
                }
                Value::Int(number) => {
                    let mut number = *number;
                    (self.api.set_property)(
                        self.mpv,
                        c_name.as_ptr(),
                        ffi::MPV_FORMAT_INT64,
                        &mut number as *mut i64 as *mut c_void,
                    )
                }
                Value::Float(number) => {
                    let mut number = *number;
                    (self.api.set_property)(
                        self.mpv,
                        c_name.as_ptr(),
                        ffi::MPV_FORMAT_DOUBLE,
                        &mut number as *mut f64 as *mut c_void,
                    )
                }
                other => {
                    let text = value_to_string(other);
                    let c_value = match CString::new(text) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    return (self.api.set_property_string)(
                        self.mpv,
                        c_name.as_ptr(),
                        c_value.as_ptr(),
                    ) >= 0;
                }
            }
        };
        if rc < 0 {
            let msg = unsafe { CStr::from_ptr((self.api.error_string)(rc)) }
                .to_string_lossy()
                .into_owned();
            self.log(format!("[set] {name}={value:?}: {msg}"));
            return false;
        }
        true
    }

    /// Queue a command asynchronously; never blocks the UI thread.
    pub fn cmd(&self, args: &[&str]) -> bool {
        self.run_command(args, true)
    }

    /// Run a command synchronously and return its result, if any.
    pub fn cmd_sync(&self, args: &[&str]) -> Option<Value> {
        if !self.alive.load(Ordering::Relaxed) {
            return None;
        }
        let c_strings: Vec<CString> = args.iter().filter_map(|a| CString::new(*a).ok()).collect();
        if c_strings.len() != args.len() {
            return None;
        }
        let mut pointers: Vec<*const c_char> = c_strings.iter().map(|s| s.as_ptr()).collect();
        pointers.push(std::ptr::null());

        let mut node = mpv_node::none();
        let rc = unsafe {
            (self.api.command_ret)(
                self.mpv,
                pointers.as_ptr(),
                &mut node as *mut mpv_node as *mut c_void,
            )
        };
        if rc < 0 {
            return None;
        }
        let value = unsafe { Value::from_mpv_node(&node) };
        unsafe { (self.api.free_node_contents)(&mut node) };
        Some(value)
    }

    fn run_command(&self, args: &[&str], async_: bool) -> bool {
        if !self.alive.load(Ordering::Relaxed) {
            return false;
        }
        let c_strings: Vec<CString> = match args.iter().map(|a| CString::new(*a)).collect() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let mut pointers: Vec<*const c_char> = c_strings.iter().map(|s| s.as_ptr()).collect();
        pointers.push(std::ptr::null());

        let rc = unsafe {
            if async_ {
                (self.api.command_async)(self.mpv, 0, pointers.as_ptr())
            } else {
                (self.api.command)(self.mpv, pointers.as_ptr())
            }
        };
        if rc < 0 {
            let joined = args.join(" ");
            let msg = unsafe { CStr::from_ptr((self.api.error_string)(rc)) }
                .to_string_lossy()
                .into_owned();
            self.log(format!("[cmd] {joined}: {msg}"));
            return false;
        }
        true
    }

    /// Replace the current file with extra per-load mpv options (`k=v,k=v`).
    /// `options` may be empty; it must not contain commas inside values.
    pub fn load_with_options(&self, path: &str, start: Option<f64>, options: &str) -> bool {
        let mut all = String::new();
        if let Some(seconds) = start {
            all.push_str(&format!("start={seconds:.2}"));
        }
        if !options.is_empty() {
            if !all.is_empty() {
                all.push(',');
            }
            all.push_str(options);
        }
        match all.is_empty() {
            true => self.cmd(&["loadfile", path, "replace"]),
            false => self.cmd(&["loadfile", path, "replace", "-1", &all]),
        }
    }

    /// yt-dlp format selector for a quality id ("best", "1080", "audio", ...).
    pub fn ytdl_format(quality: &str) -> &'static str {
        match quality {
            "2160" => "bestvideo[height<=?2160]+bestaudio/best[height<=?2160]",
            "1440" => "bestvideo[height<=?1440]+bestaudio/best[height<=?1440]",
            "1080" => "bestvideo[height<=?1080]+bestaudio/best[height<=?1080]",
            "720" => "bestvideo[height<=?720]+bestaudio/best[height<=?720]",
            "480" => "bestvideo[height<=?480]+bestaudio/best[height<=?480]",
            "360" => "bestvideo[height<=?360]+bestaudio/best[height<=?360]",
            "audio" => "bestaudio/best",
            _ => "bestvideo+bestaudio/best",
        }
    }

    pub fn add(&self, name: &str, delta: f64, lo: Option<f64>, hi: Option<f64>) -> f64 {
        let mut value = self.get_f64(name) + delta;
        if let Some(lo) = lo {
            value = value.max(lo);
        }
        if let Some(hi) = hi {
            value = value.min(hi);
        }
        self.set(name, &Value::Float(value));
        value
    }

    fn log(&self, line: String) {
        if let Ok(mut lines) = self.log_lines.lock() {
            if lines.len() >= 200 {
                lines.pop_front();
            }
            lines.push_back(line.clone());
        }
        // Mirror failures to the diagnostic log file (best-effort).
        crate::logging::write(&line);
    }

    pub fn log_lines(&self) -> Vec<String> {
        self.log_lines
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn shutdown(&mut self) {
        // `Player::on_exit` shuts the engine down explicitly, and `Engine::drop` calls this
        // method as a final guard.  Do not destroy the mpv handle twice.
        if !claim_shutdown(&self.alive) {
            return;
        }
        if let Some(handle) = self._event_thread.take() {
            let _ = handle.join();
        }
        unsafe { (self.api.terminate_destroy)(self.mpv) };
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn yes_no(value: bool) -> String {
    if value { "yes".into() } else { "no".into() }
}

/// `Key: Value` lines joined into the comma-separated form mpv's
/// `http-header-fields` option expects. Commas inside a header value are escaped.
fn http_header_fields(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && line.contains(':'))
        .map(escape_option_list_field)
        .collect::<Vec<_>>()
        .join(",")
}

fn escape_option_list_field(value: &str) -> String {
    value.replace('\\', "\\\\").replace(',', "\\,")
}

/// Render scalar values in the format expected by mpv option and property strings.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::Bool(v) => yes_no(*v),
        Value::Int(v) => v.to_string(),
        Value::Float(v) => format!("{v}"),
        Value::Str(v) => v.clone(),
        _ => String::new(),
    }
}

/// mpv's event thread: blocks in `mpv_wait_event` and forwards what the UI cares about.
fn event_loop(
    handle: SendHandle,
    api: Api,
    tx: Sender<EngineMsg>,
    log_lines: Arc<Mutex<VecDeque<String>>>,
    alive: Arc<AtomicBool>,
    verbose: bool,
) {
    let mpv = handle.0;
    while alive.load(Ordering::Relaxed) {
        let event: *mut mpv_event = unsafe { (api.wait_event)(mpv, EVENT_WAIT_TIMEOUT_SECS) };
        if event.is_null() {
            continue;
        }
        let event = unsafe { &*event };
        match event.event_id {
            MPV_EVENT_NONE => {}
            MPV_EVENT_SHUTDOWN => {
                alive.store(false, Ordering::Release);
                break;
            }
            MPV_EVENT_FILE_LOADED => {
                let _ = tx.send(EngineMsg::FileLoaded);
            }
            MPV_EVENT_END_FILE if !event.data.is_null() => {
                let end = unsafe { &*(event.data as *const mpv_event_end_file) };
                let _ = tx.send(EngineMsg::EndFile {
                    reason: end.reason,
                    error: end.error,
                    entry_id: end.playlist_entry_id,
                });
            }
            MPV_EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                let prop = unsafe { &*(event.data as *const mpv_event_property) };
                let name = unsafe { cstr(prop.name) };
                if prop.format != MPV_FORMAT_NONE && !prop.data.is_null() {
                    let node = unsafe { &*(prop.data as *const mpv_node) };
                    let value = unsafe { Value::from_mpv_node(node) };
                    let _ = tx.send(EngineMsg::Prop(name, value));
                } else {
                    let _ = tx.send(EngineMsg::Prop(name, Value::None));
                }
            }
            MPV_EVENT_LOG_MESSAGE if !event.data.is_null() => {
                let msg = unsafe { &*(event.data as *const mpv_event_log_message) };
                let level = unsafe { cstr(msg.level) };
                let prefix = unsafe { cstr(msg.prefix) };
                let text = unsafe { cstr(msg.text) };
                let line = format!("[{level}] {prefix}: {text}");
                if let Ok(mut lines) = log_lines.lock() {
                    if lines.len() >= 200 {
                        lines.pop_front();
                    }
                    lines.push_back(line.clone());
                }
                if verbose {
                    eprintln!("{line}");
                }
                crate::logging::write(&line);
            }
            _ => {}
        }
    }
}

/// # Safety
/// `ptr` must be NULL or a valid NUL-terminated string.
unsafe fn cstr(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ytdl_quality_ids_map_to_format_selectors() {
        assert_eq!(Engine::ytdl_format("best"), "bestvideo+bestaudio/best");
        assert!(Engine::ytdl_format("720").contains("height<=?720"));
        assert_eq!(Engine::ytdl_format("audio"), "bestaudio/best");
        // Unknown ids degrade to "best", never to an invalid selector.
        assert_eq!(Engine::ytdl_format("bogus"), Engine::ytdl_format("best"));
    }

    #[test]
    fn video_sync_ids_map_onto_real_mpv_values() {
        // mpv has no "auto"; both auto and audio select the audio clock.
        assert_eq!(Engine::video_sync_value("auto"), "audio");
        assert_eq!(Engine::video_sync_value("audio"), "audio");
        assert_eq!(
            Engine::video_sync_value("display-resync"),
            "display-resample"
        );
        assert_eq!(Engine::video_sync_value("display-desync"), "display-desync");
        // Unknown ids degrade to the default clock, never an invalid option.
        assert_eq!(Engine::video_sync_value("bogus"), "audio");
    }

    #[test]
    fn http_headers_accept_lines_and_commas() {
        assert_eq!(
            http_header_fields("Referer: https://x\nCookie: a=b"),
            "Referer: https://x,Cookie: a=b"
        );
        assert_eq!(http_header_fields("Cookie: a,b"), "Cookie: a\\,b");
        assert_eq!(http_header_fields("no-colon"), "");
    }

    #[test]
    fn shutdown_state_can_only_be_claimed_once() {
        let alive = AtomicBool::new(true);
        assert!(claim_shutdown(&alive));
        assert!(!claim_shutdown(&alive));
    }

    #[test]
    fn track_selection_values_are_safe() {
        assert_eq!(
            TrackSelection::from_value(&Value::Str("auto".into())),
            TrackSelection::Auto
        );
        assert_eq!(
            TrackSelection::from_value(&Value::Int(2)),
            TrackSelection::Id(2)
        );
        assert_eq!(TrackSelection::Id(0).as_value(), Value::Str("no".into()));
    }

    #[test]
    fn audio_track_info_filters_non_audio_tracks() {
        let tracks = Value::Array(vec![
            Value::Map(vec![
                ("id".into(), Value::Int(2)),
                ("type".into(), Value::Str("audio".into())),
                ("title".into(), Value::Str("English".into())),
                ("lang".into(), Value::Str("en".into())),
                ("selected".into(), Value::Bool(true)),
                ("external".into(), Value::Bool(false)),
            ]),
            Value::Map(vec![
                ("id".into(), Value::Int(3)),
                ("type".into(), Value::Str("sub".into())),
            ]),
        ]);
        let audio: Vec<AudioTrackInfo> = tracks
            .as_array()
            .iter()
            .filter(|track| track.get("type").as_str() == Some("audio"))
            .filter_map(|track| {
                Some(AudioTrackInfo {
                    id: track.get("id").as_i64()?,
                    title: track.get("title").as_str()?.into(),
                    language: track.get("lang").as_str()?.into(),
                    selected: track.get("selected").as_bool(),
                    external: track.get("external").as_bool(),
                })
            })
            .collect();
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].id, 2);
        assert!(audio[0].selected);
    }
}
