//! The player window: chrome layout, mpv property handling, keymap and every user action.
//!
//! The MacPlayer application state, restructured around egui's immediate-mode frame.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, Context, Id, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::dialogs::{
    ControlPanel, GoToDialog, GoToMode, PreferencesDialog, TextDialog, UrlDialog,
};
use crate::engine::{Engine, EngineMsg, FileTrackState, TrackSelection};
use crate::ffi;
use crate::menus::{self, Action, MenuCtx, MenuState, TrackKind, TrackSel};
use crate::playlist::{Playlist, PlaylistOut};
use crate::render::{RectPx, Renderer};
use crate::settings::Settings;
use crate::skin::{self, *};
use crate::util::{
    expand_paths, ext, fmt_time, human_size, is_sub, is_url, read_playlist_file, similar_files,
    title_of,
};
use crate::value::Value;
use crate::widgets::{
    self, Badge, Click, IconBtn, Osd, SeekUi, VolumeUi, apply_cursor, fmt_pair, info_badges,
    seek_bar, time_label, title_bar, volume_bar,
};

const PLAYLIST_EXT: &[&str] = crate::util::PLAYLIST_EXT;
const FS_EDGE_TIMEOUT: f64 = 3.0;

/// A single thing the UI wants the player to do. Chrome, menus and dialogs all push into
/// one queue; it is drained once per frame, after painting.
#[derive(Debug, Clone, Default)]
struct Pending {
    actions: Vec<Action>,
    /// (paths, append)
    open: Vec<(Vec<String>, bool)>,
    quit: bool,
}

impl Pending {
    fn new() -> Self {
        Self {
            actions: Vec::new(),
            open: Vec::new(),
            quit: false,
        }
    }

    fn push(&mut self, action: Action) {
        self.actions.push(action);
    }

    fn open_paths(&mut self, paths: Vec<String>, append: bool) {
        self.open.push((paths, append));
    }
}

pub struct Player {
    engine: Option<Engine>,
    renderer: Option<Arc<Mutex<Renderer>>>,
    startup_error: Option<String>,
    ipc_rx: Receiver<Vec<String>>,

    settings: Settings,
    state: HashMap<String, Value>,

    playlist: Playlist,
    current_path: Option<String>,
    file_ready: bool,
    last_pos: f64,
    seek_target: (f64, f64),
    fail_count: usize,
    autofit_pending: bool,
    resumed_at: Option<f64>,
    pending_subs: Vec<String>,

    osd: Osd,
    seek: SeekUi,
    volume: VolumeUi,
    badges: Vec<Badge>,

    title_text: String,
    aspect_name: String,
    display_mode: String,
    flips: (bool, bool),
    normalize: bool,
    eq_active: bool,
    info_visible: bool,
    show_remaining: bool,

    menu: MenuState,
    track_menu_kind: Option<TrackKind>,
    goto_dialog: GoToDialog,
    url_dialog: UrlDialog,
    text_dialog: TextDialog,
    prefs: PreferencesDialog,
    panel: ControlPanel,

    pending: Pending,

    // -- window state -----------------------------------------------------------------------------
    playlist_visible: bool,
    fullscreen: bool,
    maximized: bool,
    fs_top: bool,
    fs_bottom: bool,
    fs_right: bool,
    fs_pin: bool,
    was_maximized: bool,
    on_top: bool,
    state_changed: f64,
    press_origin: Option<Pos2>,

    // -- cursor ------------------------------------------------------------------------------------
    cursor_hidden: bool,
    mouse_in_video: bool,
    last_pointer_move: Option<Instant>,

    // -- drag and drop -------------------------------------------------------------------------------
    hovered_files: Vec<String>,

    gl_ready: bool,
    pending_open: Vec<(Vec<String>, bool)>,
    closing: bool,
    next_save: f64,
    next_autofit: f64,
    next_cursor: f64,
    fs_top_deadline: f64,
    fs_bottom_deadline: f64,
    next_info: f64,
    now: f64,
    started_at: Instant,
}

impl Player {
    pub fn new(
        ctx: Context,
        args: Vec<String>,
        ipc_rx: Receiver<Vec<String>>,
        settings: Settings,
        verbose: bool,
        fullscreen: bool,
    ) -> Self {
        let playlist_visible = settings.playlist_visible;
        let show_remaining = settings.show_remaining;
        let on_top = settings.on_top == "always";
        let (engine, renderer, startup_error) = match Engine::new(&settings, false, verbose) {
            Ok(engine) => {
                let renderer = Renderer::new(engine.api(), engine.handle(), ctx);
                (Some(engine), Some(Arc::new(Mutex::new(renderer))), None)
            }
            Err(error) => (None, None, Some(error)),
        };
        let mut player = Self {
            engine,
            renderer,
            startup_error,
            ipc_rx,
            settings,
            state: HashMap::new(),
            playlist: Playlist::default(),
            current_path: None,
            file_ready: false,
            last_pos: 0.0,
            seek_target: (0.0, 0.0),
            fail_count: 0,
            autofit_pending: false,
            resumed_at: None,
            pending_subs: Vec::new(),
            osd: Osd::default(),
            seek: SeekUi::default(),
            volume: VolumeUi::default(),
            badges: Vec::new(),
            title_text: String::new(),
            aspect_name: "Default".into(),
            display_mode: "fit".into(),
            flips: (false, false),
            normalize: false,
            eq_active: false,
            info_visible: false,
            show_remaining,
            menu: MenuState::default(),
            track_menu_kind: None,
            goto_dialog: GoToDialog::default(),
            url_dialog: UrlDialog::default(),
            text_dialog: TextDialog::default(),
            prefs: PreferencesDialog::default(),
            panel: ControlPanel::default(),
            pending: Pending::new(),
            playlist_visible,
            fullscreen,
            maximized: false,
            fs_top: false,
            fs_bottom: false,
            fs_right: false,
            fs_pin: false,
            was_maximized: false,
            on_top,
            state_changed: 0.0,
            press_origin: None,
            cursor_hidden: false,
            mouse_in_video: false,
            last_pointer_move: None,
            hovered_files: Vec::new(),
            gl_ready: false,
            pending_open: Vec::new(),
            closing: false,
            next_save: 0.0,
            next_autofit: 0.0,
            next_cursor: 0.0,
            fs_top_deadline: 0.0,
            fs_bottom_deadline: 0.0,
            next_info: 0.0,
            now: 0.0,
            started_at: Instant::now(),
        };
        if !args.is_empty() {
            player.pending.open_paths(args, false);
        }
        player
            .playlist
            .set_modes(&player.settings.repeat_mode, player.settings.shuffle);
        player.apply_eq();
        player
    }

    // -- state helpers ------------------------------------------------------------------------------

    fn tracks(&self) -> &[Value] {
        self.state
            .get("track-list")
            .map(|v| v.as_array())
            .unwrap_or(&[])
    }

    fn has_file(&self) -> bool {
        !self
            .state
            .get("idle-active")
            .map(|v| v.as_bool())
            .unwrap_or(true)
    }

    fn is_playing(&self) -> bool {
        self.has_file()
            && !self
                .state
                .get("pause")
                .map(|v| v.as_bool())
                .unwrap_or(false)
            && !self
                .state
                .get("eof-reached")
                .map(|v| v.as_bool())
                .unwrap_or(false)
    }

    fn duration(&self) -> f64 {
        self.state
            .get("duration")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    fn position(&self) -> f64 {
        if self.file_ready { self.last_pos } else { 0.0 }
    }

    fn has_video(&self) -> bool {
        self.tracks()
            .iter()
            .any(|t| t.get("type").as_str() == Some("video") && t.get("selected").as_bool())
    }

    fn engine(&self) -> Option<&Engine> {
        self.engine.as_ref()
    }

    fn set_prop(&self, name: &str, value: impl Into<Value>) {
        if let Some(engine) = self.engine() {
            engine.set(name, &value.into());
        }
    }

    fn track_code(selection: TrackSelection) -> i64 {
        match selection {
            TrackSelection::Auto => -2,
            TrackSelection::None => -1,
            TrackSelection::Id(id) => id.max(1),
        }
    }

    fn remember_track_preference(&mut self, kind: TrackKind, selection: TrackSelection) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let existing = self.settings.track_preference(&path).cloned();
        let mut audio = existing.as_ref().map(|p| p.audio).unwrap_or(-2);
        let mut subtitle = existing.as_ref().map(|p| p.subtitle).unwrap_or(-2);
        let secondary = existing
            .as_ref()
            .map(|p| p.secondary_subtitle)
            .unwrap_or(-1);
        match kind {
            TrackKind::Audio => audio = Self::track_code(selection),
            TrackKind::Sub => subtitle = Self::track_code(selection),
            TrackKind::Video => return,
        }
        self.settings
            .set_track_preference(&path, audio, subtitle, secondary);
        self.settings.save();
    }

    fn remember_secondary_subtitle(&mut self, selection: TrackSelection) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let existing = self.settings.track_preference(&path).cloned();
        self.settings.set_track_preference(
            &path,
            existing.as_ref().map(|p| p.audio).unwrap_or(-2),
            existing.as_ref().map(|p| p.subtitle).unwrap_or(-2),
            Self::track_code(selection),
        );
        self.settings.save();
    }

    fn apply_saved_track_preference(&mut self) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let Some(preference) = self.settings.track_preference(&path).cloned() else {
            return;
        };
        let selection = |code: i64| match code {
            -2 => TrackSelection::Auto,
            -1 => TrackSelection::None,
            id if id > 0 => TrackSelection::Id(id),
            _ => TrackSelection::Auto,
        };
        if let Some(engine) = self.engine() {
            engine.apply_file_track_state(&FileTrackState {
                audio: selection(preference.audio),
                subtitle: selection(preference.subtitle),
                secondary_subtitle: selection(preference.secondary_subtitle),
            });
        }
    }

    fn cmd(&self, args: &[&str]) {
        if let Some(engine) = self.engine() {
            engine.cmd(args);
        }
    }

    fn get_f64(&self, name: &str) -> f64 {
        self.engine().map(|e| e.get_f64(name)).unwrap_or(0.0)
    }

    fn get_bool(&self, name: &str) -> bool {
        self.engine()
            .and_then(|e| e.get(name))
            .map(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn osd(&mut self, text: impl Into<String>) {
        self.osd.show_message(text, 1800, TEXT_E4);
    }

    fn osd_ms(&mut self, text: impl Into<String>, ms: u64) {
        self.osd.show_message(text, ms, TEXT_E4);
    }

    fn osd_error(&mut self, text: impl Into<String>, ms: u64) {
        self.osd.show_message(text, ms, DANGER);
    }

    // -- layout --------------------------------------------------------------------------------------

    /// Window chrome beyond the video: borders, title bar, seek row, control bar and playlist.
    fn chrome_extra(&self) -> Vec2 {
        let width = 2.0 * FRAME
            + if self.playlist_visible {
                self.settings.playlist_width as f32 + 1.0
            } else {
                0.0
            };
        let height = 2.0 * FRAME + TITLE_H + 1.0 + 1.0 + SEEK_H + 1.0 + CTRL_H;
        vec2(width, height)
    }

    fn chrome_extra_fullscreen(&self) -> Vec2 {
        let width = if self.fs_right {
            self.settings.playlist_width as f32 + 1.0
        } else {
            0.0
        };
        let height = if self.fs_bottom {
            SEEK_H + CTRL_H + 2.0
        } else {
            0.0
        };
        vec2(width, height)
    }

    /// Where each chrome piece goes, given the full window rect.
    fn layout(&self, window: Rect) -> Layout {
        let mut l = Layout::default();
        if self.fullscreen {
            let top = self.fs_top;
            let bottom = self.fs_bottom;
            let requested_right = self.fs_right || self.fs_pin;
            let available_width = window.width().max(0.0);
            let right = requested_right && available_width >= MIN_MAIN_W + MIN_PLAYLIST_W + 1.0;
            let pw = if right {
                (self.settings.playlist_width as f32).clamp(
                    MIN_PLAYLIST_W,
                    (available_width - MIN_MAIN_W - 1.0).max(MIN_PLAYLIST_W),
                )
            } else {
                0.0
            };
            let chrome_right = if right {
                window.right() - pw - 1.0
            } else {
                window.right()
            };
            let ctrl_top = (window.bottom() - CTRL_H).max(window.top());
            let seek_bottom = (ctrl_top - 1.0).max(window.top());
            let seek_top = (seek_bottom - SEEK_H).max(window.top());
            let video_top = if top {
                (window.top() + TITLE_H + 1.0).min(seek_top)
            } else {
                window.top()
            };
            let video_bottom = if bottom {
                (seek_top - 1.0).max(video_top)
            } else {
                window.bottom()
            };
            l.video = Rect::from_min_max(
                pos2(window.left(), video_top),
                pos2(chrome_right, video_bottom),
            );
            if top {
                l.title = Some(Rect::from_min_size(
                    window.min,
                    vec2((chrome_right - window.left()).max(0.0), TITLE_H),
                ));
            }
            if bottom {
                let controls_width = (chrome_right - window.left()).max(0.0);
                let volume_width = VOLUME_W.min((controls_width * 0.32).max(0.0));
                let seek_width = (controls_width - volume_width).max(0.0);
                l.seek = Some(Rect::from_min_max(
                    pos2(window.left(), seek_top),
                    pos2(window.left() + seek_width, seek_bottom),
                ));
                l.volume = Some(Rect::from_min_size(
                    pos2(window.left() + seek_width, seek_top),
                    vec2(volume_width, (seek_bottom - seek_top).max(0.0)),
                ));
                l.ctrl = Some(Rect::from_min_max(
                    pos2(window.left(), ctrl_top),
                    pos2(chrome_right, window.bottom()),
                ));
            }
            if right {
                let py = if top { TITLE_H + 1.0 } else { 0.0 };
                let panel_top = window.top() + py;
                let panel_bottom = (window.bottom()
                    - if bottom { SEEK_H + CTRL_H + 2.0 } else { 0.0 })
                .max(panel_top);
                l.playlist = Some(Rect::from_min_size(
                    pos2(chrome_right + 1.0, panel_top),
                    vec2(pw, panel_bottom - panel_top),
                ));
            }
            l.playlist_right = right;
            l.has_top = top;
            l.has_bottom = bottom;
        } else {
            let content_left = window.left() + FRAME;
            let content_right = window.right() - FRAME;
            let available_width = (content_right - content_left).max(0.0);
            let right =
                self.playlist_visible && available_width >= MIN_MAIN_W + MIN_PLAYLIST_W + 1.0;
            let pw = if right {
                (self.settings.playlist_width as f32).clamp(
                    MIN_PLAYLIST_W,
                    (available_width - MIN_MAIN_W - 1.0).max(MIN_PLAYLIST_W),
                )
            } else {
                0.0
            };
            let x1 = content_right - if right { pw + 1.0 } else { 0.0 };
            let cw = (x1 - content_left).max(0.0);
            l.title = Some(Rect::from_min_size(
                pos2(content_left, window.top() + FRAME),
                vec2(cw, TITLE_H),
            ));
            let vy = window.top() + FRAME + TITLE_H + 1.0;
            let ctrl_top = (window.bottom() - FRAME - CTRL_H).max(window.top() + FRAME);
            let seek_bottom = (ctrl_top - 1.0).max(window.top() + FRAME);
            let sy = (seek_bottom - SEEK_H).max(window.top() + FRAME);
            let volume_width = VOLUME_W.min((cw * 0.32).max(0.0));
            let seek_width = (cw - volume_width).max(0.0);
            l.video = Rect::from_min_max(
                pos2(content_left, vy.min(sy)),
                pos2(content_left + cw, (sy - 1.0).max(vy.min(sy))),
            );
            l.seek = Some(Rect::from_min_size(
                pos2(content_left, sy),
                vec2(seek_width, (seek_bottom - sy).max(0.0)),
            ));
            l.volume = Some(Rect::from_min_size(
                pos2(content_left + seek_width, sy),
                vec2(volume_width, (seek_bottom - sy).max(0.0)),
            ));
            l.ctrl = Some(Rect::from_min_max(
                pos2(content_left, ctrl_top),
                pos2(content_left + cw, window.bottom() - FRAME),
            ));
            if right {
                l.playlist = Some(Rect::from_min_size(
                    pos2(x1 + 1.0, window.top() + FRAME),
                    vec2(pw, (window.height() - 2.0 * FRAME).max(0.0)),
                ));
            }
            l.playlist_right = right;
            l.has_top = true;
            l.has_bottom = true;
        }
        l
    }

    fn update_sub_margin(&mut self) {
        let mut margin = 22.0;
        if self.fullscreen && self.fs_bottom {
            margin += ((SEEK_H + CTRL_H + 2.0) * 720.0 / self.window_height().max(1.0)).round();
        }
        self.set_prop("sub-margin-y", margin as f64);
    }

    fn window_height(&self) -> f32 {
        // The renderer knows the current viewport; we only need an approximate height for the
        // subtitle margin heuristic, so fall back to the configured size.
        self.settings.window_h as f32
    }

    fn remember_window_geometry(&mut self, ctx: &Context) {
        if self.fullscreen || self.maximized {
            return;
        }
        let size = ctx.content_rect().size();
        if size.x.is_finite() && size.y.is_finite() && size.x >= 320.0 && size.y >= 200.0 {
            self.settings.window_w = size.x as f64;
            self.settings.window_h = size.y as f64;
        }
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect)
            && rect.min.x.is_finite()
            && rect.min.y.is_finite()
        {
            self.settings.window_x = Some(rect.min.x as f64);
            self.settings.window_y = Some(rect.min.y as f64);
        }
    }

    // -- property handlers ---------------------------------------------------------------------------

    fn on_prop(&mut self, name: &str, value: Value) {
        // Store first so handlers that read the state map see the new value.
        self.state.insert(name.to_string(), value.clone());
        match name {
            "time-pos" => {
                if value.is_none() || !self.file_ready {
                    return;
                }
                if let Some(v) = value.as_f64() {
                    self.last_pos = v;
                    self.seek.position = v;
                }
            }
            "duration" => {
                let d = value.as_f64().unwrap_or(0.0);
                self.seek.duration = d;
                if d > 0.0 {
                    if let Some(path) = self.current_path.clone() {
                        self.playlist.set_duration(&path, d);
                    }
                }
            }
            "pause" | "hwdec-current" | "paused-for-cache" | "cache-buffering-state" => {}
            "eof-reached" => self.on_eof(value.as_bool()),
            "idle-active" => {
                if value.as_bool() {
                    self.file_ready = false;
                    self.last_pos = 0.0;
                    self.seek.reset();
                    self.osd.center.clear();
                }
                self.update_title();
                self.update_badges();
            }
            "media-title" | "path" => self.update_title(),
            "volume" => {
                if let Some(v) = value.as_f64() {
                    let volume = v.round().clamp(0.0, 100.0) as i64;
                    self.volume.volume = volume;
                    self.settings.volume = volume;
                }
            }
            "mute" => {
                self.volume.muted = value.as_bool();
                self.settings.mute = value.as_bool();
            }
            "track-list" => {
                self.update_badges();
            }
            "chapter-list" => {
                self.seek.chapters = value
                    .as_array()
                    .iter()
                    .filter_map(|c| c.get("time").as_f64())
                    .collect();
            }
            "video-params" => {
                if value.get("dw").as_f64().is_some() && self.autofit_pending {
                    self.autofit_pending = false;
                    self.next_autofit = self.now + 0.05;
                }
                self.update_badges();
            }
            "demuxer-cache-time" => {
                if let Some(path) = &self.current_path {
                    if is_url(path) {
                        self.seek.cache_end = value.as_f64();
                    } else {
                        self.seek.cache_end = None;
                    }
                } else {
                    self.seek.cache_end = None;
                }
            }
            "ab-loop-a" => self.seek.ab.0 = value.as_f64(),
            "ab-loop-b" => self.seek.ab.1 = value.as_f64(),
            _ => {}
        }
        self.refresh_derived();
    }

    fn on_eof(&mut self, reached: bool) {
        if !reached || !self.file_ready {
            return;
        }
        if let Some(path) = self.current_path.clone() {
            self.settings.set_position(&path, None);
        }
        if self.settings.repeat_mode == "one" {
            self.cmd(&["seek", "0", "absolute"]);
            self.set_prop("pause", false);
        } else if let Some(row) =
            self.playlist
                .step(1, &self.settings.repeat_mode, self.settings.shuffle)
        {
            self.play_row(row, false);
        } else {
            self.stop(false);
        }
    }

    fn on_engine_event(&mut self, msg: EngineMsg) {
        match msg {
            EngineMsg::Prop(name, value) => self.on_prop(&name, value),
            EngineMsg::FileLoaded => {
                self.file_ready = true;
                self.fail_count = 0;
                self.aspect_name = "Default".into();
                if let Some(path) = self.current_path.clone() {
                    self.settings.add_recent(&path);
                }
                for sub in std::mem::take(&mut self.pending_subs) {
                    let sub = sub.clone();
                    self.cmd(&["sub-add", &sub, "select"]);
                }
                self.apply_saved_track_preference();
                self.update_title();
                if let Some(at) = self.resumed_at {
                    self.osd(format!(
                        "Resumed at {}  (Backspace: play from the beginning)",
                        fmt_time(at)
                    ));
                } else if let Some(path) = &self.current_path {
                    let title = title_of(path);
                    self.osd(title);
                }
                self.update_title();
            }
            EngineMsg::EndFile {
                reason,
                error,
                entry_id,
            } => {
                let _ = entry_id;
                if reason == ffi::MPV_END_FILE_REASON_ERROR && error != 0 {
                    self.file_ready = false;
                    self.fail_count += 1;
                    let name = title_of(self.current_path.as_deref().unwrap_or(""));
                    self.osd_error(format!("Cannot open: {name}"), 3500);
                    if self.fail_count < 3 && self.playlist.count() > 1 {
                        if let Some(row) = self.playlist.step(1, "off", false) {
                            self.next_autofit = self.now + 1.2;
                            self.autofit_pending = false;
                            self.play_row(row, true);
                        }
                    }
                }
            }
            EngineMsg::StartFile => {}
            EngineMsg::PlaybackRestart | EngineMsg::Seek | EngineMsg::VideoReconfig => {}
            EngineMsg::Log(line) => {
                let _ = line;
            }
        }
    }

    fn refresh_derived(&mut self) {
        self.update_buffering();
        self.update_logo();
        self.update_play_state();
    }

    fn update_buffering(&mut self) {
        if self
            .state
            .get("paused-for-cache")
            .map(|v| v.as_bool())
            .unwrap_or(false)
            && self.has_file()
        {
            let pct = self
                .state
                .get("cache-buffering-state")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            self.osd.center = format!("Buffering {pct:.0}%");
        } else if self.hovered_files.is_empty() {
            self.osd.center.clear();
        }
    }

    fn update_logo(&mut self) {
        // Handled at paint time by reading `logo_title`.
    }

    fn logo_title(&self) -> String {
        let idle = self
            .state
            .get("idle-active")
            .map(|v| v.as_bool())
            .unwrap_or(true);
        let audio_only = self.file_ready && !self.has_video();
        if audio_only && !idle {
            title_of(self.current_path.as_deref().unwrap_or(""))
        } else {
            String::new()
        }
    }

    fn update_play_state(&mut self) {
        self.apply_on_top();
    }

    fn update_badges(&mut self) {
        self.badges.clear();
        if !self.has_file() || !self.file_ready {
            return;
        }
        let tracks = self.tracks().to_vec();
        let video = tracks
            .iter()
            .find(|t| t.get("type").as_str() == Some("video") && t.get("selected").as_bool());
        let audio = tracks
            .iter()
            .find(|t| t.get("type").as_str() == Some("audio") && t.get("selected").as_bool());
        if let Some(video) = video {
            if !video.get("albumart").as_bool() {
                let hw = self
                    .state
                    .get("hwdec-current")
                    .and_then(|v| v.as_str())
                    .unwrap_or("no")
                    != "no";
                self.badges.push(Badge {
                    text: if hw { "H/W" } else { "S/W" }.into(),
                    on: hw,
                });
                let codec = video.get("codec").as_str().unwrap_or("").to_string();
                self.badges.push(Badge {
                    text: skin::codec_label(skin::VIDEO_CODECS, &codec),
                    on: false,
                });
            }
        }
        if let Some(audio) = audio {
            let codec = audio.get("codec").as_str().unwrap_or("").to_string();
            self.badges.push(Badge {
                text: skin::codec_label(skin::AUDIO_CODECS, &codec),
                on: false,
            });
        }
    }

    fn update_title(&mut self) {
        if !self.has_file() || self.current_path.is_none() {
            self.title_text.clear();
            return;
        }
        let path = self.current_path.clone().unwrap_or_default();
        let name = if is_url(&path) {
            self.state
                .get("media-title")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| path.clone())
        } else {
            title_of(&path)
        };
        let count = self.playlist.count();
        let row = self.playlist.current_row();
        let prefix = match (count, row) {
            (c, Some(r)) if c > 1 => format!("[{}/{}] ", r + 1, c),
            _ => String::new(),
        };
        self.title_text = format!("{prefix}{name}");
    }

    fn remember_position(&mut self) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        if !self.file_ready {
            return;
        }
        let pos = self.last_pos;
        let dur = self.duration();
        if self.settings.resume
            && pos > 5.0
            && (dur <= 0.0 || pos < dur - (10.0f64).max(dur * 0.03))
        {
            self.settings.set_position(&path, Some(pos));
        } else {
            self.settings.set_position(&path, None);
        }
    }

    fn autosave(&mut self) {
        if self.is_playing() {
            self.remember_position();
            self.settings.save();
        }
    }

    // -- opening media --------------------------------------------------------------------------------

    fn open_paths(&mut self, paths: &[String], append: bool) {
        let paths: Vec<String> = paths.iter().filter(|p| !p.is_empty()).cloned().collect();
        if paths.is_empty() {
            return;
        }
        if !self.gl_ready {
            self.pending_open.push((paths, append));
            return;
        }
        let mut items: Vec<String> = Vec::new();
        for path in expand_paths(&paths) {
            if !is_url(&path) && PLAYLIST_EXT.contains(&ext(&path).as_str()) && ext(&path) != ".cue"
            {
                items.extend(read_playlist_file(&path));
            } else {
                items.push(path);
            }
        }
        let subs: Vec<String> = items.iter().filter(|p| is_sub(p)).cloned().collect();
        let media: Vec<String> = items.iter().filter(|p| !is_sub(p)).cloned().collect();
        if media.is_empty() {
            if !subs.is_empty() && self.has_file() {
                self.add_subtitles(&subs);
            } else if !subs.is_empty() {
                self.osd_error("Open a video first, then add the subtitle", 3000);
            } else {
                self.osd_error("No playable files found", 3000);
            }
            return;
        }
        let local: Vec<&String> = media.iter().filter(|p| !is_url(p)).collect();
        if let Some(first) = local.first() {
            self.settings.last_dir = Path::new(first.as_str())
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        if append {
            let first = self.playlist.add(&media);
            if !self.has_file() {
                self.pending_subs = subs;
                self.play_row(first, true);
            } else {
                let n = media.len();
                self.osd(format!(
                    "Added {n} file{} to the playlist",
                    if n == 1 { "" } else { "s" }
                ));
            }
            return;
        }
        let (group, row) = if paths.len() == 1 && media.len() == 1 && Path::new(&media[0]).is_file()
        {
            let group = similar_files(&media[0]);
            let row = group.iter().position(|p| *p == media[0]).unwrap_or(0);
            (group, row)
        } else {
            (media, 0)
        };
        self.playlist.set_items(&group);
        self.pending_subs = subs;
        self.play_row(row, true);
    }

    /// Open one or more paths or URLs copied as plain text without opening a
    /// second URL dialog.
    fn open_clipboard(&mut self) {
        let text = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => text,
            Err(error) => {
                self.osd_error(format!("Could not read the clipboard: {error}"), 3500);
                return;
            }
        };
        let paths: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect();
        if paths.is_empty() {
            self.osd_error("The clipboard does not contain a file or URL", 3000);
            return;
        }
        self.open_paths(&paths, false);
    }

    fn play_row(&mut self, row: usize, remember: bool) {
        let Some(path) = self.playlist.path(row).map(str::to_string) else {
            return;
        };
        if remember {
            self.remember_position();
        }
        self.playlist.set_current(Some(row));
        self.current_path = Some(path.clone());
        self.file_ready = false;
        self.last_pos = 0.0;
        self.autofit_pending = true;
        self.seek.reset();
        self.resumed_at = if self.settings.resume {
            self.settings.get_position(&path)
        } else {
            None
        };
        self.set_prop("pause", false);
        if let Some(engine) = self.engine() {
            engine.load(&path, self.resumed_at);
        }
        self.update_title();
    }

    fn add_subtitles(&mut self, paths: &[String]) {
        if !self.has_file() {
            return;
        }
        for path in paths {
            let path = path.clone();
            self.cmd(&["sub-add", &path, "select"]);
        }
        self.set_prop("sub-visibility", true);
        if let Some(last) = paths.last() {
            self.settings.last_sub_dir = Path::new(last.as_str())
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let title = title_of(last);
            self.osd(format!("Subtitle: {title}"));
        }
    }

    fn stop(&mut self, remember: bool) {
        if remember {
            self.remember_position();
        }
        self.file_ready = false;
        self.cmd(&["stop"]);
        self.settings.save();
    }

    fn step_file(&mut self, direction: i32) {
        if self.playlist.count() == 0 {
            return;
        }
        let repeat = self.settings.repeat_mode.clone();
        let shuffle = self.settings.shuffle;
        match self.playlist.step(direction, &repeat, shuffle) {
            Some(row) => self.play_row(row, true),
            None => self.osd(if direction > 0 {
                "Last file"
            } else {
                "First file"
            }),
        }
    }

    // -- playback actions ------------------------------------------------------------------------------

    fn toggle_pause(&mut self) {
        if !self.has_file() {
            if self.playlist.count() > 0 {
                let row = self.playlist.current_row().unwrap_or(0);
                self.play_row(row.max(0), true);
            }
            return;
        }
        if self.get_bool("eof-reached") {
            self.cmd(&["seek", "0", "absolute"]);
            self.set_prop("pause", false);
            return;
        }
        let paused = !self.get_bool("pause");
        self.set_prop("pause", paused);
        self.osd(if paused { "Pause" } else { "Play" });
    }

    fn seek_osd(&mut self, target: f64) {
        let dur = self.duration();
        let text = if dur > 0.0 {
            format!("{} / {}", fmt_time(target), fmt_time(dur))
        } else {
            fmt_time(target)
        };
        self.osd(text);
    }

    fn seek_rel(&mut self, amount: f64) {
        if !self.has_file() || !self.file_ready {
            return;
        }
        let base = if self.now - self.seek_target.0 < 0.6 {
            self.seek_target.1
        } else {
            self.last_pos
        };
        let dur = self.duration();
        let mut target = (base + amount).max(0.0);
        if dur > 0.0 {
            target = target.min(dur);
        }
        self.seek_target = (self.now, target);
        self.cmd(&["seek", &format!("{amount}"), "relative"]);
        self.seek_osd(target);
    }

    fn seek_abs(&mut self, seconds: f64) {
        if !self.has_file() {
            return;
        }
        self.cmd(&["seek", &format!("{}", seconds.max(0.0)), "absolute"]);
        self.seek_osd(seconds);
    }

    fn seek_bar_seek(&mut self, seconds: f64, exact: bool) {
        if !self.has_file() {
            return;
        }
        let mode = if exact {
            "absolute+exact"
        } else {
            "absolute+keyframes"
        };
        self.cmd(&["seek", &format!("{seconds:.3}"), mode]);
        self.seek.position = seconds;
    }

    fn frame_step(&mut self, direction: i32) {
        if !self.has_file() {
            return;
        }
        self.cmd(&[if direction > 0 {
            "frame-step"
        } else {
            "frame-back-step"
        }]);
        self.osd_ms(
            if direction > 0 {
                "Next frame"
            } else {
                "Previous frame"
            },
            900,
        );
    }

    fn set_speed(&mut self, value: f64) {
        let value = value.clamp(0.2, 12.0).round() * 100.0 / 100.0;
        self.set_prop("speed", value);
        self.osd(format!("Speed: {value:.1}x"));
    }

    fn change_speed(&mut self, delta: f64) {
        let current = self.get_f64("speed");
        let current = if current == 0.0 { 1.0 } else { current };
        self.set_speed(current + delta);
    }

    fn ab_set(&mut self, which: char) {
        if !self.has_file() {
            return;
        }
        let pos = self.position();
        let prop = format!("ab-loop-{which}");
        self.set_prop(&prop, pos);
        self.osd(format!(
            "A-B repeat: {} point {}",
            if which == 'a' { "start" } else { "end" },
            fmt_time(pos)
        ));
    }

    fn ab_clear(&mut self) {
        self.set_prop("ab-loop-a", "no");
        self.set_prop("ab-loop-b", "no");
        self.osd("A-B repeat: off");
    }

    fn chapter_step(&mut self, direction: i32) {
        if self
            .state
            .get("chapter-list")
            .map(|v| v.as_array().is_empty())
            .unwrap_or(true)
            || !self.has_file()
        {
            self.osd("No chapters");
            return;
        }
        self.cmd(&["add", "chapter", &direction.to_string()]);
        self.osd(if direction > 0 {
            "Next chapter"
        } else {
            "Previous chapter"
        });
    }

    fn set_chapter(&mut self, index: i64) {
        self.set_prop("chapter", index);
    }

    fn set_repeat(&mut self, mode: &str) {
        self.settings.repeat_mode = mode.to_string();
        self.playlist.repeat_mode = mode.to_string();
        let shuffle = self.settings.shuffle;
        self.playlist.shuffle = shuffle;
        self.osd(match mode {
            "off" => "Repeat: off",
            "one" => "Repeat: current file",
            _ => "Repeat: playlist",
        });
    }

    fn cycle_repeat(&mut self) {
        let order = ["off", "one", "all"];
        let current = order
            .iter()
            .position(|m| *m == self.settings.repeat_mode)
            .unwrap_or(0);
        let next = order[(current + 1) % 3];
        self.set_repeat(next);
    }

    fn toggle_shuffle(&mut self) {
        self.settings.shuffle = !self.settings.shuffle;
        self.playlist.shuffle = self.settings.shuffle;
        self.osd(format!(
            "Shuffle: {}",
            if self.settings.shuffle { "on" } else { "off" }
        ));
    }

    fn toggle_resume(&mut self) {
        self.settings.resume = !self.settings.resume;
        self.osd(format!(
            "Remember playback position: {}",
            if self.settings.resume { "on" } else { "off" }
        ));
    }

    fn add_bookmark(&mut self) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        if !self.file_ready {
            return;
        }
        let time = self.position();
        let title = title_of(&path);
        if self.settings.add_bookmark(&path, time, &title) {
            self.settings.save();
            self.osd(format!("Bookmark added at {}", fmt_time(time)));
        }
    }

    fn seek_bookmark(&mut self, path: &str, time: f64) {
        if self.current_path.as_deref() == Some(path) {
            self.seek_abs(time);
        } else {
            self.pending.open_paths(vec![path.to_string()], false);
        }
    }

    // -- subtitles --------------------------------------------------------------------------------------

    fn toggle_sub_visibility(&mut self) {
        let current = self
            .state
            .get("sub-visibility")
            .map(|v| v.as_bool())
            .unwrap_or_else(|| self.get_bool("sub-visibility"));
        let visible = !current;
        self.set_prop("sub-visibility", visible);
        self.settings.sub_visible = visible;
        self.osd(format!(
            "Subtitles: {}",
            if visible { "show" } else { "hide" }
        ));
    }

    fn set_track(&mut self, kind: TrackKind, sel: TrackSel) {
        self.set_prop(kind.prop(), sel.as_str());
        let label = kind.label();
        if matches!(sel, TrackSel::No) {
            self.osd(format!("{label}: off"));
            return;
        }
        if kind == TrackKind::Sub {
            self.set_prop("sub-visibility", true);
        }
        let tracks = self.tracks().to_vec();
        let wanted = match sel {
            TrackSel::Id(id) => Some(id),
            _ => None,
        };
        let found = tracks.iter().find(|t| {
            t.get("type").as_str() == Some(kind.type_name()) && t.get("id").as_i64() == wanted
        });
        let text = match found {
            Some(t) => skin::track_label(t),
            None => sel.as_str(),
        };
        let selection = match sel {
            TrackSel::Auto => TrackSelection::Auto,
            TrackSel::Id(id) => TrackSelection::Id(id),
            TrackSel::No => TrackSelection::None,
        };
        self.remember_track_preference(kind, selection);
        self.osd_ms(format!("{label}: {text}"), 2200);
    }

    fn cycle_track(&mut self, kind: TrackKind) {
        if !self.has_file() {
            return;
        }
        let tracks = self.tracks().to_vec();
        let ids: Vec<i64> = tracks
            .iter()
            .filter(|t| t.get("type").as_str() == Some(kind.type_name()))
            .filter_map(|t| t.get("id").as_i64())
            .collect();
        if ids.is_empty() {
            self.osd(format!("No {} tracks", kind.plural()));
            return;
        }
        let mut choices: Vec<TrackSel> = ids.into_iter().map(TrackSel::Id).collect();
        if kind == TrackKind::Sub {
            choices.push(TrackSel::No);
        }
        let current = tracks
            .iter()
            .find(|t| {
                t.get("type").as_str() == Some(kind.type_name()) && t.get("selected").as_bool()
            })
            .and_then(|t| t.get("id").as_i64())
            .map(TrackSel::Id)
            .unwrap_or(TrackSel::No);
        let index = choices
            .iter()
            .position(|c| *c == current)
            .map(|i| i as i32)
            .unwrap_or(-1);
        let next = choices[((index + 1) as usize) % choices.len()];
        self.set_track(kind, next);
    }

    fn sub_delay(&mut self, delta: Option<f64>) {
        let value = match delta {
            None => 0.0,
            Some(d) => (self.get_f64("sub-delay") + d).round() * 1000.0 / 1000.0,
        };
        self.set_prop("sub-delay", value);
        self.osd(format!("Subtitle sync: {value:+.1} sec"));
    }

    fn sub_scale(&mut self, delta: f64) {
        let value = if let Some(engine) = self.engine() {
            engine.add("sub-scale", delta, Some(0.3), Some(3.0))
        } else {
            1.0
        };
        self.osd(format!("Subtitle size: {:.0}%", value * 100.0));
    }

    fn sub_move(&mut self, direction: i32) {
        let value = if let Some(engine) = self.engine() {
            engine.add("sub-pos", (direction * 2) as f64, Some(0.0), Some(150.0))
        } else {
            100.0
        };
        self.osd(format!("Subtitle position: {value:.0}"));
    }

    fn sub_reset(&mut self) {
        self.set_prop("sub-scale", 1.0);
        self.set_prop("sub-pos", self.settings.sub_pos as f64);
        self.set_prop("sub-delay", 0.0);
        self.osd("Subtitle size and position reset");
    }

    fn set_sub_codepage(&mut self, codepage: &str) {
        self.settings.sub_codepage = codepage.to_string();
        self.set_prop("sub-codepage", codepage);
        if self.has_file() {
            self.cmd(&["sub-reload"]);
        }
        self.osd(format!("Subtitle encoding: {codepage}"));
    }

    // -- video -----------------------------------------------------------------------------------------

    fn set_aspect(&mut self, label: &str, value: &str) {
        self.aspect_name = label.to_string();
        self.set_prop("video-aspect-override", value);
        self.osd(format!("Aspect ratio: {label}"));
    }

    fn set_display_mode(&mut self, mode: &str) {
        self.display_mode = mode.to_string();
        self.set_prop("keepaspect", mode != "stretch");
        self.set_prop("panscan", if mode == "fill" { 1.0 } else { 0.0 });
        self.set_prop("video-unscaled", mode == "original");
        self.osd(match mode {
            "fit" => "Keep aspect ratio",
            "fill" => "Fill window (crop)",
            "stretch" => "Stretch to window",
            _ => "Original size",
        });
    }

    fn zoom(&mut self, delta: f64) {
        let value = if let Some(engine) = self.engine() {
            engine.add("video-zoom", delta, Some(-2.0), Some(3.0))
        } else {
            0.0
        };
        self.osd(format!("Zoom: {:.0}%", 2f64.powf(value) * 100.0));
    }

    fn pan(&mut self, dx: f64, dy: f64) {
        if let Some(engine) = self.engine() {
            if dx != 0.0 {
                engine.add("video-pan-x", dx, Some(-3.0), Some(3.0));
            }
            if dy != 0.0 {
                engine.add("video-pan-y", dy, Some(-3.0), Some(3.0));
            }
        }
    }

    fn reset_geometry(&mut self) {
        for prop in ["video-zoom", "video-pan-x", "video-pan-y"] {
            self.set_prop(prop, 0.0);
        }
        self.set_aspect("Default", "-1");
        self.set_display_mode("fit");
        self.osd("Pan & Scan reset");
    }

    fn rotate(&mut self, degrees: Option<i64>) {
        let degrees = match degrees {
            Some(d) => d,
            None => ((self.get_f64("video-rotate") as i64).rem_euclid(360) + 90).rem_euclid(360),
        };
        self.set_prop("video-rotate", degrees);
        self.osd(format!("Rotation: {degrees}°"));
    }

    fn toggle_flip(&mut self, name: &'static str) {
        let on = if name == "hflip" {
            !self.flips.0
        } else {
            !self.flips.1
        };
        if name == "hflip" {
            self.flips.0 = on;
        } else {
            self.flips.1 = on;
        }
        self.cmd(&["vf", "toggle", &format!("@fp{name}:{name}")]);
        self.osd(format!(
            "{}: {}",
            if name == "hflip" {
                "Flip horizontally"
            } else {
                "Flip vertically"
            },
            if on { "on" } else { "off" }
        ));
    }

    fn image_adjust(&mut self, prop: &str, direction: i32) {
        let value = if let Some(engine) = self.engine() {
            engine.add(prop, (direction * 2) as f64, Some(-100.0), Some(100.0))
        } else {
            0.0
        };
        let name = prop
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let rest: String = prop.chars().skip(1).collect();
        self.osd(format!("{name}{rest}: {value:+.0}"));
    }

    fn image_reset(&mut self) {
        for prop in ["brightness", "contrast", "saturation", "hue", "gamma"] {
            self.set_prop(prop, 0.0);
        }
        self.osd("Image adjustments reset");
    }

    fn toggle_deinterlace(&mut self) {
        let value = !self.get_bool("deinterlace");
        self.set_prop("deinterlace", value);
        self.osd(format!("Deinterlace: {}", if value { "on" } else { "off" }));
    }

    fn set_hwdec(&mut self, enabled: bool) {
        self.settings.hwdec = enabled;
        self.set_prop("hwdec", Engine::hwdec_value(enabled));
        self.osd(format!(
            "Hardware decoding: {}",
            if enabled { "on" } else { "off" }
        ));
    }

    // -- audio -----------------------------------------------------------------------------------------

    fn set_volume(&mut self, value: i64) {
        let value = value.clamp(0, 100);
        self.set_prop("volume", value);
        if value > 0 && self.state.get("mute").map(|v| v.as_bool()).unwrap_or(false) {
            self.set_prop("mute", false);
        }
        self.volume.volume = value;
        self.osd(format!("Volume: {value}%"));
    }

    fn volume_step(&mut self, direction: i32) {
        let step = self.settings.volume_step as i64;
        let current = self.volume.volume;
        self.set_volume(current + direction as i64 * step);
    }

    fn toggle_mute(&mut self) {
        let muted = !self.get_bool("mute");
        self.set_prop("mute", muted);
        self.osd(format!("Mute: {}", if muted { "on" } else { "off" }));
    }

    fn audio_delay(&mut self, delta: Option<f64>) {
        let value = match delta {
            None => 0.0,
            Some(d) => (self.get_f64("audio-delay") + d).round() * 1000.0 / 1000.0,
        };
        self.set_prop("audio-delay", value);
        self.osd(format!("Audio sync: {value:+.2} sec"));
    }

    fn set_normalize(&mut self, enabled: bool) {
        if enabled == self.normalize {
            return;
        }
        if enabled {
            self.cmd(&[
                "af",
                "add",
                "@fpnorm:lavfi=[dynaudnorm=g=5:f=250:r=0.9:p=1]",
            ]);
        } else {
            self.cmd(&["af", "remove", "@fpnorm"]);
        }
        self.normalize = enabled;
        self.osd(format!(
            "Normalize volume: {}",
            if enabled { "on" } else { "off" }
        ));
    }

    fn apply_eq(&mut self) {
        let gains: Vec<f64> = self.settings.eq.iter().take(10).copied().collect();
        let bands: Vec<(f64, f64)> = EQ_FREQS
            .iter()
            .zip(gains.iter())
            .filter(|(_, g)| **g != 0.0)
            .map(|(f, g)| (*f, *g))
            .collect();
        if self.settings.eq_enabled && !bands.is_empty() {
            let chain: Vec<String> = bands
                .iter()
                .map(|(f, g)| format!("equalizer=f={f}:t=o:w=1:g={g}"))
                .collect();
            let spec = format!("@fpeq:lavfi=[{}]", chain.join(","));
            self.cmd(&["af", "add", &spec]);
            self.eq_active = true;
        } else if self.eq_active {
            self.cmd(&["af", "remove", "@fpeq"]);
            self.eq_active = false;
        }
    }

    fn screenshot_dir(&self) -> String {
        let folder = self.settings.screenshot_dir.clone();
        if folder.is_empty() {
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
            return format!("{home}/Pictures/MacPlayer");
        }
        shellexpand(&folder)
    }

    fn screenshot(&mut self, mode: &str, clipboard: bool) {
        if !self.has_file() {
            return;
        }
        if clipboard {
            let path = std::env::temp_dir().join(format!(
                "macplayer-frame-{}-{}.png",
                std::process::id(),
                self.now.to_bits()
            ));
            let path_text = path.to_string_lossy().to_string();
            let captured = self
                .engine()
                .and_then(|engine| engine.cmd_sync(&["screenshot-to-file", &path_text, mode]))
                .is_some();
            if !captured {
                self.osd_error("Could not capture the current frame", 3000);
                return;
            }
            let result = std::fs::read(&path)
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|image| image.to_rgba8());
            let _ = std::fs::remove_file(&path);
            let Some(image) = result else {
                self.osd_error("Could not read the captured frame", 3000);
                return;
            };
            let (width, height) = image.dimensions();
            let data = arboard::ImageData {
                width: width as usize,
                height: height as usize,
                bytes: Cow::Owned(image.into_raw()),
            };
            match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(data)) {
                Ok(()) => self.osd_ms("Frame copied to clipboard", 2200),
                Err(error) => self.osd_error(format!("Could not copy frame: {error}"), 3000),
            }
            return;
        }
        let folder = self.screenshot_dir();
        if std::fs::create_dir_all(&folder).is_err() {
            self.osd_error(format!("Cannot create folder: {folder}"), 3500);
            return;
        }
        let stem = title_of(self.current_path.as_deref().unwrap_or("frame"));
        let stem: String = stem.chars().take(80).collect();
        let stem = stem.replace('/', "_");
        let pos = self.position();
        let base = format!(
            "{}_{}.{:03}",
            stem.trim_end_matches(".png"),
            fmt_time(pos).replace(':', "."),
            ((pos * 1000.0) as i64).rem_euclid(1000)
        );
        let mut path = format!("{folder}/{base}.png");
        let mut counter = 1;
        while Path::new(&path).exists() {
            counter += 1;
            path = format!("{folder}/{base}_{counter}.png");
        }
        self.cmd(&["screenshot-to-file", &path, mode]);
        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.osd_ms(format!("Saved: {name}"), 2500);
    }

    fn open_screenshot_folder(&mut self) {
        let folder = self.screenshot_dir();
        let _ = std::fs::create_dir_all(&folder);
        open_folder(&folder);
    }

    // -- window ----------------------------------------------------------------------------------------

    fn toggle_maximize(&mut self, ctx: &Context) {
        if self.fullscreen {
            return;
        }
        self.state_changed = self.now;
        self.maximized = !self.maximized;
        self.request_window_mode(ctx);
    }

    fn toggle_fullscreen(&mut self, ctx: &Context) {
        self.state_changed = self.now;
        if self.fullscreen {
            self.fullscreen = false;
            self.fs_top = false;
            self.fs_bottom = false;
            self.fs_right = false;
            self.fs_top_deadline = 0.0;
            self.fs_bottom_deadline = 0.0;
            self.maximized = self.was_maximized;
        } else {
            self.was_maximized = self.maximized;
            self.fs_top = false;
            self.fs_bottom = false;
            self.fs_right = false;
            self.fs_top_deadline = 0.0;
            self.fs_bottom_deadline = 0.0;
            self.fullscreen = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::SetMonitor(
                self.settings.fullscreen_monitor,
            ));
        }
        self.request_window_mode(ctx);
    }

    fn request_window_mode(&self, ctx: &Context) {
        if self.fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            if self.maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        }));
    }

    fn set_window_scale(&mut self, percent: i32, ctx: &Context) {
        if !self.has_file() || !self.has_video() {
            return;
        }
        if self.fullscreen || self.maximized {
            self.osd("Window size is not available in full screen");
            return;
        }
        let limited = self.fit_window(Some(percent as f64 / 100.0), ctx);
        self.osd(format!(
            "Window size: {percent}%{}",
            if limited { " (limited by screen)" } else { "" }
        ));
    }

    fn fit_window(&mut self, scale: Option<f64>, ctx: &Context) -> bool {
        let params = self.state.get("video-params");
        let (dw, dh) = match params {
            Some(p) => (p.get("dw").as_f64(), p.get("dh").as_f64()),
            None => (None, None),
        };
        let (Some(dw), Some(dh)) = (dw, dh) else {
            return false;
        };
        if !dw.is_finite() || !dh.is_finite() || dw <= 0.0 || dh <= 0.0 {
            return false;
        }
        if self.fullscreen || self.maximized {
            return false;
        }
        let rotate = params
            .as_ref()
            .map(|p| p.get("rotate").as_f64().unwrap_or(0.0) as i64)
            .unwrap_or(0);
        let extra_rot = self.get_f64("video-rotate") as i64;
        let rotation = (rotate + extra_rot).rem_euclid(180);
        let (dw, dh) = if rotation == 90 { (dh, dw) } else { (dw, dh) };
        let extra = self.chrome_extra();
        let (screen_w, screen_h) = screen_size(ctx);
        let mut limited = false;
        let (w, h, factor) = match scale {
            None => {
                let pct = (self.settings.autofit_max_pct.clamp(30, 100) as f64) / 100.0;
                let w = dw;
                let h = dh;
                let factor = 1.0f64
                    .min((screen_w * pct - extra.x as f64) / w)
                    .min((screen_h * pct - extra.y as f64) / h)
                    .max(0.01);
                (w, h, factor)
            }
            Some(scale) => {
                let w = dw * scale;
                let h = dh * scale;
                let factor = 1.0f64
                    .min((screen_w - extra.x as f64) / w)
                    .min((screen_h - extra.y as f64) / h)
                    .max(0.01);
                limited = factor < 1.0;
                (w, h, factor)
            }
        };
        let out_w = (w * factor).round().max(320.0);
        let out_h = (h * factor).round().max(90.0);
        self.settings.window_w = out_w + extra.x as f64;
        self.settings.window_h = out_h + extra.y as f64;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
            self.settings.window_w as f32,
            self.settings.window_h as f32,
        )));
        limited
    }

    fn toggle_autofit(&mut self) {
        self.settings.autofit = !self.settings.autofit;
    }

    fn toggle_playlist(&mut self, ctx: &Context) {
        if self.fullscreen {
            self.fs_pin = !self.fs_pin;
            self.update_sub_margin();
            ctx.request_repaint();
            return;
        }
        // Use the width egui actually allocated to the panel when closing.  The
        // layout clamps an oversized playlist to preserve the minimum player
        // width, so subtracting the configured width could leave the window
        // offset from the geometry it had before the panel was opened.
        let allocated_playlist_width = self
            .layout(ctx.content_rect())
            .playlist
            .map(|rect| rect.width());
        self.playlist_visible = !self.playlist_visible;
        self.settings.playlist_visible = self.playlist_visible;
        let delta = playlist_resize_delta(
            allocated_playlist_width,
            self.settings.playlist_width as f32,
        );
        if !self.maximized {
            let (w, h) = window_size(ctx);
            let new_w = w + if self.playlist_visible { delta } else { -delta };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(new_w.max(320.0), h)));
        }
        self.update_sub_margin();
        // Viewport commands are applied after the current frame.  Ensure egui
        // renders a frame with the new visibility state immediately instead of
        // relying on the regular playback repaint timer.
        ctx.request_repaint();
    }

    fn set_on_top(&mut self, mode: &str) {
        self.settings.on_top = mode.to_string();
        self.on_top = mode != "never";
        self.osd(match mode {
            "never" => "On top: never",
            "always" => "On top: always",
            _ => "On top: while playing",
        });
    }

    fn cycle_on_top(&mut self) {
        let order = ["never", "always", "playing"];
        let index = order
            .iter()
            .position(|m| *m == self.settings.on_top)
            .unwrap_or(0);
        let next = order[(index + 1) % 3];
        self.set_on_top(next);
    }

    fn apply_on_top(&mut self) {
        let mode = self.settings.on_top.clone();
        let want = mode == "always" || (mode == "playing" && self.is_playing());
        if want != self.on_top && !self.fullscreen {
            self.on_top = want;
        }
    }

    fn toggle_remaining(&mut self) {
        self.show_remaining = !self.show_remaining;
        self.settings.show_remaining = self.show_remaining;
    }

    fn toggle_info(&mut self) {
        self.info_visible = !self.info_visible;
        if self.info_visible {
            self.next_info = self.now + 0.4;
        } else {
            self.osd.info.clear();
        }
    }

    fn info_text(&mut self, brief: bool) -> String {
        let engine = match &self.engine {
            Some(engine) => engine,
            None => return "No file is open.".into(),
        };
        if !self.has_file() {
            return "No file is open.".into();
        }
        let path = self.current_path.clone().unwrap_or_default();
        let mut lines = vec![format!(
            "File      {}",
            if brief { title_of(&path) } else { path.clone() }
        )];
        let details: Vec<String> = [
            engine.get("file-size").map(|v| human_size(v.as_f64())),
            engine
                .get("file-format")
                .and_then(|v| v.as_str().map(str::to_string)),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !details.is_empty() {
            lines.push(format!("          {}", details.join(", ")));
        }
        let params = self.state.get("video-params").cloned();
        let tracks = self.tracks().to_vec();
        let video = tracks
            .iter()
            .find(|t| t.get("type").as_str() == Some("video") && t.get("selected").as_bool());
        if let (Some(video), Some(params)) = (video, params) {
            let fps = engine
                .get("container-fps")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let fps = if fps == 0.0 {
                engine
                    .get("estimated-vf-fps")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
            } else {
                fps
            };
            let hw = self
                .state
                .get("hwdec-current")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "no".into());
            lines.push(format!(
                "Video     {}  {}x{}  {:.3} fps  {}  {}",
                video.get("codec").as_str().unwrap_or("").to_uppercase(),
                params.get("w").as_f64().unwrap_or(0.0) as i64,
                params.get("h").as_f64().unwrap_or(0.0) as i64,
                fps,
                params.get("pixelformat").as_str().unwrap_or(""),
                if hw != "no" {
                    format!("hardware ({hw})")
                } else {
                    "software".into()
                },
            ));
            let bitrate = engine.get("video-bitrate").and_then(|v| v.as_f64());
            if let Some(bitrate) = bitrate {
                if !brief {
                    lines.push(format!("          {:.0} kb/s", bitrate / 1000.0));
                }
            }
        }
        let audio = tracks
            .iter()
            .find(|t| t.get("type").as_str() == Some("audio") && t.get("selected").as_bool());
        if let Some(audio) = audio {
            let ap = engine.get("audio-params");
            let lang = audio.get("lang").as_str().unwrap_or("").to_string();
            let lang_code = lang.to_lowercase();
            let lang_name = skin::lang_name(&lang_code)
                .unwrap_or(lang.as_str())
                .to_string();
            lines.push(format!(
                "Audio     {}  {} Hz  {} ch  {}",
                audio.get("codec").as_str().unwrap_or("").to_uppercase(),
                ap.as_ref()
                    .map(|a| a.get("samplerate").as_f64().unwrap_or(0.0) as i64)
                    .unwrap_or(0),
                ap.as_ref()
                    .map(|a| a.get("channel-count").as_f64().unwrap_or(0.0) as i64)
                    .unwrap_or(0),
                lang_name,
            ));
        }
        let sub = tracks
            .iter()
            .find(|t| t.get("type").as_str() == Some("sub") && t.get("selected").as_bool());
        if let Some(sub) = sub {
            lines.push(format!(
                "Subtitle  {}  sync {:+.1} sec",
                skin::track_label(sub).replace("&&", "&"),
                engine.get_f64("sub-delay"),
            ));
        }
        lines.push(format!(
            "Playback  {} / {}  {:.2}x  A/V {:+.3}  dropped {}",
            fmt_time(self.position()),
            fmt_time(self.duration()),
            engine.get_f64("speed"),
            engine.get_f64("avsync"),
            (engine.get_f64("frame-drop-count") + engine.get_f64("decoder-frame-drop-count"))
                as i64,
        ));
        if !brief {
            lines.push(String::new());
            lines.push("Tracks".into());
            for track in &tracks {
                lines.push(format!(
                    "  {:<6} {}{}",
                    track.get("type").as_str().unwrap_or(""),
                    skin::track_label(track).replace("&&", "&"),
                    if track.get("selected").as_bool() {
                        "  *"
                    } else {
                        ""
                    },
                ));
            }
            if let Some(chapters) = self.state.get("chapter-list") {
                let arr = chapters.as_array();
                if !arr.is_empty() {
                    lines.push(String::new());
                    lines.push("Chapters".into());
                    for (i, chapter) in arr.iter().enumerate() {
                        lines.push(format!(
                            "  {:>2}. {}  {}",
                            i + 1,
                            fmt_time(chapter.get("time").as_f64().unwrap_or(0.0)),
                            chapter.get("title").as_str().unwrap_or(""),
                        ));
                    }
                }
            }
            lines.push(String::new());
            lines.push("System".into());
            let mpv_version = engine
                .get("mpv-version")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "?".into());
            let ffmpeg_version = engine
                .get("ffmpeg-version")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "?".into());
            lines.push(format!("  mpv      {mpv_version}"));
            lines.push(format!("  ffmpeg   {ffmpeg_version}"));
            let logs = engine.log_lines();
            if !logs.is_empty() {
                lines.push(String::new());
                lines.push("Recent engine messages".into());
                let start = logs.len().saturating_sub(15);
                for line in &logs[start..] {
                    lines.push(format!("  {line}"));
                }
            }
        }
        lines.join("\n")
    }

    fn apply_settings(&mut self, hwdec_changed: bool) {
        let s = &self.settings;
        self.set_prop("sub-font-size", s.sub_size);
        self.set_prop("sub-bold", s.sub_bold);
        self.set_prop(
            "sub-font",
            if s.sub_font.is_empty() {
                "sans-serif"
            } else {
                s.sub_font.as_str()
            },
        );
        self.set_prop("sub-auto", s.sub_auto.as_str());
        self.set_prop("slang", s.preferred_sub_lang.as_str());
        self.set_prop("alang", s.preferred_audio_lang.as_str());
        let current = self
            .engine()
            .and_then(|e| e.get("sub-codepage"))
            .and_then(|v| v.as_str().map(str::to_string));
        if current.as_deref() != Some(s.sub_codepage.as_str()) {
            self.set_prop("sub-codepage", s.sub_codepage.as_str());
            if self.has_file() {
                self.cmd(&["sub-reload"]);
            }
        }
        if hwdec_changed {
            let hwdec = s.hwdec;
            self.set_prop("hwdec", Engine::hwdec_value(hwdec));
        }
        self.set_prop(
            "target-trc",
            if s.hdr_tone_mapping { "srgb" } else { "auto" },
        );
        self.set_prop(
            "target-prim",
            if s.hdr_tone_mapping { "bt.709" } else { "auto" },
        );
        self.set_prop("target-peak", if s.hdr_tone_mapping { 203.0 } else { 0.0 });
        self.set_prop(
            "tone-mapping",
            if s.hdr_tone_mapping {
                s.hdr_tone_mapping_mode.as_str()
            } else {
                "auto"
            },
        );
        self.set_prop("hdr-compute-peak", s.hdr_tone_mapping);
    }

    fn quit(&mut self) {
        self.closing = true;
        self.remember_position();
        self.settings.first_run = false;
        self.settings.save();
    }

    // -- dialogs ---------------------------------------------------------------------------------------

    fn open_files_dialog(&mut self, append: bool) {
        let start = if self.settings.last_dir.is_empty() {
            home_dir()
                .map(|h| format!("{h}/Movies"))
                .unwrap_or_default()
        } else {
            self.settings.last_dir.clone()
        };
        if let Some(files) = rfd::FileDialog::new()
            .set_directory(&start)
            .add_filter("Media files", &media_extensions())
            .pick_files()
        {
            let paths: Vec<String> = files
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            self.pending.open_paths(paths, append);
        }
    }

    fn open_folder_dialog(&mut self, append: bool) {
        let start = if self.settings.last_dir.is_empty() {
            home_dir()
                .map(|h| format!("{h}/Movies"))
                .unwrap_or_default()
        } else {
            self.settings.last_dir.clone()
        };
        if let Some(folder) = rfd::FileDialog::new().set_directory(&start).pick_folder() {
            self.pending
                .open_paths(vec![folder.to_string_lossy().to_string()], append);
        }
    }

    fn load_subtitle_dialog(&mut self) {
        if !self.has_file() {
            self.osd("Open a video first");
            return;
        }
        let mut start = if self.settings.last_sub_dir.is_empty() {
            self.settings.last_dir.clone()
        } else {
            self.settings.last_sub_dir.clone()
        };
        if let Some(path) = &self.current_path {
            if !is_url(path) {
                start = Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
            }
        }
        if let Some(files) = rfd::FileDialog::new()
            .set_directory(&start)
            .add_filter("Subtitle files", &sub_extensions())
            .pick_files()
        {
            let paths: Vec<String> = files
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            self.add_subtitles(&paths);
        }
    }

    fn load_audio_dialog(&mut self) {
        if !self.has_file() {
            return;
        }
        let start = self
            .current_path
            .as_deref()
            .filter(|p| !is_url(p))
            .and_then(|p| {
                Path::new(p)
                    .parent()
                    .map(|d| d.to_string_lossy().to_string())
            })
            .unwrap_or_default();
        if let Some(path) = rfd::FileDialog::new().set_directory(&start).pick_file() {
            let path = path.to_string_lossy().to_string();
            self.cmd(&["audio-add", &path, "select"]);
            let title = title_of(&path);
            self.osd_ms(format!("Audio: {title}"), 2500);
        }
    }

    // -- action dispatch -------------------------------------------------------------------------------

    fn dispatch(&mut self, action: Action, ctx: &Context) {
        match action {
            Action::OpenFiles { append } => self.open_files_dialog(append),
            Action::OpenFolder { append } => self.open_folder_dialog(append),
            Action::OpenUrl => self.url_dialog.open = true,
            Action::OpenClipboard => self.open_clipboard(),
            Action::OpenRecent(path) => self.pending.open_paths(vec![path], false),
            Action::ClearRecent => {
                self.settings.recent.clear();
                self.settings.save();
            }
            Action::PlayRow(row) => self.play_row(row, true),
            Action::Stop => self.stop(true),
            Action::TogglePause => self.toggle_pause(),
            Action::PrevFile => self.step_file(-1),
            Action::NextFile => self.step_file(1),
            Action::SeekRel(amount) => self.seek_rel(amount),
            Action::SeekAbs(seconds) => self.seek_abs(seconds),
            Action::SeekBar(seconds, exact) => self.seek_bar_seek(seconds, exact),
            Action::SeekTo { seconds, percent } => {
                if let Some(s) = seconds {
                    self.seek_abs(s);
                } else if let Some(p) = percent {
                    let dur = self.duration();
                    if dur > 0.0 {
                        self.seek_abs(p * dur);
                    }
                }
            }
            Action::FrameStep(direction) => self.frame_step(direction),
            Action::ChangeSpeed(delta) => self.change_speed(delta),
            Action::SetSpeed(value) => self.set_speed(value),
            Action::OpenGoTo => {
                if self.has_file() {
                    self.goto_dialog.open(GoToMode::Time);
                }
            }
            Action::AbSet(which) => self.ab_set(which),
            Action::AbClear => self.ab_clear(),
            Action::ChapterStep(direction) => self.chapter_step(direction),
            Action::SetChapter(index) => self.set_chapter(index),
            Action::SetRepeat(mode) => self.set_repeat(&mode),
            Action::ToggleShuffle => self.toggle_shuffle(),
            Action::ToggleResume => self.toggle_resume(),
            Action::AddBookmark => self.add_bookmark(),
            Action::RemoveBookmark { path, time } => {
                if self.settings.remove_bookmark(&path, time) {
                    self.settings.save();
                }
            }
            Action::SeekBookmark { path, time } => self.seek_bookmark(&path, time),
            Action::ToggleAutofit => self.toggle_autofit(),
            Action::SetWindowScale(percent) => self.set_window_scale(percent, ctx),

            Action::ToggleSubVisibility => self.toggle_sub_visibility(),
            Action::SetTrack(kind, sel) => self.set_track(kind, sel),
            Action::CycleTrack(kind) => self.cycle_track(kind),
            Action::SetSecondarySub(sel) => {
                self.set_prop("secondary-sid", sel.as_str());
                let selection = match sel {
                    TrackSel::Auto => TrackSelection::Auto,
                    TrackSel::Id(id) => TrackSelection::Id(id),
                    TrackSel::No => TrackSelection::None,
                };
                self.remember_secondary_subtitle(selection);
            }
            Action::SubDelay(delta) => self.sub_delay(delta),
            Action::SubScale(delta) => self.sub_scale(delta),
            Action::SubMove(direction) => self.sub_move(direction),
            Action::SubReset => self.sub_reset(),
            Action::SubSeek(direction) => {
                if self.has_file() {
                    self.cmd(&["sub-seek", &direction.to_string()]);
                }
            }
            Action::SetSubCodepage(codepage) => self.set_sub_codepage(&codepage),
            Action::LoadSubtitleDialog => self.load_subtitle_dialog(),
            Action::LoadAudioDialog => self.load_audio_dialog(),

            Action::SetAspect(label, value) => self.set_aspect(&label, &value),
            Action::SetDisplayMode(mode) => self.set_display_mode(&mode),
            Action::Zoom(delta) => self.zoom(delta),
            Action::Pan(dx, dy) => self.pan(dx, dy),
            Action::ResetGeometry => self.reset_geometry(),
            Action::Rotate(degrees) => self.rotate(degrees),
            Action::ToggleFlip(name) => self.toggle_flip(name),
            Action::ImageAdjust(prop, direction) => self.image_adjust(prop, direction),
            Action::ImageReset => self.image_reset(),
            Action::ToggleDeinterlace => self.toggle_deinterlace(),
            Action::SetHwdec(enabled) => self.set_hwdec(enabled),

            Action::SetVolume(value) => self.set_volume(value),
            Action::VolumeStep(direction) => self.volume_step(direction),
            Action::ToggleMute => self.toggle_mute(),
            Action::AudioDelay(delta) => self.audio_delay(delta),
            Action::SetAudioDevice(name) => self.set_prop("audio-device", name.as_str()),
            Action::ToggleNormalize => self.set_normalize(!self.normalize),
            Action::SetNormalize(enabled) => self.set_normalize(enabled),
            Action::SetEqualizer(gains, enabled) => {
                self.settings.eq = gains;
                self.settings.eq_enabled = enabled;
                self.apply_eq();
            }
            Action::SetKeepAspect(keep) => self.set_prop("keepaspect", keep),
            Action::SetSubPos(value) => {
                self.set_prop("sub-pos", value);
            }
            Action::PlayUrl { url, loop_s } => {
                if loop_s > 0 {
                    self.set_prop("loop-playlist", loop_s);
                }
                self.pending.open_paths(vec![url], false);
            }

            Action::PlaylistRemove => {
                if self.playlist.remove_selected() {
                    self.update_title();
                }
            }
            Action::PlaylistRemoveMissing => {
                if self.playlist.remove_missing() {
                    self.update_title();
                }
            }
            Action::PlaylistRemoveDuplicates => {
                if self.playlist.remove_duplicates() {
                    self.update_title();
                }
            }
            Action::PlaylistClear => {
                self.playlist.clear();
                self.update_title();
            }
            Action::PlaylistSort(key) => {
                self.playlist.sort_by(key);
                self.update_title();
            }
            Action::PlaylistCopyPath(row) => {
                if let Some(path) = self.playlist.path(row) {
                    ctx.copy_text(path.to_string());
                    self.osd("Path copied to the clipboard");
                }
            }
            Action::PlaylistShowInFolder(row) => {
                if let Some(path) = self.playlist.path(row) {
                    open_folder(path);
                }
            }

            Action::Screenshot(mode, clipboard) => self.screenshot(mode, clipboard),
            Action::OpenScreenshotFolder => self.open_screenshot_folder(),

            Action::SetOnTop(mode) => self.set_on_top(&mode),
            Action::CycleOnTop => self.cycle_on_top(),
            Action::ToggleInfo => self.toggle_info(),
            Action::ToggleFullscreen => self.toggle_fullscreen(ctx),
            Action::ExitFullscreen => {
                if self.fullscreen {
                    self.toggle_fullscreen(ctx);
                }
            }
            Action::ToggleMaximize => self.toggle_maximize(ctx),
            Action::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Action::Close => {
                self.closing = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::TogglePlaylist => self.toggle_playlist(ctx),

            Action::ShowPreferences(tab) => {
                self.prefs.open(&self.settings);
                if let Some(tab) = tab {
                    self.prefs.select_tab(tab);
                }
            }
            Action::ShowControlPanel(tab) => {
                self.panel.open = true;
                if let Some(tab) = tab {
                    self.panel.select_tab(tab);
                }
                if let Some(engine) = self.engine.take() {
                    let settings = &self.settings;
                    self.panel.refresh(&engine, settings);
                    self.engine = Some(engine);
                }
            }
            Action::ShowMediaInfo => {
                let text = self.info_text(false);
                self.text_dialog
                    .set("Playback/System Info", &text, true, false);
            }
            Action::ShowAbout => {
                let e = self.engine();
                let mpv = e
                    .and_then(|e| e.get("mpv-version"))
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "?".into());
                let ffmpeg = e
                    .and_then(|e| e.get("ffmpeg-version"))
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "?".into());
                let text = format!(
                    "MacPlayer\n\nA native media player built on libmpv and egui.\n\nmpv:     {mpv}\nffmpeg:  {ffmpeg}\n\nRight-click the video for the full menu.\nSettings: ~/Library/Application Support/MacPlayer/settings.json"
                );
                self.text_dialog.set("About", &text, false, false);
            }
            Action::ShowTrackMenu(kind) => {
                if self.has_file() {
                    self.track_menu_kind = Some(kind);
                    let pos = ctx.input(|i| i.pointer.hover_pos()).unwrap_or_else(|| {
                        let size = ctx.content_rect().size();
                        pos2(size.x / 2.0, size.y / 2.0)
                    });
                    self.menu.open_at(pos);
                }
            }
            Action::ShowMainMenu => {
                let pos = ctx.input(|i| i.pointer.hover_pos()).unwrap_or_else(|| {
                    let size = ctx.content_rect().size();
                    pos2(size.x / 2.0, size.y / 2.0)
                });
                self.menu.open_at(pos);
                self.track_menu_kind = None;
            }
            Action::ApplySettings { hwdec_changed } => {
                if let Some((settings, changed)) = self.prefs.result.take() {
                    self.settings = settings;
                    self.apply_settings(changed || hwdec_changed);
                    self.settings.save();
                }
            }
            Action::Quit => {
                self.quit();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    // -- keymap -----------------------------------------------------------------------------------------

    fn keymap(&self, key: egui::Key, mods: egui::Modifiers) -> Option<(Action, bool)> {
        use egui::Key as K;
        let s = &self.settings;
        // On macOS the command key plays the role of Control.
        let ctrl = mods.command || mods.ctrl;
        let shift = mods.shift;
        let alt = mods.alt;
        let none = !ctrl && !shift && !alt;

        let seek_small = s.seek_small;
        let seek_medium = s.seek_medium;
        let seek_large = s.seek_large;
        let seek_huge = s.seek_huge;

        let action = match key {
            K::Space => (Action::TogglePause, false),
            K::Enter | K::F11 => (Action::ToggleFullscreen, false),
            K::Escape => (Action::ExitFullscreen, false),
            K::ArrowLeft if ctrl && alt => (Action::SeekRel(-seek_huge), true),
            K::ArrowRight if ctrl && alt => (Action::SeekRel(seek_huge), true),
            K::ArrowLeft if shift => (Action::SeekRel(-seek_large), true),
            K::ArrowRight if shift => (Action::SeekRel(seek_large), true),
            K::ArrowLeft if ctrl => (Action::SeekRel(-seek_medium), true),
            K::ArrowRight if ctrl => (Action::SeekRel(seek_medium), true),
            K::ArrowLeft => (Action::SeekRel(-seek_small), true),
            K::ArrowRight => (Action::SeekRel(seek_small), true),
            K::ArrowUp if alt => (Action::SubMove(-1), true),
            K::ArrowDown if alt => (Action::SubMove(1), true),
            K::ArrowUp => (Action::VolumeStep(1), true),
            K::ArrowDown => (Action::VolumeStep(-1), true),
            K::M => (Action::ToggleMute, false),
            K::PageUp if alt => (Action::SubScale(0.1), true),
            K::PageDown if alt => (Action::SubScale(-0.1), true),
            K::PageUp if shift => (Action::ChapterStep(-1), false),
            K::PageDown if shift => (Action::ChapterStep(1), false),
            K::PageUp => (Action::PrevFile, false),
            K::PageDown => (Action::NextFile, false),
            K::Backspace | K::Home => (Action::SeekAbs(0.0), false),
            K::F => (Action::FrameStep(1), true),
            K::D => (Action::FrameStep(-1), true),
            K::C if none => (Action::ChangeSpeed(0.1), true),
            K::X if none => (Action::ChangeSpeed(-0.1), true),
            K::Z if none => (Action::SetSpeed(1.0), false),
            K::G => (Action::OpenGoTo, false),
            K::B if none => (Action::AddBookmark, false),
            K::OpenBracket => (Action::AbSet('a'), false),
            K::CloseBracket => (Action::AbSet('b'), false),
            K::Backslash => (Action::AbClear, false),
            K::Comma if shift => (Action::AudioDelay(Some(-0.05)), true),
            K::Period if shift => (Action::AudioDelay(Some(0.05)), true),
            K::Slash if shift => (Action::AudioDelay(None), false),
            K::Comma => (Action::SubDelay(Some(-0.5)), true),
            K::Period => (Action::SubDelay(Some(0.5)), true),
            K::Slash => (Action::SubDelay(None), false),
            K::H if alt => (Action::ToggleSubVisibility, false),
            K::L if alt => (Action::CycleTrack(TrackKind::Sub), false),
            K::L => (Action::ShowTrackMenu(TrackKind::Sub), false),
            K::A if alt => (Action::CycleTrack(TrackKind::Audio), false),
            K::A if shift => (Action::CycleTrack(TrackKind::Audio), false),
            K::A => (Action::ShowTrackMenu(TrackKind::Audio), false),
            K::O if alt => (Action::LoadSubtitleDialog, false),
            K::O if ctrl => (Action::OpenFiles { append: false }, false),
            K::F3 => (Action::OpenFiles { append: false }, false),
            K::F2 => (Action::OpenFolder { append: false }, false),
            K::U if ctrl => (Action::OpenUrl, false),
            K::V if ctrl && shift => (Action::OpenClipboard, false),
            K::F4 if alt => (Action::Quit, false),
            K::F4 => (Action::Stop, false),
            K::F5 => (Action::ShowPreferences(None), false),
            K::F6 => (Action::TogglePlaylist, false),
            K::F7 => (Action::ShowControlPanel(None), false),
            K::F1 if ctrl => (Action::ShowMediaInfo, false),
            K::F1 => (Action::ShowAbout, false),
            K::Tab => (Action::ToggleInfo, false),
            K::E if ctrl && alt => (Action::Screenshot("subtitles", false), false),
            K::E if ctrl => (Action::Screenshot("video", false), false),
            K::C if ctrl && alt => (Action::Screenshot("subtitles", true), false),
            K::C if ctrl => (Action::Screenshot("video", true), false),
            K::T if ctrl => (Action::CycleOnTop, false),
            K::Num1 if none => (Action::SetWindowScale(50), false),
            K::Num2 if none => (Action::SetWindowScale(100), false),
            K::Num3 if none => (Action::SetWindowScale(150), false),
            K::Num4 if none => (Action::SetWindowScale(200), false),
            K::W if none => (Action::ImageAdjust("brightness", -1), true),
            K::E if none => (Action::ImageAdjust("brightness", 1), true),
            K::R if none => (Action::ImageAdjust("contrast", -1), true),
            K::T if none => (Action::ImageAdjust("contrast", 1), true),
            K::Y if none => (Action::ImageAdjust("saturation", -1), true),
            K::U if none => (Action::ImageAdjust("saturation", 1), true),
            K::I if none => (Action::ImageAdjust("hue", -1), true),
            K::O if none => (Action::ImageAdjust("hue", 1), true),
            K::Q if none => (Action::ImageReset, false),
            K::K if alt => (Action::Rotate(None), false),
            K::Z if ctrl => (Action::ToggleFlip("hflip"), false),
            K::V if ctrl => (Action::ToggleFlip("vflip"), false),
            K::Q if ctrl => (Action::Quit, false),
            _ => return None,
        };
        Some(action)
    }

    // -- main frame --------------------------------------------------------------------------------------

    fn frame(&mut self, ctx: &Context, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.now = self.started_at.elapsed().as_secs_f64();
        self.osd.tick();

        if let Some(error) = &self.startup_error {
            ui.label(error.clone());
            return;
        }

        // Pull mpv events.
        let messages: Vec<EngineMsg> = self.engine().map(|e| e.poll()).unwrap_or_default();
        for message in messages {
            self.on_engine_event(message);
        }
        while let Ok(paths) = self.ipc_rx.try_recv() {
            if !paths.is_empty() {
                self.pending.open_paths(paths, false);
            }
        }

        // Control panel refresh timer.
        if self.panel.open && self.now >= self.panel.next_refresh {
            self.panel.next_refresh = self.now + 0.4;
            if let Some(engine) = self.engine.take() {
                let settings = &self.settings;
                self.panel.refresh(&engine, settings);
                self.engine = Some(engine);
            }
        }

        // Periodic autosave + info + cursor.
        if self.now >= self.next_save {
            self.next_save = self.now + 10.0;
            self.autosave();
        }
        if self.info_visible && self.now >= self.next_info {
            self.next_info = self.now + 0.4;
            let text = self.info_text(true);
            self.osd.info = text;
        }
        self.update_cursor(ctx);

        // Deferred autofit.
        if self.next_autofit > 0.0 && self.now >= self.next_autofit {
            self.next_autofit = 0.0;
            if self.settings.autofit {
                self.fit_window(None, ctx);
            }
        }

        self.handle_keyboard(ctx);
        self.handle_dropped_files(ctx);

        let window = ctx.content_rect();
        let layout = self.layout(window);
        self.remember_window_geometry(ctx);

        self.paint_chrome(ctx, ui, &layout);
        self.paint_video(ui, &layout);

        self.handle_menus(ctx, ui);
        self.handle_dialogs(ctx);

        // Drain the action queue built this frame.
        let pending = std::mem::take(&mut self.pending);
        for (paths, append) in pending.open {
            self.open_paths(&paths, append);
        }
        for action in pending.actions {
            self.dispatch(action, ctx);
        }
        if pending.quit {
            self.quit();
        }

        // Keep the window level in sync.
        let want_level = if self.on_top && !self.fullscreen {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(want_level));

        let title = if self.title_text.is_empty() {
            "MacPlayer".to_string()
        } else {
            format!("{} - MacPlayer", self.title_text)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        self.gl_ready = true;
        if !self.pending_open.is_empty() {
            let pending_open = std::mem::take(&mut self.pending_open);
            for (paths, append) in pending_open {
                self.open_paths(&paths, append);
            }
        }

        let interval = if self.is_playing() { 16 } else { 80 };
        ctx.request_repaint_after(Duration::from_millis(interval));
    }

    fn handle_keyboard(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            {
                if !pressed {
                    continue;
                }
                let mapped = self
                    .keymap(key, modifiers)
                    .or_else(|| physical_key.and_then(|physical| self.keymap(physical, modifiers)));
                if let Some((action, allows_repeat)) = mapped {
                    if repeat && !allows_repeat {
                        continue;
                    }
                    self.pending.push(action);
                }
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let hovered: Vec<String> = ctx.input(|i| {
            i.raw
                .hovered_files
                .iter()
                .filter_map(|f| f.path.as_ref().map(|p| p.to_string_lossy().to_string()))
                .collect()
        });
        if hovered != self.hovered_files {
            self.hovered_files = hovered;
            if self.hovered_files.is_empty() {
                self.osd.center.clear();
            } else if self.hovered_files.iter().all(|p| is_sub(p)) {
                self.osd.center = "Drop to load subtitle".into();
            } else if self.drop_appends(ctx) {
                self.osd.center = "Drop to add to playlist".into();
            } else {
                self.osd.center = "Drop to play".into();
            }
        }
        let dropped: Vec<String> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_string_lossy().to_string())
                .collect()
        });
        if !dropped.is_empty() {
            self.osd.center.clear();
            let all_sub = dropped.iter().all(|p| is_sub(p));
            if all_sub && self.has_file() {
                self.add_subtitles(&dropped);
            } else {
                self.pending.open_paths(dropped, self.drop_appends(ctx));
            }
        }
    }

    fn drop_appends(&self, ctx: &Context) -> bool {
        let on_playlist = ctx.pointer_hover_pos().is_some_and(|pos| {
            self.layout(ctx.content_rect())
                .playlist
                .is_some_and(|rect| rect.contains(pos))
        });
        on_playlist || ctx.input(|input| input.modifiers.command || input.modifiers.ctrl)
    }

    fn update_cursor(&mut self, ctx: &Context) {
        let (hover, delta, pointer_down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.delta(),
                i.pointer.any_down(),
            )
        });
        let window = ctx.content_rect();
        let top_zone = Rect::from_min_max(
            window.min,
            pos2(
                window.right(),
                (window.top() + TITLE_H).min(window.bottom()),
            ),
        );
        let bottom_zone = Rect::from_min_max(
            pos2(
                window.left(),
                (window.bottom() - SEEK_H - CTRL_H).max(window.top()),
            ),
            window.max,
        );
        let top_hovered = self.fullscreen && hover.is_some_and(|pos| top_zone.contains(pos));
        let bottom_hovered = self.fullscreen && hover.is_some_and(|pos| bottom_zone.contains(pos));
        if self.fullscreen {
            if top_hovered {
                self.fs_top = true;
                self.fs_top_deadline = self.now + FS_EDGE_TIMEOUT;
            } else if self.fs_top && self.now >= self.fs_top_deadline {
                self.fs_top = false;
            }
            if bottom_hovered {
                self.fs_bottom = true;
                self.fs_bottom_deadline = self.now + FS_EDGE_TIMEOUT;
            } else if self.fs_bottom && self.now >= self.fs_bottom_deadline {
                self.fs_bottom = false;
            }
        } else {
            self.fs_top = false;
            self.fs_bottom = false;
            self.fs_top_deadline = 0.0;
            self.fs_bottom_deadline = 0.0;
        }
        let video_rect = self.layout(window).video;
        self.mouse_in_video = hover.is_some_and(|pos| video_rect.contains(pos));
        let moved = delta != Vec2::ZERO;
        if self.mouse_in_video {
            if moved || self.last_pointer_move.is_none() {
                self.cursor_hidden = false;
                self.last_pointer_move = Some(Instant::now());
                self.next_cursor = self.now + 1.5;
            }
        } else {
            self.cursor_hidden = false;
        }
        if self.is_playing()
            && self.mouse_in_video
            && self.has_video()
            && self.now >= self.next_cursor
        {
            if !pointer_down && !ctx.any_popup_open() {
                self.cursor_hidden = true;
            }
        }
        if !self.is_playing() {
            self.cursor_hidden = false;
        }
        apply_cursor(ctx, self.cursor_hidden);
    }

    fn paint_chrome(&mut self, ctx: &Context, ui: &mut Ui, layout: &Layout) {
        let window = ctx.content_rect();

        // Frame border.
        let painter = ui.painter();
        painter.rect_filled(window, 0.0, LINE);

        // Title bar.
        if let Some(rect) = layout.title {
            let pin = self.settings.on_top != "never";
            let icons = [
                "close",
                "minimize",
                if self.maximized {
                    "restore"
                } else {
                    "maximize"
                },
                "pin",
                if self.fullscreen {
                    "fullscreen_exit"
                } else {
                    "fullscreen"
                },
            ];
            let checked = [false, false, false, pin, false];
            let bar = title_bar(
                ui,
                rect,
                &self.title_text,
                "MacPlayer",
                icons,
                checked,
                &mut self.press_origin,
            );
            if bar.menu_click.any() {
                self.menu.open_at(rect.left_bottom());
                self.track_menu_kind = None;
            }
            if bar.buttons[0].primary {
                self.quit();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if bar.buttons[1].primary {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            if bar.buttons[2].primary {
                self.pending.push(Action::ToggleMaximize);
            }
            if bar.buttons[3].primary {
                let mode = if pin { "never" } else { "always" };
                self.pending.push(Action::SetOnTop(mode.into()));
            }
            if bar.buttons[4].primary {
                self.pending.push(Action::ToggleFullscreen);
            }
            if bar.double {
                self.pending.push(Action::ToggleFullscreen);
            }
            if bar.drag_started {
                if !self.fullscreen && !self.maximized {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            }
        }

        // Seek bar + volume.
        if let (Some(seek_rect), Some(vol_rect)) = (layout.seek, layout.volume) {
            let now = Instant::now();
            let out = seek_bar(ui, seek_rect, Id::new("seek"), &mut self.seek, now);
            if let Some((seconds, exact)) = out.seek {
                self.pending.push(Action::SeekBar(seconds, exact));
            }
            if let Some(seconds) = out.hover {
                if seconds >= 0.0 {
                    let mut text = fmt_time(seconds);
                    let chapters = self.state.get("chapter-list").cloned();
                    if let Some(chapters) = chapters {
                        let mut current = None;
                        for chapter in chapters.as_array() {
                            if chapter.get("time").as_f64().unwrap_or(0.0) <= seconds {
                                current = Some(chapter);
                            }
                        }
                        if let Some(title) =
                            current.and_then(|c| c.get("title").as_str().map(str::to_string))
                        {
                            text.push_str(&format!("  {title}"));
                        }
                    }
                    let font_id = skin::font(11.0, false);
                    let painter = ui.painter();
                    let w = skin::text_width(painter, &text, &font_id) + 14.0;
                    let h = 20.0;
                    let x = (seek_rect.center().x - w / 2.0)
                        .clamp(window.left() + 2.0, window.right() - w - 2.0);
                    let bubble = Rect::from_min_size(pos2(x, seek_rect.top() - h), vec2(w, h));
                    let bp = ui.painter_at(bubble);
                    bp.rect_filled(bubble, 0.0, Color32::BLACK);
                    bp.rect_filled(bubble.shrink(1.0), 0.0, DIALOG_BG);
                    bp.text(
                        bubble.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        font_id,
                        TEXT_BRIGHT,
                    );
                }
            }
            if out.wheel != 0 {
                let step = self.settings.seek_small;
                self.pending.push(Action::SeekRel(out.wheel as f64 * step));
            }
            let vol_out = volume_bar(ui, vol_rect, Id::new("volume"), &mut self.volume);
            if let Some(value) = vol_out.volume {
                self.pending.push(Action::SetVolume(value));
            }
            if vol_out.mute {
                self.pending.push(Action::ToggleMute);
            }
            if vol_out.wheel != 0 {
                let step = self.settings.volume_step as i32;
                self.pending.push(Action::VolumeStep(vol_out.wheel * step));
            }
        }

        // Control bar.
        if let Some(rect) = layout.ctrl {
            self.paint_control_bar(ui, rect);
        }

        // Playlist.
        if let Some(rect) = layout.playlist {
            let ctx2 = ctx.clone();
            let out = self.playlist.ui(ui, rect, &ctx2);
            self.apply_playlist_out(out, ctx);
        }
    }

    fn paint_control_bar(&mut self, ui: &mut Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, BG);

        let playing = self.is_playing();
        let button_w = (rect.width() / 7.0).min(BTN_W).max(0.0);
        let button_area = button_w * 7.0;

        // Buttons: menu, play, prev, next, open, gear, list.
        let buttons: [(&str, f32); 7] = [
            ("menu", 0.0),
            ("prev", BTN_W as f32),
            ("play", BTN_W as f32 * 2.0),
            ("next", BTN_W as f32 * 3.0),
            ("eject", BTN_W as f32 * 4.0),
            ("gear", BTN_W as f32 * 5.0),
            ("menu", BTN_W as f32 * 6.0),
        ];
        let mut clicks = [Click::default(); 7];
        for (index, (glyph, x)) in buttons.iter().enumerate() {
            let glyph = if *glyph == "menu" && index == 6 {
                "menu"
            } else {
                glyph
            };
            let r = Rect::from_min_size(
                pos2(rect.left() + *x / BTN_W * button_w, rect.top()),
                vec2(button_w, rect.height()),
            );
            let mut btn = IconBtn::new(glyph, button_w, rect.height()).flat(true);
            if index == 0 {
                btn = btn.tip("Menu");
            }
            if index == 2 {
                btn = btn.tip(if playing { "Pause" } else { "Play" });
            }
            if index == 5 {
                btn = btn.tip("Control Panel");
            }
            if index == 6 {
                btn = btn.tip("Playlist");
            }
            let clicked = btn.ui(ui, ui.id().with(("ctrl-btn", index)), r);
            clicks[index] = clicked;
            if index == 2 {
                skin::draw_icon(
                    &painter,
                    if playing { "pause" } else { "play" },
                    r.center().x.round(),
                    r.center().y.round(),
                    if clicked.hovered { ICON_HOVER } else { ICON },
                    BG,
                );
            }
        }
        if clicks[0].any() {
            self.menu.open_at(rect.left_top());
            self.track_menu_kind = None;
        }
        if clicks[1].primary {
            self.pending.push(Action::PrevFile);
        }
        if clicks[2].primary {
            self.pending.push(Action::TogglePause);
        }
        if clicks[3].primary {
            self.pending.push(Action::NextFile);
        }
        if clicks[4].primary {
            self.pending.push(Action::OpenFiles { append: false });
        }
        if clicks[4].secondary {
            self.pending.push(Action::OpenFolder { append: false });
        }
        if clicks[5].primary {
            self.pending.push(Action::ShowControlPanel(None));
        }
        if clicks[6].primary {
            self.pending.push(Action::TogglePlaylist);
        }

        // Time label on the right of the buttons.
        let (left, right) = self.time_parts();
        let remaining = (rect.width() - button_area).max(0.0);
        let time_w = remaining.min(150.0);
        let badge_w = (remaining - time_w).min(180.0);
        let time_x = rect.right() - badge_w - time_w;
        if time_w > 0.0 {
            let time_rect =
                Rect::from_min_size(pos2(time_x, rect.top()), vec2(time_w, rect.height()));
            if time_label(ui, time_rect, Id::new("time"), &left, &right) {
                self.pending.push(Action::ToggleInfo);
            }
        }
        if badge_w > 0.0 {
            let badge_rect = Rect::from_min_size(
                pos2(rect.right() - badge_w, rect.top()),
                vec2(badge_w, rect.height()),
            );
            info_badges(ui, badge_rect, &self.badges);
        }
    }

    fn time_parts(&self) -> (String, String) {
        let pos = self.position();
        let dur = self.duration();
        let (elapsed, total) = fmt_pair(pos, dur);
        if self.show_remaining && dur > 0.0 {
            let remaining = fmt_time((dur - pos).max(0.0));
            (format!("-{remaining}"), total)
        } else {
            (elapsed, total)
        }
    }

    fn paint_video(&mut self, ui: &mut Ui, layout: &Layout) {
        let rect = layout.video;
        let idle = self
            .state
            .get("idle-active")
            .map(|v| v.as_bool())
            .unwrap_or(true);
        let audio_only = self.file_ready && !self.has_video();
        let show_logo = idle || audio_only;

        let renderer = self.renderer.clone();
        let callback = egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let viewport = info.viewport_in_pixels();
            let rect_px = RectPx {
                x: viewport.left_px,
                y: viewport.from_bottom_px,
                width: viewport.width_px,
                height: viewport.height_px,
            };
            let Some(renderer) = &renderer else { return };
            let Ok(mut renderer) = renderer.lock() else {
                return;
            };
            if let Err(error) = unsafe { renderer.ensure_context(gl) } {
                eprintln!("render context: {error}");
                return;
            }
            if let Err(error) = unsafe { renderer.render(gl, rect_px) } {
                eprintln!("render: {error}");
            }
        });
        ui.painter().add(egui::epaint::PaintCallback {
            rect,
            callback: Arc::new(callback),
        });

        if show_logo {
            let title = self.logo_title();
            widgets::paint_logo(ui, rect, &title);
        }

        // OSD on top of the video.
        self.osd.paint(ui, rect);

        // Click / double-click on the video area.
        let response = ui.interact(rect, Id::new("video"), Sense::click());
        if response.clicked() {
            self.pending.push(Action::TogglePause);
        }
        if response.double_clicked() {
            self.pending.push(Action::ToggleFullscreen);
        }
        if response.hovered() {
            self.mouse_in_video = true;
        }
    }

    fn handle_menus(&mut self, ctx: &Context, ui: &mut Ui) {
        if !self.menu.is_open() {
            return;
        }
        let entries = match self.track_menu_kind {
            Some(kind) => {
                let tracks = self.tracks().to_vec();
                menus::track_menu(kind, &tracks, self.has_file())
            }
            None => {
                let engine = self.engine();
                let state = &self.state;
                let tracks: Vec<Value> = engine
                    .and_then(|e| e.get("track-list"))
                    .map(|v| v.as_array().to_vec())
                    .unwrap_or_default();
                let chapters: Vec<Value> = state
                    .get("chapter-list")
                    .map(|v| v.as_array().to_vec())
                    .unwrap_or_default();
                let audio_devices: Vec<Value> = engine
                    .and_then(|e| e.get("audio-device-list"))
                    .map(|v| v.as_array().to_vec())
                    .unwrap_or_default();
                let c = MenuCtx {
                    has_file: self.has_file(),
                    playlist_count: self.playlist.count(),
                    playlist_current: self.playlist.current_row(),
                    recent: &self.settings.recent,
                    bookmarks: &self.settings.bookmarks,
                    repeat: &self.settings.repeat_mode,
                    shuffle: self.settings.shuffle,
                    resume: self.settings.resume,
                    autofit: self.settings.autofit,
                    on_top: &self.settings.on_top,
                    hwdec: self.settings.hwdec,
                    sub_codepage: &self.settings.sub_codepage,
                    sub_visible: self.settings.sub_visible,
                    info_visible: self.info_visible,
                    fullscreen: self.fullscreen,
                    aspect_name: &self.aspect_name,
                    display_mode: &self.display_mode,
                    flips: self.flips,
                    normalize: self.normalize,
                    speed: state.get("speed").and_then(|v| v.as_f64()).unwrap_or(1.0),
                    mute: state.get("mute").map(|v| v.as_bool()).unwrap_or(false),
                    tracks: &tracks,
                    secondary_sid: state.get("secondary-sid").and_then(|v| v.as_i64()),
                    chapters: &chapters,
                    current_chapter: state.get("chapter").and_then(|v| v.as_i64()).unwrap_or(-1),
                    ab: (self.seek.ab.0, self.seek.ab.1),
                    audio_devices: &audio_devices,
                    audio_device: state
                        .get("audio-device")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    hwdec_current: state
                        .get("hwdec-current")
                        .and_then(|v| v.as_str())
                        .unwrap_or("no"),
                    seek_small: self.settings.seek_small,
                    seek_medium: self.settings.seek_medium,
                    seek_large: self.settings.seek_large,
                    seek_huge: self.settings.seek_huge,
                    deinterlace: state
                        .get("deinterlace")
                        .map(|v| v.as_bool())
                        .unwrap_or(false),
                    rotation: state
                        .get("video-rotate")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0),
                };
                menus::main_menu(&c)
            }
        };
        let mut menu = std::mem::take(&mut self.menu);
        let action = menus::show(ui, &mut menu, "main-menu", &entries);
        self.menu = menu;
        if let Some(action) = action {
            self.menu.close();
            self.track_menu_kind = None;
            self.pending.push(action);
        } else if !self.menu.is_open() {
            self.track_menu_kind = None;
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.menu.close();
            self.track_menu_kind = None;
        }
    }

    fn handle_dialogs(&mut self, ctx: &Context) {
        let duration = self.duration();
        let mut actions = Vec::new();
        self.goto_dialog.show(ctx, duration, &mut actions);
        self.url_dialog.show(ctx, &mut actions);
        self.prefs.show(ctx, &mut actions);
        {
            let mut panel_actions = Vec::new();
            let settings = &mut self.settings;
            self.panel.show(ctx, settings, &mut panel_actions);
            actions.extend(panel_actions);
        }
        self.text_dialog.show(ctx);
        for action in actions {
            self.pending.push(action);
        }
    }

    fn apply_playlist_out(&mut self, out: PlaylistOut, ctx: &Context) {
        if let Some(row) = out.play {
            self.pending.push(Action::PlayRow(row));
        }
        if out.add_files {
            self.pending.push(Action::OpenFiles { append: true });
        }
        if out.add_folder {
            self.pending.push(Action::OpenFolder { append: true });
        }
        if out.add_url {
            self.pending.push(Action::OpenUrl);
        }
        if out.close {
            self.toggle_playlist(ctx);
        }
        if out.repeat {
            self.cycle_repeat();
        }
        if out.shuffle {
            self.pending.push(Action::ToggleShuffle);
        }
        if out.changed {
            self.update_title();
        }
    }
}

impl eframe::App for Player {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame(&ctx, ui, frame);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.closing = true;
        self.remember_position();
        self.settings.first_run = false;
        self.settings.save();
        if let (Some(renderer), Some(gl)) = (self.renderer.take(), gl) {
            if let Ok(mut renderer) = renderer.lock() {
                unsafe { renderer.shutdown(gl) };
            }
        }
        if let Some(mut engine) = self.engine.take() {
            engine.shutdown();
        }
    }
}

// -- layout -------------------------------------------------------------------------------

struct Layout {
    video: Rect,
    title: Option<Rect>,
    seek: Option<Rect>,
    volume: Option<Rect>,
    ctrl: Option<Rect>,
    playlist: Option<Rect>,
    playlist_right: bool,
    has_top: bool,
    has_bottom: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            video: Rect::NOTHING,
            title: None,
            seek: None,
            volume: None,
            ctrl: None,
            playlist: None,
            playlist_right: false,
            has_top: false,
            has_bottom: false,
        }
    }
}

// -- small helpers ---------------------------------------------------------------------------

fn screen_size(ctx: &Context) -> (f64, f64) {
    ctx.input(|input| {
        input
            .viewport()
            .monitor_size
            .map(|size| (size.x as f64, size.y as f64))
            .filter(|(width, height)| {
                width.is_finite() && height.is_finite() && *width > 1.0 && *height > 1.0
            })
            .unwrap_or((1920.0, 1080.0))
    })
}

fn window_size(ctx: &Context) -> (f32, f32) {
    let size = ctx.content_rect().size();
    (size.x, size.y)
}

fn playlist_resize_delta(allocated_width: Option<f32>, configured_width: f32) -> f32 {
    allocated_width.unwrap_or(configured_width) + 1.0
}

fn home_dir() -> Option<String> {
    std::env::var("HOME").ok()
}

fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

fn open_folder(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

fn media_extensions() -> Vec<String> {
    [
        "mkv", "mp4", "avi", "mov", "webm", "flv", "wmv", "m4v", "mpg", "mpeg", "ts", "m2ts",
        "ogv", "3gp",
    ]
    .iter()
    .map(|e| e.to_string())
    .collect()
}

fn sub_extensions() -> Vec<String> {
    ["srt", "ass", "ssa", "sub", "vtt", "idx", "sup"]
        .iter()
        .map(|e| e.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::playlist_resize_delta;

    #[test]
    fn playlist_resize_uses_allocated_width_when_panel_was_clamped() {
        assert_eq!(playlist_resize_delta(Some(325.0), 400.0), 326.0);
        assert_eq!(playlist_resize_delta(None, 280.0), 281.0);
    }
}
