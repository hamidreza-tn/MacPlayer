//! The player window: chrome layout, mpv property handling, keymap and every user action.
//!
//! The MacPlayer application state, restructured around egui's immediate-mode frame.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, Context, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::dialogs::{
    ControlPanel, GoToDialog, GoToMode, HistoryDialog, PreferencesDialog, ResumeChoice,
    ResumeDialog, SettingsPopover, TextDialog, UrlDialog,
};
use crate::engine::{Engine, EngineMsg, FileTrackState, TrackSelection};
use crate::ffi;
use crate::i18n::{self, Lang};
use crate::menus::{self, Action, MenuCtx, MenuState, TrackKind, TrackSel};
use crate::playlist::{MediaStatus, Playlist, PlaylistOut, progress_fraction};
use crate::render::{RectPx, Renderer};
use crate::services::{PlatformServices, Services};
use crate::settings::Settings;
use crate::skin::{self, *};
use crate::thumbs::Thumbnails;
use crate::util::{
    expand_paths, ext, fmt_time, human_size, is_sub, is_url, read_playlist_file, similar_files,
    title_of, write_m3u,
};
use crate::value::Value;
use crate::widgets::{
    IconBtn, Osd, OsdTone, SeekUi, VolumeUi, apply_cursor, fmt_pair, seek_bar, volume_bar,
};

/// How long the floating chrome stays visible without pointer activity.
const CHROME_HIDE_SECS: f64 = 2.2;
/// How long the big play/pause flash stays on the video.
const FLASH_SECS: f64 = 0.55;
/// Chrome the floating transport cluster needs to stay clickable to the left
/// of the drawer (back+play+fwd + fullscreen, from the compaction test).
const MIN_CHROME_W: f32 = 172.0;

/// Widest playlist drawer.
const DRAWER_MAX_W: f32 = 420.0;

/// Drawer width cap for a given window width: never let the drawer swallow the
/// transport cluster, whatever `playlist_width` says.
fn max_drawer_width(window_width: f32) -> f32 {
    DRAWER_MAX_W.min(MIN_PLAYLIST_W.max(window_width - MIN_CHROME_W))
}
/// Window in which a second stage click upgrades to a double-click, so a plain
/// click-to-pause never also triggers full screen.
const VIDEO_DOUBLE_CLICK_SECS: f64 = 0.30;
const PLAYLIST_EXT: &[&str] = crate::util::PLAYLIST_EXT;
const VIDEO_EXT: &[&str] = crate::util::VIDEO_EXT;
const AUDIO_EXT: &[&str] = crate::util::AUDIO_EXT;

static SAVING: AtomicBool = AtomicBool::new(false);
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: Mutex<()> = Mutex::new(());

/// A single thing the UI wants the player to do. Chrome, menus and dialogs all push into
/// one queue; it is drained once per frame, after painting.
#[derive(Debug, Clone, Default)]
struct Pending {
    actions: Vec<Action>,
    /// (paths, append)
    open: Vec<(Vec<String>, bool)>,
}

impl Pending {
    fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, action: Action) {
        self.actions.push(action);
    }

    fn open_paths(&mut self, paths: Vec<String>, append: bool) {
        self.open.push((paths, append));
    }
}

/// Rows shown at once in the command palette.
const PALETTE_MAX_RESULTS: usize = 12;

/// The Ctrl+K command palette: a flat, filterable view over the menu tree.
#[derive(Debug, Default)]
struct CommandPalette {
    open: bool,
    query: String,
    selected: usize,
    focused: bool,
}

/// mpv state arrays the menu model reads; kept alive beside the borrowed [`MenuCtx`].
struct MenuSnapshot {
    tracks: Vec<Value>,
    chapters: Vec<Value>,
    audio_devices: Vec<Value>,
}

impl CommandPalette {
    fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focused = true;
    }

    fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
    }
}

pub struct Player {
    // `renderer` is declared before `engine` on purpose: fields drop in
    // declaration order and mpv requires mpv_render_context_free to run
    // before mpv_terminate_destroy (mpv render.h: UB otherwise). The normal
    // path frees both explicitly in `on_exit`; this order guards unwind drops.
    renderer: Option<Arc<Mutex<Renderer>>>,
    engine: Option<Engine>,
    startup_error: Option<String>,
    ipc_rx: Receiver<crate::IpcMessage>,
    /// Request/result channels for the off-thread folder expansion worker.
    expander: Option<Sender<(u64, Vec<String>, bool)>>,
    expanded_rx: Receiver<(u64, Vec<String>, bool)>,
    services: Box<dyn Services>,

    settings: Settings,
    settings_error: Option<String>,
    state: HashMap<String, Value>,

    playlist: Playlist,
    current_path: Option<String>,
    file_ready: bool,
    last_pos: f64,
    seek_target: (f64, f64),
    /// True while a paused seek is still awaiting its time-pos report.
    seek_target_pending: bool,
    fail_count: usize,
    autofit_pending: bool,
    resumed_at: Option<f64>,
    pending_subs: Vec<String>,

    osd: Osd,
    seek: SeekUi,
    volume: VolumeUi,

    title_text: String,
    aspect_name: String,
    /// mpv value of the selected aspect override ("-1" = default).
    aspect_value: String,
    display_mode: String,
    flips: (bool, bool),
    normalize: bool,
    eq_active: bool,
    info_visible: bool,
    show_remaining: bool,

    menu: MenuState,
    track_menu_kind: Option<TrackKind>,
    goto_dialog: GoToDialog,
    resume_dialog: ResumeDialog,
    url_dialog: UrlDialog,
    text_dialog: TextDialog,
    prefs: PreferencesDialog,
    panel: ControlPanel,
    settings_pop: SettingsPopover,
    palette: CommandPalette,

    pending: Pending,

    // -- window state -----------------------------------------------------------------------------
    playlist_visible: bool,
    /// Animated 0..1 drawer slide (1 = fully open); egui eases it every frame.
    drawer_slide: f32,
    fullscreen: bool,
    maximized: bool,
    on_top: bool,
    state_changed: f64,

    // -- floating chrome ---------------------------------------------------------------------------
    chrome_visible: bool,
    chrome_hide_at: f64,
    last_pointer: Option<Pos2>,
    flash_glyph: String,
    flash_until: f64,
    video_click_at: Option<f64>,
    settings_anchor: Rect,
    /// Actual current viewport height (fullscreen-aware), for subtitle margins.
    viewport_h: f32,

    // -- drag and drop -------------------------------------------------------------------------------
    hovered_files: Vec<String>,

    gl_ready: bool,
    pending_open: Vec<(Vec<String>, bool)>,
    open_generation: u64,
    /// Outstanding expander request: (generation, started_at, osd shown).
    expander_pending: Option<(u64, f64, bool)>,
    // -- Now Playing (Control Center) -------------------------------------
    now_playing: Option<crate::nowplaying::NowPlaying>,
    now_playing_rx: Option<std::sync::mpsc::Receiver<crate::nowplaying::NowPlayingCommand>>,
    now_playing_tx: Option<std::sync::mpsc::Sender<crate::nowplaying::NowPlayingCommand>>,
    now_playing_last: f64,
    /// Set once the Now Playing card has been wiped for the idle screen.
    now_playing_cleared: bool,
    // -- update checker ----------------------------------------------------
    update_rx: Option<std::sync::mpsc::Receiver<String>>,
    update_available: Option<String>,
    closing: bool,
    /// Last window title actually sent to the viewport; avoids re-sending it every frame.
    applied_title: String,
    /// Last always-on-top state actually sent; `None` until the first frame.
    applied_on_top: Option<bool>,
    next_save: f64,
    next_autofit: f64,
    next_info: f64,
    now: f64,
    started_at: Instant,
    /// Set once the first frame verified the restored window is on-screen.
    position_checked: bool,

    // -- new feature state -----------------------------------------------------------------------
    history_dialog: HistoryDialog,
    thumbs: Thumbnails,
    /// Absolute `now` deadline after which playback pauses; None = off.
    sleep_deadline: Option<f64>,
    /// Pause once the playlist reaches its end.
    sleep_after_playlist: bool,
    mini_player: bool,
    /// Window size before entering the mini player.
    mini_prev_size: Option<Vec2>,
    mini_prev_fullscreen: bool,
    mini_prev_maximized: bool,
    /// (deband, denoise, sharpen) toggle state for the video filter menu.
    video_filters: (bool, bool, bool),
    /// True while the lavfi visualizer filter is installed.
    viz_active: bool,
    /// Set at load for files with no real video track (visualizer/autofit).
    file_is_audio_only: bool,
    /// Pinch gesture latch: one gesture = one fullscreen toggle.
    pinch_latched: bool,
    /// Accumulated pinch scale for the current gesture (1.0 = neutral).
    pinch_scale: f32,
    /// Time of the last magnify event, to detect gesture end.
    pinch_last_at: f64,
    /// Leftover horizontal wheel distance for two-finger seeking.
    video_scroll_acc: f32,
    /// Leftover vertical ctrl/cmd wheel distance for video zooming.
    zoom_scroll_acc: f32,
}

impl Player {
    pub fn new(
        ctx: Context,
        args: Vec<String>,
        ipc_rx: Receiver<crate::IpcMessage>,
        settings: Settings,
        verbose: bool,
        fullscreen: bool,
    ) -> Self {
        // Folder expansion worker: one persistent thread turns open requests
        // into fully expanded path lists without ever blocking the UI.
        let (expander_tx, expander_rx) = channel::<(u64, Vec<String>, bool)>();
        let (expanded_tx, expanded_rx) = channel::<(u64, Vec<String>, bool)>();
        let expander_worker = std::thread::Builder::new()
            .name("open-expander".into())
            .spawn(move || expand_worker(expander_rx, expanded_tx))
            .ok();
        // Now Playing remote commands arrive outside frames; the UI drains
        // the channel and maps them onto the normal action queue.
        let (now_playing_tx, now_playing_rx) = crate::nowplaying::channel();
        // Startup update check (only when the feature is on).
        let (update_tx, update_rx) = channel::<String>();
        if settings.update_check {
            std::thread::Builder::new()
                .name("update-check".into())
                .spawn(move || {
                    if let crate::update::UpdateStatus::Available(tag) =
                        crate::update::check_for_update()
                    {
                        let _ = update_tx.send(tag);
                    }
                })
                .ok();
        }
        // Appearance globals must be set before the first style/font application.
        skin::set_theme(Theme::from_str(&settings.theme));
        Player::apply_accent_for(&settings);
        i18n::set_lang(Lang::from_str(&settings.language));
        skin::apply(&ctx);
        skin::apply_fonts(&ctx, i18n::rtl());
        let playlist_visible = settings.playlist_visible;
        let show_remaining = settings.show_remaining;
        let on_top = settings.on_top == "always";
        let (engine, renderer, startup_error) = match Engine::new(&settings, false, verbose) {
            Ok(engine) => {
                let renderer = Renderer::new(engine.api(), engine.handle(), ctx);
                (Some(engine), Some(Arc::new(Mutex::new(renderer))), None)
            }
            Err(error) => {
                crate::logging::write(&format!("engine startup failed: {error}"));
                (None, None, Some(error))
            }
        };
        // First run opens quietly: video + controls only, no drawer over the idle screen.
        let first_run = settings.first_run;
        let mut player = Self {
            engine,
            renderer,
            startup_error,
            ipc_rx,
            expander: expander_worker.is_some().then_some(expander_tx),
            expanded_rx,
            now_playing: if settings.now_playing {
                crate::nowplaying::NowPlaying::new(now_playing_tx.clone())
            } else {
                None
            },
            now_playing_rx: Some(now_playing_rx),
            now_playing_tx: Some(now_playing_tx),
            now_playing_last: 0.0,
            now_playing_cleared: false,
            update_rx: Some(update_rx),
            update_available: None,
            services: Box::new(PlatformServices),
            settings,
            settings_error: None,
            state: HashMap::new(),
            playlist: Playlist::default(),
            current_path: None,
            file_ready: false,
            last_pos: 0.0,
            seek_target: (0.0, 0.0),
            seek_target_pending: false,
            fail_count: 0,
            autofit_pending: false,
            resumed_at: None,
            pending_subs: Vec::new(),
            osd: Osd::default(),
            seek: SeekUi::default(),
            volume: VolumeUi::default(),
            title_text: String::new(),
            aspect_name: "Default".into(),
            aspect_value: "-1".into(),
            display_mode: "fit".into(),
            flips: (false, false),
            normalize: false,
            eq_active: false,
            info_visible: false,
            show_remaining,
            menu: MenuState::default(),
            track_menu_kind: None,
            goto_dialog: GoToDialog::default(),
            resume_dialog: ResumeDialog::default(),
            url_dialog: UrlDialog::default(),
            text_dialog: TextDialog::default(),
            prefs: PreferencesDialog::default(),
            panel: ControlPanel::default(),
            settings_pop: SettingsPopover::default(),
            palette: CommandPalette::default(),
            pending: Pending::new(),
            playlist_visible: playlist_visible && !first_run,
            drawer_slide: if playlist_visible && !first_run {
                1.0
            } else {
                0.0
            },
            fullscreen,
            maximized: false,
            on_top,
            state_changed: 0.0,
            chrome_visible: true,
            chrome_hide_at: f64::INFINITY,
            last_pointer: None,
            flash_glyph: String::new(),
            flash_until: 0.0,
            video_click_at: None,
            settings_anchor: Rect::NOTHING,
            viewport_h: 720.0,
            hovered_files: Vec::new(),
            gl_ready: false,
            pending_open: Vec::new(),
            open_generation: 0,
            expander_pending: None,
            closing: false,
            applied_title: String::new(),
            applied_on_top: None,
            next_save: 0.0,
            next_autofit: 0.0,
            next_info: 0.0,
            now: 0.0,
            started_at: Instant::now(),
            position_checked: false,
            history_dialog: HistoryDialog::default(),
            thumbs: Thumbnails::new(),
            sleep_deadline: None,
            sleep_after_playlist: false,
            mini_player: false,
            mini_prev_size: None,
            mini_prev_fullscreen: false,
            mini_prev_maximized: false,
            video_filters: (false, false, false),
            viz_active: false,
            file_is_audio_only: false,
            pinch_latched: false,
            pinch_scale: 1.0,
            pinch_last_at: 0.0,
            video_scroll_acc: 0.0,
            zoom_scroll_acc: 0.0,
        };
        if !args.is_empty() {
            player.pending.open_paths(args, false);
        } else if !player.settings.playlist_paths.is_empty() {
            let paths = player.settings.playlist_paths.clone();
            player.playlist.set_items(&paths);
            if let Some(current) = player.settings.playlist_current.as_deref()
                && let Some(row) = paths.iter().position(|path| path == current)
            {
                player.playlist.set_current(Some(row));
            }
        }
        player
            .playlist
            .set_modes(&player.settings.repeat_mode, player.settings.shuffle);
        player.apply_eq();
        // Push the engine-backed feature settings (loop, styling, sync, ...)
        // once so a fresh engine matches the saved preferences.
        player.apply_settings(false);
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

    fn save_settings(&mut self) {
        SAVE_GENERATION.fetch_add(1, Ordering::AcqRel);
        let guard = match SAVE_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Err(error) = self.settings.try_save() {
            self.settings_error = Some(error.clone());
            self.osd_error(
                format!("{}: {error}", i18n::tr("Could not save settings")),
                5000,
            );
        } else {
            self.settings_error = None;
        }
        drop(guard);
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
        self.save_settings();
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
        self.save_settings();
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

    /// Restore the per-file aspect ratio and speed remembered from last time.
    fn apply_saved_media_preference(&mut self) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let Some(preference) = self.settings.track_preference(&path) else {
            return;
        };
        if preference.aspect_label != "Default" && preference.aspect_value != "-1" {
            self.aspect_name = preference.aspect_label.clone();
            self.set_prop("video-aspect-override", preference.aspect_value.as_str());
        }
        if preference.speed > 0.0 {
            self.set_prop("speed", normalize_speed(preference.speed));
        }
    }

    /// Remember the current aspect/speed for the file being played.
    fn remember_media_preference(&mut self) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let speed = self.get_f64("speed");
        self.settings
            .set_media_preference(&path, &self.aspect_name, &self.aspect_value, speed);
        self.save_settings();
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
        self.osd.show_message(text, 1800, OsdTone::Normal);
    }

    fn osd_ms(&mut self, text: impl Into<String>, ms: u64) {
        self.osd.show_message(text, ms, OsdTone::Normal);
    }

    fn osd_error(&mut self, text: impl Into<String>, ms: u64) {
        self.osd.show_message(text, ms, OsdTone::Danger);
    }

    // -- layout --------------------------------------------------------------------------------------

    /// Window chrome beyond the video: borders, title bar, seek row, control bar and playlist.
    /// Where each chrome piece goes. The video always fills the window; every control
    /// floats over it (quiet-player design). `controls`/`top_actions` are always laid
    /// out — the fade in [`Self::paint_chrome`] decides how much of them shows.
    fn layout(&self, window: Rect) -> Layout {
        let mut l = Layout {
            video: window,
            controls: Some(Rect::from_min_max(
                pos2(window.left(), (window.bottom() - CTRL_H).max(window.top())),
                pos2(window.right(), window.bottom()),
            )),
            top_actions: Some(Rect::from_min_size(
                pos2(
                    (window.right() - 78.0).max(window.left()),
                    window.top() + 10.0,
                ),
                vec2(68.0, 32.0),
            )),
            ..Layout::default()
        };
        // The drawer slides over the right edge; `drawer_slide` eases 0..1 so both
        // the drawer and the floating controls animate together.
        if self.drawer_slide > 0.001 {
            let width = (self.settings.playlist_width as f32)
                .clamp(MIN_PLAYLIST_W, DRAWER_MAX_W)
                .min(max_drawer_width(window.width()));
            let left = (window.right() - width * self.drawer_slide).max(window.left());
            l.playlist = Some(Rect::from_min_max(
                pos2(left, window.top()),
                pos2(
                    window.right() + (1.0 - self.drawer_slide) * width,
                    window.bottom(),
                ),
            ));
        }
        // The drawer owns the right edge; the floating controls and top actions
        // shrink to the left of it so volume/fullscreen never end up under it.
        let chrome_right = l
            .playlist
            .map_or(window.right(), |playlist| playlist.left());
        if let Some(controls) = l.controls.as_mut() {
            controls.max.x = chrome_right;
        }
        if let Some(actions) = l.top_actions.as_mut() {
            actions.min.x = (chrome_right - 78.0).max(window.left());
            actions.max.x = actions.min.x + 68.0;
        }
        l
    }

    fn update_sub_margin(&mut self) {
        self.update_sub_margin_for_height(self.viewport_h);
    }

    fn update_sub_margin_for_height(&mut self, viewport_height: f32) {
        // Lift subtitles above the floating control bar. The lift is constant whether
        // the chrome is currently visible or not, so subtitles never jump on pause;
        // mpv measures margins in a 720p reference space.
        let margin = 22.0 + (CTRL_H * 720.0 / viewport_height.max(1.0)).round();
        self.set_prop("sub-margin-y", margin as i64);
    }

    /// A window position saved on a monitor that is no longer attached would
    /// restore the window completely off-screen; detect that once and center.
    fn recover_offscreen_window(&mut self, ctx: &Context) {
        let Some(monitor) = ctx.input(|input| input.viewport().monitor_size) else {
            return;
        };
        let offscreen = ctx
            .input(|input| input.viewport().inner_rect)
            .is_some_and(|rect| {
                rect.right() < 40.0
                    || rect.bottom() < 40.0
                    || rect.left() > monitor.x - 40.0
                    || rect.top() > monitor.y - 40.0
            });
        if offscreen {
            self.settings.window_x = None;
            self.settings.window_y = None;
            // No Center command in this egui version; place the window in the
            // middle of the current monitor using the remembered size.
            let x = (monitor.x - self.settings.window_w as f32) / 2.0;
            let y = (monitor.y - self.settings.window_h as f32) / 2.0;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(
                x.max(0.0),
                y.max(0.0),
            )));
        }
    }

    fn remember_window_geometry(&mut self, ctx: &Context) {
        if self.fullscreen || self.maximized || self.mini_player {
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
                    // A paused seek is settled once mpv reports its target.
                    if self.seek_target_pending && (v - self.seek_target.1).abs() <= 1.0 {
                        self.seek_target_pending = false;
                    }
                }
            }
            "duration" => {
                let d = value.as_f64().unwrap_or(0.0);
                self.seek.duration = d;
                if d > 0.0
                    && let Some(path) = self.current_path.clone()
                {
                    self.playlist.set_duration(&path, d);
                    if !is_url(&path) {
                        // The worker decodes ten frames per file — skip it
                        // entirely when no consumer is enabled.
                        if self.settings.hover_thumbnails || self.settings.continue_watching {
                            self.thumbs.queue(&path, d);
                        }
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
                    // Repaint the video surface once more so mpv replaces the last
                    // decoded frame with the idle background instead of leaving the
                    // stale frame visible under the idle screen.
                    if let Some(renderer) = &self.renderer
                        && let Ok(renderer) = renderer.lock()
                    {
                        renderer.request_render();
                    }
                }
                self.update_title();
            }
            "media-title" | "path" => self.update_title(),
            "volume" => {
                if let Some(v) = value.as_f64() {
                    let ceiling = self.settings.volume_max.max(100) as f64;
                    let volume = v.round().clamp(0.0, ceiling) as i64;
                    self.volume.volume = volume;
                    self.settings.volume = volume;
                }
            }
            "mute" => {
                self.volume.muted = value.as_bool();
                self.settings.mute = value.as_bool();
            }
            "track-list" => {}
            "chapter-list" => {
                self.seek.chapters = value
                    .as_array()
                    .iter()
                    .filter_map(|c| c.get("time").as_f64())
                    .collect();
            }
            "video-params" if value.get("dw").as_f64().is_some() && self.autofit_pending => {
                self.autofit_pending = false;
                self.next_autofit = self.now + 0.05;
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
        if self.settings.loop_mode == "file" || self.settings.repeat_mode == "one" {
            self.cmd(&["seek", "0", "absolute"]);
            self.set_prop("pause", false);
        } else {
            let repeat = if self.settings.loop_mode == "playlist" {
                "all"
            } else {
                self.settings.repeat_mode.as_str()
            };
            if let Some(row) = self.playlist.step(1, repeat, self.settings.shuffle) {
                self.play_row(row, false);
                return;
            }
            // Playlist exhausted: honor keep-open — stay on the last frame, paused,
            // with a brief replay hint. Space or a video click restarts the file;
            // an explicit Stop (F4) is what returns to the idle screen.
            if self.sleep_after_playlist {
                self.sleep_after_playlist = false;
                self.set_prop("pause", true);
                self.osd_ms(i18n::tr("Paused by the sleep timer"), 3500);
            }
            self.osd.center.clear();
            self.flash_glyph = "play".to_string();
            self.flash_until = self.now + FLASH_SECS;
            self.schedule_chrome_hide();
        }
    }

    fn on_engine_event(&mut self, msg: EngineMsg) {
        match msg {
            EngineMsg::Prop(name, value) => self.on_prop(&name, value),
            EngineMsg::FileLoaded => {
                if let Some(engine_path) = self.state.get("path").and_then(|value| value.as_str())
                    && !engine_path.is_empty()
                    && self.current_path.as_deref() != Some(engine_path)
                {
                    return;
                }
                self.file_ready = true;
                self.update_sub_margin();
                self.schedule_chrome_hide();
                if let Some(path) = self.current_path.as_deref() {
                    self.playlist.set_status(path, MediaStatus::Ready);
                }
                self.fail_count = 0;
                self.aspect_name = "Default".into();
                if let Some(path) = self.current_path.clone() {
                    self.settings.add_recent(&path);
                    self.settings.add_history(&path, &path);
                }
                for sub in std::mem::take(&mut self.pending_subs) {
                    let sub = sub.clone();
                    self.cmd(&["sub-add", &sub, "select"]);
                }
                self.apply_saved_track_preference();
                self.apply_saved_media_preference();
                // Decide "audio-only" before the visualizer can install its own
                // synthetic video track; drive the music-mode visualizer off it.
                self.file_is_audio_only = !self.has_video();
                if self.file_is_audio_only
                    && self.settings.auto_visualizer
                    && self.settings.visualizer != "off"
                    && !self.viz_active
                {
                    let mode = match self.settings.visualizer.as_str() {
                        "spectrum" => "spectrum",
                        _ => "wave",
                    };
                    self.set_visualizer(mode);
                }
                if (self.settings.hover_thumbnails || self.settings.continue_watching)
                    && self.thumbs.available()
                {
                    let path = self.current_path.clone();
                    let duration = self.duration();
                    if let Some(path) = path.filter(|p| !is_url(p)) {
                        self.thumbs.queue(&path, duration);
                    }
                }
                self.update_title();
                if let Some(at) = self.resumed_at {
                    self.set_prop("pause", true);
                    self.resume_dialog.open(at);
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
                // mpv exposes the playlist entry id only for end-file events.  The app loads
                // files with `replace`, so the path property is the reliable guard against a
                // late event from the media that was just replaced.  Keep the id in the event
                // for diagnostics and future command correlation.
                let path_matches = self
                    .state
                    .get("path")
                    .and_then(|value| value.as_str())
                    .filter(|path| !path.is_empty())
                    .is_none_or(|path| self.current_path.as_deref() == Some(path));
                if !path_matches {
                    return;
                }
                let _entry_id = entry_id;
                if reason == ffi::MPV_END_FILE_REASON_ERROR && error != 0 {
                    self.file_ready = false;
                    if let Some(path) = self.current_path.as_deref() {
                        self.playlist
                            .set_status(path, MediaStatus::Failed(format!("mpv error {error}")));
                    }
                    self.fail_count += 1;
                    let name = title_of(self.current_path.as_deref().unwrap_or(""));
                    self.osd_error(
                        format!("{}: {name}", i18n::tr("Could not open media")),
                        3500,
                    );
                    if self.fail_count < 3
                        && self.playlist.count() > 1
                        && let Some(row) = self.playlist.step(1, "off", false)
                    {
                        self.next_autofit = self.now + 1.2;
                        self.autofit_pending = false;
                        self.play_row(row, true);
                    }
                }
            }
        }
    }

    fn refresh_derived(&mut self) {
        self.update_buffering();
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
            self.osd.center = format!("{} {pct:.0}%", i18n::tr("Buffering"));
        } else if self.hovered_files.is_empty() {
            self.osd.center.clear();
        }
    }

    fn update_play_state(&mut self) {
        self.apply_on_top();
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
        if self.resume_dialog.open {
            return;
        }
        let Some(path) = self.current_path.clone() else {
            return;
        };
        if !self.file_ready {
            return;
        }
        let pos = self.last_pos;
        let dur = self.duration();
        self.settings.update_history(&path, pos, dur);
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
        self.remember_position();
        self.sync_playlist_settings();
        // The 10 s autosave must never stall the UI thread on JSON + fsync:
        // persist from a background thread instead (single-flight, so a slow
        // disk cannot pile up writers — the next tick carries newer state).
        if SAVING.swap(true, Ordering::AcqRel) {
            return;
        }
        let snapshot = self.settings.clone();
        let generation = SAVE_GENERATION.load(Ordering::Acquire);
        let file_stamp = std::fs::metadata(crate::settings::config_file())
            .and_then(|metadata| metadata.modified())
            .ok();
        let spawned = std::thread::Builder::new()
            .name("settings-save".into())
            .spawn(move || {
                let guard = match SAVE_LOCK.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                // A synchronous save may have superseded this snapshot while
                // the worker was being scheduled. Never write stale state last.
                let current_stamp = std::fs::metadata(crate::settings::config_file())
                    .and_then(|metadata| metadata.modified())
                    .ok();
                if SAVE_GENERATION.load(Ordering::Acquire) == generation
                    && current_stamp == file_stamp
                    && let Err(error) = snapshot.try_save()
                {
                    crate::logging::write(&format!("settings autosave: {error}"));
                }
                drop(guard);
                SAVING.store(false, Ordering::Release);
            })
            .is_ok();
        if !spawned {
            SAVING.store(false, Ordering::Release);
            self.save_settings();
        }
    }

    fn sync_playlist_settings(&mut self) {
        self.settings.playlist_paths = self.playlist.paths();
        self.settings.playlist_current = self
            .playlist
            .current_row()
            .and_then(|row| self.playlist.path(row).map(str::to_string));
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
        self.open_generation = self.open_generation.wrapping_add(1);
        let generation = self.open_generation;
        // A folder walk can touch tens of thousands of entries, and even the
        // routing stat of a stalled network mount can block for seconds: hand
        // the whole batch to the expander thread. Results usually come back
        // within a frame; the "Reading folder…" OSD appears only after 150 ms
        // so plain file opens never flash it.
        if let Some(expander) = &self.expander
            && expander.send((generation, paths.clone(), append)).is_ok()
        {
            self.expander_pending = Some((generation, self.now, false));
            return;
        }
        let expanded = expand_paths(&paths);
        self.open_expanded(&expanded, append);
    }

    /// Continuation of [`Self::open_paths`] once paths are fully expanded.
    fn open_expanded(&mut self, expanded: &[String], append: bool) {
        let mut items: Vec<String> = Vec::new();
        for path in expanded {
            if !is_url(path) && PLAYLIST_EXT.contains(&ext(path).as_str()) && ext(path) != ".cue" {
                items.extend(read_playlist_file(path));
            } else {
                items.push(path.clone());
            }
        }
        let subs: Vec<String> = items.iter().filter(|p| is_sub(p)).cloned().collect();
        let media: Vec<String> = items.iter().filter(|p| !is_sub(p)).cloned().collect();
        if media.is_empty() {
            if !subs.is_empty() && self.has_file() {
                self.add_subtitles(&subs);
            } else if !subs.is_empty() {
                self.osd_error(i18n::tr("Open a video first, then add the subtitle"), 3000);
            } else {
                self.osd_error(i18n::tr("No playable files found"), 3000);
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
                // Playing already: appended media cannot take the subs with it,
                // so load them on the current file instead of losing them.
                if !subs.is_empty() {
                    self.add_subtitles(&subs);
                }
                let n = media.len();
                self.osd(format!(
                    "{} {n} {}",
                    i18n::tr("Added"),
                    if n == 1 {
                        i18n::tr("file to the playlist")
                    } else {
                        i18n::tr("files to the playlist")
                    }
                ));
            }
            return;
        }
        let (group, row) =
            if expanded.len() == 1 && media.len() == 1 && Path::new(&media[0]).is_file() {
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
        let text = match self.services.clipboard_text() {
            Ok(text) => text,
            Err(error) => {
                self.osd_error(
                    format!("{}: {error}", i18n::tr("Could not read the clipboard")),
                    3500,
                );
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
            self.osd_error(
                i18n::tr("The clipboard does not contain a file or URL"),
                3000,
            );
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
        self.sync_playlist_settings();
        self.playlist.reveal_current();
        self.current_path = Some(path.clone());
        self.playlist.set_status(&path, MediaStatus::Loading);
        self.file_ready = false;
        self.last_pos = 0.0;
        self.seek_target = (0.0, 0.0);
        self.seek_target_pending = false;
        self.autofit_pending = !is_url(&path);
        self.file_is_audio_only = false;
        if self.viz_active {
            self.cmd(&["af", "remove", "@fpviz"]);
            self.viz_active = false;
        }
        self.aspect_name = "Default".into();
        self.aspect_value = "-1".into();
        self.seek.reset();
        // The URL dialog's loop count is armed only for that request; a local
        // file must never inherit it (mpv would replay the single-entry
        // playlist behind our back and race `on_eof`).
        if !is_url(&path) {
            self.set_prop("loop-playlist", "no");
        }
        self.resumed_at = if self.settings.resume {
            self.settings.get_position(&path)
        } else {
            None
        };
        // Load paused whenever a resume prompt is coming, so the file never
        // audibly starts for a few frames before the dialog pauses it again.
        self.set_prop("pause", self.resumed_at.is_none());
        let loaded = if let Some(engine) = self.engine() {
            if is_url(&path) {
                engine.set_stream_options(
                    &self.settings.http_user_agent,
                    &self.settings.http_headers,
                );
            }
            engine.load_with_options(&path, None, "")
        } else {
            false
        };
        if !loaded {
            self.playlist.set_status(
                &path,
                MediaStatus::Failed(i18n::tr("Load command failed").into()),
            );
            self.osd_error(i18n::tr("Could not start media load"), 3500);
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
            self.osd(format!("{}: {title}", i18n::tr("Subtitle")));
        }
    }

    fn stop(&mut self, remember: bool) {
        if remember {
            self.remember_position();
        }
        self.file_ready = false;
        self.cmd(&["stop"]);
        self.save_settings();
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
                i18n::tr("Last file")
            } else {
                i18n::tr("First file")
            }),
        }
    }

    // -- playback actions ------------------------------------------------------------------------------

    fn toggle_pause(&mut self) {
        if !self.has_file() {
            if self.playlist.count() > 0 {
                let row = self.playlist.current_row().unwrap_or(0);
                self.play_row(row, true);
            } else {
                self.pending.push(Action::OpenFiles { append: false });
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
        self.osd(if paused {
            i18n::tr("Pause")
        } else {
            i18n::tr("Play")
        });
        // The property cache lags behind the set above; derive the flash from the
        // state we just applied (playing after the toggle means !paused).
        self.flash_glyph = if paused { "pause" } else { "play" }.to_string();
        self.flash_until = self.now + FLASH_SECS;
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
        // While paused, mpv reports the new position only after the seek
        // completes — which can outlive the 0.6 s freshness window. Keep
        // chaining from the pending target until `on_prop` sees it land.
        let paused = self
            .state
            .get("pause")
            .map(|v| v.as_bool())
            .unwrap_or(false);
        let fresh = self.now - self.seek_target.0 < 0.6;
        let pending_paused = paused && self.seek_target_pending;
        let base = if fresh || pending_paused {
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
        self.seek_target_pending = true;
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
                i18n::tr("Next frame")
            } else {
                i18n::tr("Previous frame")
            },
            900,
        );
    }

    fn set_speed(&mut self, value: f64) {
        let value = normalize_speed(value);
        self.set_prop("speed", value);
        self.remember_media_preference();
        self.osd(format!("{}: {value:.2}x", i18n::tr("Speed")));
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
            "{}: {} {}",
            i18n::tr("A-B repeat"),
            if which == 'a' {
                i18n::tr("start point")
            } else {
                i18n::tr("end point")
            },
            fmt_time(pos)
        ));
    }

    fn ab_clear(&mut self) {
        self.set_prop("ab-loop-a", "no");
        self.set_prop("ab-loop-b", "no");
        self.osd(i18n::tr("A-B repeat: off"));
    }

    fn chapter_step(&mut self, direction: i32) {
        if self
            .state
            .get("chapter-list")
            .map(|v| v.as_array().is_empty())
            .unwrap_or(true)
            || !self.has_file()
        {
            self.osd(i18n::tr("No chapters"));
            return;
        }
        self.cmd(&["add", "chapter", &direction.to_string()]);
        self.osd(if direction > 0 {
            i18n::tr("Next chapter")
        } else {
            i18n::tr("Previous chapter")
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
            "off" => i18n::tr("Repeat: off"),
            "one" => i18n::tr("Repeat: current file"),
            _ => i18n::tr("Repeat: playlist"),
        });
        self.save_settings();
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
        self.osd(if self.settings.shuffle {
            i18n::tr("Shuffle: on")
        } else {
            i18n::tr("Shuffle: off")
        });
        self.save_settings();
    }

    fn toggle_resume(&mut self) {
        self.settings.resume = !self.settings.resume;
        self.osd(if self.settings.resume {
            i18n::tr("Remember playback position: on")
        } else {
            i18n::tr("Remember playback position: off")
        });
        self.save_settings();
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
            self.save_settings();
            self.osd(format!(
                "{}: {}",
                i18n::tr("Bookmark saved"),
                fmt_time(time)
            ));
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
        self.osd(if visible {
            i18n::tr("Subtitles: show")
        } else {
            i18n::tr("Subtitles: hide")
        });
    }

    fn set_track(&mut self, kind: TrackKind, sel: TrackSel) {
        self.set_prop(kind.prop(), sel.as_str());
        let label = i18n::tr(kind.label());
        if matches!(sel, TrackSel::No) {
            self.osd(format!("{label}: {}", i18n::tr("Off")));
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
            self.osd(match kind {
                TrackKind::Sub => i18n::tr("No subtitle tracks"),
                TrackKind::Audio => i18n::tr("No audio tracks"),
                TrackKind::Video => i18n::tr("No video tracks"),
            });
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
            // Round at millisecond precision; rounding the sum itself would
            // snap every 0.5s step to a whole second.
            Some(d) => ((self.get_f64("sub-delay") + d) * 1000.0).round() / 1000.0,
        };
        self.set_prop("sub-delay", value);
        self.osd(format!(
            "{}: {value:+.1} {}",
            i18n::tr("Subtitle sync"),
            i18n::tr("sec")
        ));
    }

    fn set_sub_delay(&mut self, value: f64) {
        let value = if value.is_finite() {
            (value.clamp(-600.0, 600.0) * 1000.0).round() / 1000.0
        } else {
            0.0
        };
        self.set_prop("sub-delay", value);
        self.osd(format!(
            "{}: {value:+.1} {}",
            i18n::tr("Subtitle sync"),
            i18n::tr("sec")
        ));
    }

    fn sub_scale(&mut self, delta: f64) {
        let value = if let Some(engine) = self.engine() {
            engine.add("sub-scale", delta, Some(0.3), Some(3.0))
        } else {
            1.0
        };
        self.osd(format!("Subtitle size: {:.0}%", value * 100.0));
    }

    fn set_sub_scale(&mut self, value: f64) {
        let value = if value.is_finite() {
            value.clamp(0.3, 3.0)
        } else {
            1.0
        };
        self.set_prop("sub-scale", value);
        self.osd(format!(
            "{}: {:.0}%",
            i18n::tr("Subtitle size"),
            value * 100.0
        ));
    }

    fn sub_move(&mut self, direction: i32) {
        let value = if let Some(engine) = self.engine() {
            engine.add("sub-pos", (direction * 2) as f64, Some(0.0), Some(100.0))
        } else {
            100.0
        };
        self.osd(format!("{}: {value:.0}", i18n::tr("Subtitle position")));
    }

    fn sub_reset(&mut self) {
        self.set_prop("sub-scale", 1.0);
        self.set_prop("sub-pos", self.settings.sub_pos as f64);
        self.set_prop("sub-delay", 0.0);
        self.osd(i18n::tr("Subtitle size and position reset"));
    }

    fn set_sub_codepage(&mut self, codepage: &str) {
        self.settings.sub_codepage = codepage.to_string();
        self.set_prop("sub-codepage", codepage);
        if self.has_file() {
            self.cmd(&["sub-reload"]);
        }
        self.osd(format!("{}: {codepage}", i18n::tr("Subtitle encoding")));
    }

    // -- video -----------------------------------------------------------------------------------------

    fn set_aspect(&mut self, label: &str, value: &str) {
        self.aspect_name = label.to_string();
        self.aspect_value = value.to_string();
        self.set_prop("video-aspect-override", value);
        self.remember_media_preference();
        self.osd(format!("{}: {label}", i18n::tr("Aspect Ratio")));
    }

    fn set_display_mode(&mut self, mode: &str) {
        self.display_mode = mode.to_string();
        self.set_prop("keepaspect", mode != "stretch");
        self.set_prop("panscan", if mode == "fill" { 1.0 } else { 0.0 });
        self.set_prop("video-unscaled", mode == "original");
        self.osd(match mode {
            "fit" => i18n::tr("Keep aspect ratio"),
            "fill" => i18n::tr("Fill window (crop)"),
            "stretch" => i18n::tr("Stretch to window"),
            _ => i18n::tr("Original size"),
        });
    }

    fn zoom(&mut self, delta: f64) {
        let value = if let Some(engine) = self.engine() {
            engine.add("video-zoom", delta, Some(-2.0), Some(3.0))
        } else {
            0.0
        };
        self.osd(format!(
            "{}: {:.0}%",
            i18n::tr("Zoom"),
            2f64.powf(value) * 100.0
        ));
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
        self.osd(i18n::tr("Pan & Scan reset"));
    }

    fn rotate(&mut self, degrees: Option<i64>) {
        let degrees = match degrees {
            Some(d) => d,
            None => ((self.get_f64("video-rotate") as i64).rem_euclid(360) + 90).rem_euclid(360),
        };
        self.set_prop("video-rotate", degrees);
        self.osd(format!("{}: {degrees}°", i18n::tr("Rotation")));
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
                i18n::tr("Flip Horizontally")
            } else {
                i18n::tr("Flip Vertically")
            },
            if on { i18n::tr("On") } else { i18n::tr("Off") }
        ));
    }

    /// Display name of an image-adjustment property, translated for the OSD.
    fn adjust_label(prop: &str) -> &'static str {
        match prop {
            "brightness" => i18n::tr("Brightness"),
            "contrast" => i18n::tr("Contrast"),
            "saturation" => i18n::tr("Saturation"),
            "hue" => i18n::tr("Hue"),
            _ => i18n::tr("Gamma"),
        }
    }

    fn image_adjust(&mut self, prop: &str, direction: i32) {
        let value = if let Some(engine) = self.engine() {
            engine.add(prop, (direction * 2) as f64, Some(-100.0), Some(100.0))
        } else {
            0.0
        };
        self.osd(format!("{}: {value:+.0}", Self::adjust_label(prop)));
    }

    fn set_image_adjust(&mut self, prop: &str, value: f64) {
        let value = if value.is_finite() {
            value.clamp(-100.0, 100.0).round()
        } else {
            0.0
        };
        self.set_prop(prop, value);
        self.osd(format!("{}: {value:+.0}", Self::adjust_label(prop)));
    }

    fn image_reset(&mut self) {
        for prop in ["brightness", "contrast", "saturation", "hue", "gamma"] {
            self.set_prop(prop, 0.0);
        }
        self.osd(i18n::tr("Image adjustments reset"));
    }

    fn toggle_deinterlace(&mut self) {
        let value = !self.get_bool("deinterlace");
        self.set_prop("deinterlace", value);
        self.osd(format!(
            "{}: {}",
            i18n::tr("Deinterlace"),
            if value {
                i18n::tr("On")
            } else {
                i18n::tr("Off")
            }
        ));
    }

    fn set_hwdec(&mut self, enabled: bool) {
        self.settings.hwdec = enabled;
        self.set_prop("hwdec", Engine::hwdec_value(enabled));
        self.save_settings();
        self.osd(format!(
            "{}: {}",
            i18n::tr("Hardware decoding"),
            if enabled {
                i18n::tr("On")
            } else {
                i18n::tr("Off")
            }
        ));
    }

    // -- audio -----------------------------------------------------------------------------------------

    fn set_volume(&mut self, value: i64) {
        let value = value.clamp(0, self.settings.volume_max.max(100));
        self.set_prop("volume", value);
        if value > 0 && self.state.get("mute").map(|v| v.as_bool()).unwrap_or(false) {
            self.set_prop("mute", false);
        }
        self.volume.volume = value;
        self.osd(format!("{}: {value}%", i18n::tr("Volume")));
    }

    fn volume_step(&mut self, direction: i32) {
        let step = self.settings.volume_step as i64;
        let current = self.volume.volume;
        self.set_volume(current + direction as i64 * step);
    }

    fn toggle_mute(&mut self) {
        let muted = !self.get_bool("mute");
        self.set_prop("mute", muted);
        self.osd(format!(
            "{}: {}",
            i18n::tr("Mute"),
            if muted {
                i18n::tr("On")
            } else {
                i18n::tr("Off")
            }
        ));
    }

    fn audio_delay(&mut self, delta: Option<f64>) {
        let value = match delta {
            None => 0.0,
            // Millisecond precision: rounding the sum would swallow the
            // 0.05s steps entirely while the delay is still near zero.
            Some(d) => ((self.get_f64("audio-delay") + d) * 1000.0).round() / 1000.0,
        };
        self.set_prop("audio-delay", value);
        self.osd(format!(
            "{}: {value:+.2} {}",
            i18n::tr("Audio sync"),
            i18n::tr("sec")
        ));
    }

    fn set_audio_delay(&mut self, value: f64) {
        let value = if value.is_finite() {
            (value.clamp(-60.0, 60.0) * 1000.0).round() / 1000.0
        } else {
            0.0
        };
        self.set_prop("audio-delay", value);
        self.osd(format!(
            "{}: {value:+.2} {}",
            i18n::tr("Audio sync"),
            i18n::tr("sec")
        ));
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
            "{}: {}",
            i18n::tr("Normalize volume"),
            if enabled {
                i18n::tr("On")
            } else {
                i18n::tr("Off")
            }
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
        if self.eq_active {
            self.cmd(&["af", "remove", "@fpeq"]);
            self.eq_active = false;
        }
        if self.settings.eq_enabled && !bands.is_empty() {
            let chain: Vec<String> = bands
                .iter()
                .map(|(f, g)| format!("equalizer=f={f}:t=o:w=1:g={g}"))
                .collect();
            let spec = format!("@fpeq:lavfi=[{}]", chain.join(","));
            self.cmd(&["af", "add", &spec]);
            self.eq_active = true;
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
                self.osd_error(i18n::tr("Could not capture the current frame"), 3000);
                return;
            }
            let result = std::fs::read(&path)
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|image| image.to_rgba8());
            let _ = std::fs::remove_file(&path);
            let Some(image) = result else {
                self.osd_error(i18n::tr("Could not read the captured frame"), 3000);
                return;
            };
            let (width, height) = image.dimensions();
            match self.services.clipboard_put_image(
                width as usize,
                height as usize,
                image.into_raw(),
            ) {
                Ok(()) => self.osd_ms(i18n::tr("Frame copied to clipboard"), 2200),
                Err(error) => self.osd_error(
                    format!("{}: {error}", i18n::tr("Could not copy frame")),
                    3000,
                ),
            }
            return;
        }
        let folder = self.screenshot_dir();
        if std::fs::create_dir_all(&folder).is_err() {
            self.osd_error(
                format!("{}: {folder}", i18n::tr("Cannot create folder")),
                3500,
            );
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
        let captured = self
            .engine()
            .and_then(|engine| engine.cmd_sync(&["screenshot-to-file", &path, mode]))
            .is_some()
            && Path::new(&path).is_file();
        if !captured {
            self.osd_error(i18n::tr("Could not save the current frame"), 3000);
            return;
        }
        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.osd_ms(format!("{}: {name}", i18n::tr("Saved")), 2500);
    }

    fn open_screenshot_folder(&mut self) {
        let folder = self.screenshot_dir();
        let _ = std::fs::create_dir_all(&folder);
        self.services.reveal_path(&folder);
    }

    // -- window ----------------------------------------------------------------------------------------

    fn toggle_fullscreen(&mut self, ctx: &Context) {
        self.state_changed = self.now;
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::SetMonitor(
                self.settings.fullscreen_monitor,
            ));
        }
        self.schedule_chrome_hide();
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
            self.osd(i18n::tr("Window size is not available in full screen"));
            return;
        }
        let limited = self.fit_window(Some(percent as f64 / 100.0), ctx);
        self.osd(format!(
            "{}: {percent}%{}",
            i18n::tr("Window size"),
            if limited {
                format!(" ({})", i18n::tr("limited by screen"))
            } else {
                String::new()
            }
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
        // Floating chrome overlays the video, so it consumes no layout space.
        let extra = Vec2::ZERO;
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
        self.save_settings();
    }

    fn toggle_playlist(&mut self, ctx: &Context) {
        self.playlist_visible = !self.playlist_visible;
        // A hidden drawer can no longer own the keyboard: release list focus so the
        // navigation keys fall back to the player keymap instead of vanishing.
        if !self.playlist_visible {
            self.playlist.has_focus = false;
        } else {
            // Opening the drawer brings the playing row into view.
            self.playlist.reveal_current();
        }
        self.settings.playlist_visible = self.playlist_visible;
        self.schedule_chrome_hide();
        self.update_sub_margin();
        ctx.request_repaint();
    }

    fn set_on_top(&mut self, mode: &str) {
        self.settings.on_top = mode.to_string();
        self.on_top = mode == "always" || (mode == "playing" && self.is_playing());
        self.osd(match mode {
            "never" => i18n::tr("On top: never"),
            "always" => i18n::tr("On top: always"),
            _ => i18n::tr("On top: while playing"),
        });
        self.save_settings();
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
        if want != self.on_top {
            self.on_top = want;
        }
    }

    fn toggle_remaining(&mut self) {
        self.show_remaining = !self.show_remaining;
        self.settings.show_remaining = self.show_remaining;
        self.save_settings();
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
            None => return format!("{}\n", i18n::tr("No file is open.")),
        };
        if !self.has_file() {
            return format!("{}\n", i18n::tr("No file is open."));
        }
        let path = self.current_path.clone().unwrap_or_default();
        let mut lines = vec![format!(
            "{:<9}{}",
            i18n::tr("File"),
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
                "{:<9}{}  {}x{}  {:.3} fps  {}  {}",
                i18n::tr("Video"),
                video.get("codec").as_str().unwrap_or("").to_uppercase(),
                params.get("w").as_f64().unwrap_or(0.0) as i64,
                params.get("h").as_f64().unwrap_or(0.0) as i64,
                fps,
                params.get("pixelformat").as_str().unwrap_or(""),
                if hw != "no" {
                    format!("{} ({hw})", i18n::tr("hardware"))
                } else {
                    i18n::tr("software").to_string()
                },
            ));
            let bitrate = engine.get("video-bitrate").and_then(|v| v.as_f64());
            if let Some(bitrate) = bitrate
                && !brief
            {
                lines.push(format!("          {:.0} kb/s", bitrate / 1000.0));
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
                "{:<9}{}  {} Hz  {} ch  {}",
                i18n::tr("Audio"),
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
                "{:<9}{}  {} {:+.1} {}",
                i18n::tr("Subtitle"),
                skin::track_label(sub).replace("&&", "&"),
                i18n::tr("Subtitle sync"),
                engine.get_f64("sub-delay"),
                i18n::tr("sec"),
            ));
        }
        lines.push(format!(
            "{:<9}{} / {}  {:.2}x  A/V {:+.3}  {} {}",
            i18n::tr("Playback"),
            fmt_time(self.position()),
            fmt_time(self.duration()),
            engine.get_f64("speed"),
            engine.get_f64("avsync"),
            i18n::tr("dropped"),
            (engine.get_f64("frame-drop-count") + engine.get_f64("decoder-frame-drop-count"))
                as i64,
        ));
        if !brief {
            lines.push(String::new());
            lines.push(i18n::tr("Tracks").into());
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
                    lines.push(i18n::tr("Chapters").into());
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
            lines.push(i18n::tr("System").into());
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
                lines.push(i18n::tr("Recent engine messages").into());
                let start = logs.len().saturating_sub(15);
                for line in &logs[start..] {
                    lines.push(format!("  {line}"));
                }
            }
        }
        lines.join("\n")
    }

    /// Re-apply the appearance globals (theme, accent, language) after preferences change.
    /// Apply one remotely patched setting immediately where it can take
    /// effect live; everything else is simply persisted (GUI is the writer).
    fn apply_runtime_setting(&mut self, ctx: &Context, key: &str, value: &serde_json::Value) {
        match key {
            "volume" => {
                if let Some(volume) = value.as_i64() {
                    self.set_volume(volume);
                }
            }
            "mute" => {
                if let Some(muted) = value.as_bool() {
                    self.set_prop("mute", muted);
                }
            }
            "theme" | "accent" | "accent_custom" | "language" => {
                self.apply_appearance(ctx);
            }
            "now_playing" => {
                if self.settings.now_playing && self.now_playing.is_none() {
                    if let Some(tx) = self.now_playing_tx.clone() {
                        self.now_playing = crate::nowplaying::NowPlaying::new(tx);
                    }
                } else if !self.settings.now_playing
                    && let Some(now_playing) = self.now_playing.take()
                {
                    now_playing.clear();
                }
            }
            // hover_thumbnails / row_previews / cover_art / continue_watching
            // are read live each frame; window geometry keys apply on restart.
            _ => {}
        }
    }

    /// Push the configured accent (preset or custom hex) into the skin.
    fn apply_accent(&self) {
        Player::apply_accent_for(&self.settings);
    }

    fn apply_accent_for(settings: &Settings) {
        if settings.accent == "custom" {
            skin::set_custom_accent(&settings.accent_custom);
        } else {
            skin::set_accent(Accent::from_str(&settings.accent));
        }
    }

    fn apply_appearance(&mut self, ctx: &Context) {
        skin::set_theme(Theme::from_str(&self.settings.theme));
        self.apply_accent();
        i18n::set_lang(Lang::from_str(&self.settings.language));
        skin::apply(ctx);
        skin::apply_fonts(ctx, i18n::rtl());
        ctx.request_repaint();
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
        // mpv rejects a numeric 0 peak; "auto" is its off value.
        self.set_prop(
            "target-peak",
            if s.hdr_tone_mapping { "203" } else { "auto" },
        );
        self.set_prop(
            "tone-mapping",
            if s.hdr_tone_mapping {
                s.hdr_tone_mapping_mode.as_str()
            } else {
                "auto"
            },
        );
        self.set_prop("hdr-compute-peak", s.hdr_tone_mapping);

        // -- engine-backed feature settings ---------------------------------------------------
        self.set_prop("volume-max", s.volume_max);
        if s.volume_max >= 100 {
            self.volume.max = s.volume_max;
        }
        // Looping is owned by the app playlist. mpv only ever contains the
        // currently loaded entry because every file is loaded with `replace`.
        self.set_prop("loop-file", "no");
        self.set_prop("loop-playlist", "no");
        self.set_prop("audio-pitch-correction", s.audio_pitch_correction);
        self.set_prop("audio-channels", s.audio_channels.as_str());
        self.set_prop(
            "video-sync",
            Engine::video_sync_value(s.video_sync.as_str()),
        );
        self.set_prop("interpolation", s.interpolation);
        if s.interpolation {
            self.set_prop("tscale", "oversample");
        }
        self.set_prop("scale", s.scale_profile.as_str());
        self.set_prop("ytdl-format", Engine::ytdl_format(&s.ytdl_quality));
        // Subtitle styling.
        self.set_prop("sub-color", format!("#FF{}", &s.sub_color[1..]));
        self.set_prop("sub-border-size", s.sub_border_size);
        self.set_prop(
            "sub-border-color",
            format!("#FF{}", &s.sub_border_color[1..]),
        );
        self.set_prop("sub-shadow-offset", s.sub_shadow_offset);
        let back = if s.sub_back_enable {
            format!("#C8{}", &s.sub_back_color[1..])
        } else {
            "#00000000".to_string()
        };
        self.set_prop("sub-back-color", back);

        // Now Playing follows the feature toggle at runtime.
        if self.settings.now_playing && self.now_playing.is_none() {
            if let Some(tx) = self.now_playing_tx.clone() {
                self.now_playing = crate::nowplaying::NowPlaying::new(tx);
            }
        } else if !self.settings.now_playing
            && let Some(now_playing) = self.now_playing.take()
        {
            now_playing.clear();
        }
    }

    // -- new feature helpers -----------------------------------------------------------------

    /// mpv loop options from the "none" | "file" | "playlist" setting.
    fn set_loop_mode(&mut self, mode: &str) {
        self.settings.loop_mode = mode.to_string();
        self.save_settings();
        self.osd(match mode {
            "file" => i18n::tr("Loop: current file"),
            "playlist" => i18n::tr("Loop: playlist"),
            _ => i18n::tr("Loop: off"),
        });
    }

    /// 0 = off, -1 = after playlist, otherwise minutes.
    fn set_sleep_timer(&mut self, minutes: i32) {
        match minutes {
            0 => {
                self.sleep_deadline = None;
                self.sleep_after_playlist = false;
                self.osd(i18n::tr("Sleep timer off"));
            }
            -1 => {
                self.sleep_deadline = None;
                self.sleep_after_playlist = true;
                self.osd(i18n::tr("Sleep after playlist"));
            }
            mins => {
                self.sleep_after_playlist = false;
                self.sleep_deadline = Some(self.now + f64::from(mins) * 60.0);
                self.osd(format!(
                    "{}: {mins} {}",
                    i18n::tr("Sleep timer"),
                    i18n::tr("min")
                ));
            }
        }
    }

    fn sleep_timer_tick(&mut self) {
        if let Some(deadline) = self.sleep_deadline
            && self.now >= deadline
        {
            self.sleep_deadline = None;
            if self.has_file() {
                self.set_prop("pause", true);
            }
            self.osd_ms(i18n::tr("Paused by the sleep timer"), 3500);
        }
    }

    /// Toggle one of the preset lavfi video filters.
    fn set_video_filter(&mut self, filter: &'static str, on: bool) {
        let spec = match filter {
            "deband" => "@fpdeband:lavfi=[gradfun=1.2]",
            "denoise" => "@fpdenoise:lavfi=[hqdn3d=1.5:1.5:6:6]",
            _ => "@fpsharpen:lavfi=[unsharp=5:5:0.8]",
        };
        let name = match filter {
            "deband" => "@fpdeband",
            "denoise" => "@fpdenoise",
            _ => "@fpsharpen",
        };
        if on {
            self.cmd(&["vf", "add", spec]);
        } else {
            self.cmd(&["vf", "remove", name]);
        }
        match filter {
            "deband" => self.video_filters.0 = on,
            "denoise" => self.video_filters.1 = on,
            _ => self.video_filters.2 = on,
        }
        self.osd(format!(
            "{}: {}",
            match filter {
                "deband" => i18n::tr("Deband"),
                "denoise" => i18n::tr("Denoise"),
                _ => i18n::tr("Sharpen"),
            },
            if on { i18n::tr("On") } else { i18n::tr("Off") }
        ));
    }

    /// Install/remove the lavfi audio visualizer (a synthesized video track).
    fn set_visualizer(&mut self, mode: &str) {
        let spec = match mode {
            "wave" => "@fpviz:lavfi=[showwaves=s=854x480:rate=30:mode=cline]",
            "spectrum" => "@fpviz:lavfi=[showspectrum=s=854x480:mode=combined:slide=scroll]",
            _ => "",
        };
        if self.viz_active {
            self.cmd(&["af", "remove", "@fpviz"]);
            self.viz_active = false;
        }
        if !spec.is_empty() {
            self.cmd(&["af", "add", spec]);
            self.viz_active = true;
        }
        self.settings.visualizer = mode.to_string();
        self.save_settings();
        self.osd(format!(
            "{}: {}",
            i18n::tr("Audio Visualizer"),
            match mode {
                "wave" => i18n::tr("Wave"),
                "spectrum" => i18n::tr("Spectrum"),
                _ => i18n::tr("Off"),
            }
        ));
    }

    /// Borderless, always-on-top small window (PiP-style mini player).
    fn toggle_mini_player(&mut self, ctx: &Context) {
        self.mini_player = !self.mini_player;
        if self.mini_player {
            let size = ctx.content_rect().size();
            self.mini_prev_size = Some(size);
            self.mini_prev_fullscreen = self.fullscreen;
            self.mini_prev_maximized = self.maximized;
            if self.playlist_visible {
                self.toggle_playlist(ctx);
            }
            self.fullscreen = false;
            self.maximized = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(480.0, 300.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop,
            ));
            self.applied_on_top = Some(true);
        } else {
            self.fullscreen = self.mini_prev_fullscreen;
            self.maximized = self.mini_prev_maximized;
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!self.fullscreen));
            if let Some(size) = self.mini_prev_size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            // Re-sync the level with the on-top preference.
            self.applied_on_top = None;
            self.request_window_mode(ctx);
        }
        self.schedule_chrome_hide();
        ctx.request_repaint();
    }

    /// Open a DVD (VIDEO_TS) or Blu-ray (BDMV) folder through mpv's disc support.
    fn open_disc_dialog(&mut self) {
        let Some(folder) = self.services.pick_folder(&self.settings.last_dir) else {
            return;
        };
        let is_dvd = Path::new(&folder).join("VIDEO_TS").is_dir();
        let is_bd = Path::new(&folder).join("BDMV").is_dir();
        if !is_dvd && !is_bd {
            self.osd_error(i18n::tr("Not a disc folder (VIDEO_TS / BDMV)"), 3500);
            return;
        }
        let url = if is_dvd {
            "dvd://".to_string()
        } else {
            "bluray://".to_string()
        };
        self.settings.last_dir = folder.clone();
        // Disc titles are not regular files; hand them straight to mpv.
        self.current_path = Some(url.clone());
        self.playlist.set_items(std::slice::from_ref(&url));
        self.playlist.set_current(Some(0));
        self.playlist.set_status(&url, MediaStatus::Loading);
        self.file_ready = false;
        self.last_pos = 0.0;
        self.autofit_pending = false;
        self.file_is_audio_only = false;
        self.seek.reset();
        self.set_prop("pause", false);
        let loaded = if let Some(engine) = self.engine() {
            let device = Value::Str(folder.clone());
            if is_dvd {
                engine.set("dvd-device", &device);
            } else {
                engine.set("bluray-device", &device);
            }
            engine.load_with_options(&url, None, "")
        } else {
            false
        };
        if !loaded {
            self.playlist.set_status(
                &url,
                MediaStatus::Failed(i18n::tr("Load command failed").into()),
            );
            self.osd_error(i18n::tr("Could not start media load"), 3500);
        }
        self.update_title();
    }

    /// Save the current playlist as an extended .m3u8 file.
    fn export_playlist(&mut self) {
        if self.playlist.count() == 0 {
            return;
        }
        let Some(path) = self.services.pick_save_file("playlist.m3u8") else {
            return;
        };
        let items: Vec<(String, String, Option<f64>)> = self
            .playlist
            .items
            .iter()
            .map(|item| (item.path.clone(), item.title.clone(), item.duration))
            .collect();
        match write_m3u(&path, &items) {
            Ok(_) => {
                self.osd(format!("{}: {path}", i18n::tr("Playlist exported")));
            }
            Err(error) => {
                self.osd_error(
                    format!("{}: {error}", i18n::tr("Could not export playlist")),
                    3500,
                );
            }
        }
    }

    /// Trackpad pinch → fullscreen; two-finger horizontal scroll over the video → seek.
    fn handle_gestures(&mut self, ctx: &Context, layout: &Layout) {
        let over_video = ctx.pointer_hover_pos().is_some_and(|pos| {
            layout.video.contains(pos)
                && !layout.playlist.is_some_and(|rect| rect.contains(pos))
                && !layout.controls.is_some_and(|rect| rect.contains(pos))
                && !layout.top_actions.is_some_and(|rect| rect.contains(pos))
                && !self.playlist_visible
                && !self.settings_pop.open
                && !self.menu.is_open()
                && !self.playlist.menu_open()
                && !self.any_dialog_open()
                && !self.palette.open
        });

        // Pinch-to-fullscreen reads only `Zoom` events (trackpad pinch). The
        // ctrl+scroll wheel path below means video zoom instead — `zoom_delta()`
        // would conflate the two. Per-frame scale deltas accumulate into
        // `pinch_scale` for the whole gesture; crossing a threshold toggles once,
        // and the gesture resets after a short quiet period.
        let mut magnify = 0.0f32;
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Zoom(delta) = event {
                    magnify += *delta;
                }
            }
        });
        if over_video && magnify != 0.0 {
            self.pinch_scale = (self.pinch_scale * (1.0 + magnify)).clamp(0.2, 5.0);
            self.pinch_last_at = self.now;
            if !self.pinch_latched && !(0.78..=1.28).contains(&self.pinch_scale) {
                self.pinch_latched = true;
                self.pending.push(Action::ToggleFullscreen);
            }
        } else if self.now - self.pinch_last_at > 0.3 {
            // Gesture ended: re-arm and recenter the accumulated scale.
            self.pinch_latched = false;
            self.pinch_scale = 1.0;
        }

        if !over_video {
            return;
        }
        // Wheel handling over the video: two-finger scroll seeks, ctrl/cmd+scroll
        // zooms the video itself (browser convention), plain wheels just seek.
        let mut steps = 0i32;
        let mut zoom_steps = 0i32;
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } = event
                {
                    let (x, y) = match unit {
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                            (delta.x, delta.y)
                        }
                        egui::MouseWheelUnit::Point => (delta.x / 10.0, delta.y / 10.0),
                    };
                    if modifiers.ctrl || modifiers.command {
                        self.zoom_scroll_acc += y;
                    } else if !modifiers.alt {
                        self.video_scroll_acc += x;
                    }
                    while self.video_scroll_acc >= 1.0 {
                        steps += 1;
                        self.video_scroll_acc -= 1.0;
                    }
                    while self.video_scroll_acc <= -1.0 {
                        steps -= 1;
                        self.video_scroll_acc += 1.0;
                    }
                    while self.zoom_scroll_acc >= 1.0 {
                        zoom_steps += 1;
                        self.zoom_scroll_acc -= 1.0;
                    }
                    while self.zoom_scroll_acc <= -1.0 {
                        zoom_steps -= 1;
                        self.zoom_scroll_acc += 1.0;
                    }
                }
            }
        });
        if steps != 0 {
            let amount = steps as f64 * self.settings.seek_small;
            self.pending.push(Action::SeekRel(amount));
        }
        if zoom_steps != 0 {
            self.pending.push(Action::Zoom(zoom_steps as f64 * 0.1));
        }
    }

    fn quit(&mut self) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.remember_position();
        self.settings.first_run = false;
        self.save_settings();
    }

    fn request_close(&mut self, ctx: &Context) {
        if self.closing {
            return;
        }
        self.quit();
        if self.has_file() {
            self.cmd(&["stop"]);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn apply_resume_choice(&mut self, choice: ResumeChoice) {
        let Some(position) = self.resumed_at.take() else {
            return;
        };
        match choice {
            ResumeChoice::Resume => {
                self.last_pos = position;
                self.seek.position = position;
                self.cmd(&["seek", &format!("{position:.3}"), "absolute"]);
                self.set_prop("pause", false);
                self.osd(format!(
                    "{}: {}",
                    i18n::tr("Resumed at"),
                    fmt_time(position)
                ));
            }
            ResumeChoice::StartOver => {
                self.last_pos = 0.0;
                self.seek.position = 0.0;
                if let Some(path) = &self.current_path {
                    self.settings.set_position(path, None);
                }
                self.save_settings();
                self.cmd(&["seek", "0", "absolute"]);
                self.set_prop("pause", false);
                self.osd(i18n::tr("Started from the beginning"));
            }
        }
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
        let files = self
            .services
            .pick_files(i18n::tr("Media files"), &media_extensions(), &start);
        if !files.is_empty() {
            self.pending.open_paths(files, append);
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
        if let Some(folder) = self.services.pick_folder(&start) {
            self.pending.open_paths(vec![folder], append);
        }
    }

    fn load_subtitle_dialog(&mut self) {
        if !self.has_file() {
            self.osd(i18n::tr("Open a video first"));
            return;
        }
        let mut start = if self.settings.last_sub_dir.is_empty() {
            self.settings.last_dir.clone()
        } else {
            self.settings.last_sub_dir.clone()
        };
        if let Some(path) = &self.current_path
            && !is_url(path)
        {
            start = Path::new(path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        let files = self
            .services
            .pick_files(i18n::tr("Subtitle files"), &sub_extensions(), &start);
        if !files.is_empty() {
            self.add_subtitles(&files);
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
        if let Some(path) =
            self.services
                .pick_file(i18n::tr("Audio files"), &media_extensions(), &start)
        {
            self.cmd(&["audio-add", &path, "select"]);
            let title = title_of(&path);
            self.osd_ms(format!("{}: {title}", i18n::tr("Audio")), 2500);
        }
    }

    // -- action dispatch -------------------------------------------------------------------------------

    fn dispatch(&mut self, action: Action, ctx: &Context) {
        let context = menus::CommandContext {
            has_file: self.has_file(),
            playlist_count: self.playlist.count(),
            can_undo_playlist: self.playlist.can_undo(),
            can_redo_playlist: self.playlist.can_redo(),
        };
        if let Some(spec) = menus::command_spec(menus::action_id(&action)) {
            if !spec.enabled(context) {
                return;
            }
            if spec.mutation {
                self.state_changed = self.now;
            }
        }
        match action {
            Action::OpenFiles { append } => self.open_files_dialog(append),
            Action::OpenFolder { append } => self.open_folder_dialog(append),
            Action::OpenUrl => self.url_dialog.open(false),
            Action::OpenClipboard => self.open_clipboard(),
            Action::OpenRecent(path) => self.pending.open_paths(vec![path], false),
            Action::ClearRecent => {
                self.settings.recent.clear();
                self.save_settings();
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
                    self.save_settings();
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
            Action::SetSubDelay(value) => self.set_sub_delay(value),
            Action::SubScale(delta) => self.sub_scale(delta),
            Action::SetSubScale(value) => self.set_sub_scale(value),
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
            Action::SetImageAdjust(prop, value) => self.set_image_adjust(prop, value),
            Action::ImageReset => self.image_reset(),
            Action::ToggleDeinterlace => self.toggle_deinterlace(),
            Action::SetHwdec(enabled) => self.set_hwdec(enabled),

            Action::SetVolume(value) => self.set_volume(value),
            Action::VolumeStep(direction) => self.volume_step(direction),
            Action::ToggleMute => self.toggle_mute(),
            Action::AudioDelay(delta) => self.audio_delay(delta),
            Action::SetAudioDelay(value) => self.set_audio_delay(value),
            Action::SetAudioDevice(name) => self.set_prop("audio-device", name.as_str()),
            Action::ToggleNormalize => self.set_normalize(!self.normalize),
            Action::SetNormalize(enabled) => self.set_normalize(enabled),
            Action::SetEqualizer(gains, enabled) => {
                self.settings.eq = gains;
                self.settings.eq_enabled = enabled;
                self.apply_eq();
                // Persist immediately instead of waiting for the 10s autosave.
                self.save_settings();
            }
            Action::SetSubPos(value) => {
                self.set_prop("sub-pos", value);
                // mpv clamps sub-pos to 0..=100; keep the stored setting in
                // range so it never disagrees with what is actually playing.
                self.settings.sub_pos = value.round().clamp(0.0, 100.0) as i64;
                self.save_settings();
            }
            Action::PlayUrl {
                url,
                loop_s,
                append,
            } => {
                // Arm (or clear) the repeat count for exactly this request.
                if loop_s > 0 {
                    self.set_prop("loop-playlist", loop_s);
                } else {
                    self.set_prop("loop-playlist", "no");
                }
                self.pending.open_paths(vec![url], append);
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
            Action::UndoPlaylist => {
                if self.playlist.undo() {
                    self.sync_playlist_settings();
                    self.update_title();
                    self.save_settings();
                    self.osd(i18n::tr("Playlist change undone"));
                }
            }
            Action::RedoPlaylist => {
                if self.playlist.redo() {
                    self.sync_playlist_settings();
                    self.update_title();
                    self.save_settings();
                    self.osd(i18n::tr("Playlist change redone"));
                }
            }
            Action::PlaylistCopyPath(row) => {
                if let Some(path) = self.playlist.path(row) {
                    ctx.copy_text(path.to_string());
                    self.osd(i18n::tr("Path copied to the clipboard"));
                }
            }
            Action::PlaylistShowInFolder(row) => {
                if let Some(path) = self.playlist.path(row) {
                    self.services.reveal_path(path);
                }
            }

            Action::Screenshot(mode, clipboard) => self.screenshot(mode, clipboard),
            Action::OpenScreenshotFolder => self.open_screenshot_folder(),

            Action::SetOnTop(mode) => self.set_on_top(&mode),
            Action::CycleOnTop => self.cycle_on_top(),
            Action::ToggleInfo => self.toggle_info(),
            Action::ToggleRemaining => self.toggle_remaining(),
            Action::ToggleFullscreen => self.toggle_fullscreen(ctx),
            Action::ExitFullscreen => {
                if self.fullscreen {
                    self.toggle_fullscreen(ctx);
                }
            }
            Action::Close => self.request_close(ctx),
            Action::TogglePlaylist => self.toggle_playlist(ctx),

            Action::ShowPreferences(tab) => {
                self.prefs.open(&self.settings);
                if let Some(tab) = tab {
                    self.prefs.select_tab(tab);
                }
            }
            Action::ShowCommandPalette => self.palette.open(),
            Action::ShowControlPanel(tab) => {
                self.panel.open_panel();
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
                    .set(i18n::tr("Playback/System Info..."), &text, true, false);
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
                    "{}\n\n{}\n\n{}\n  mpv     {mpv}\n  ffmpeg  {ffmpeg}\n\n{}\n  {}\n  {}\n  {}",
                    env!("CARGO_PKG_VERSION"),
                    i18n::tr(
                        "A fast, native media player for macOS — built with Rust, egui and libmpv."
                    ),
                    i18n::tr("Engine"),
                    i18n::tr("Tips"),
                    i18n::tr("Right-click the video for the full menu."),
                    i18n::tr("Press ⌘K for the command palette."),
                    i18n::tr("Settings: ~/Library/Application Support/MacPlayer/settings.json")
                );
                self.text_dialog.set(i18n::tr("About"), &text, false, false);
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
            Action::OpenDisc => self.open_disc_dialog(),
            Action::SetYtdlQuality(quality) => {
                self.settings.ytdl_quality = quality.to_string();
                let format = Engine::ytdl_format(quality);
                self.set_prop("ytdl-format", format);
                self.save_settings();
                self.osd(format!("{}: {quality}", i18n::tr("Online Stream Quality")));
            }
            Action::SetLoopMode(mode) => self.set_loop_mode(&mode),
            Action::SetSleepTimer(minutes) => self.set_sleep_timer(minutes),
            Action::SetVideoSync(mode) => {
                self.settings.video_sync = mode.clone();
                self.set_prop("video-sync", Engine::video_sync_value(&mode));
                self.save_settings();
                self.osd(format!("{}: {mode}", i18n::tr("Motion & Sync")));
            }
            Action::SetInterpolation(on) => {
                self.settings.interpolation = on;
                self.set_prop("interpolation", on);
                if on {
                    self.set_prop("tscale", "oversample");
                }
                self.save_settings();
                self.osd(format!(
                    "{}: {}",
                    i18n::tr("Frame Interpolation"),
                    if on { i18n::tr("On") } else { i18n::tr("Off") }
                ));
            }
            Action::SetScaleProfile(profile) => {
                self.settings.scale_profile = profile.to_string();
                self.set_prop("scale", profile);
                self.save_settings();
                self.osd(format!("{}: {profile}", i18n::tr("Image Scaling")));
            }
            Action::SetVideoFilter { filter, on } => self.set_video_filter(filter, on),
            Action::SetVisualizer(mode) => self.set_visualizer(mode),
            Action::ToggleMiniPlayer => self.toggle_mini_player(ctx),
            Action::ShowHistory => self.history_dialog.open = true,
            Action::OpenUpdatePage => {
                self.services.open_url(crate::update::releases_page());
            }
            Action::ReportIssue => {
                // Bundle versions + recent diagnostics; the browser opens the
                // issue tracker and the text is on the clipboard.
                let engine = self.engine();
                let mpv = engine
                    .and_then(|e| e.get("mpv-version"))
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "?".into());
                let report = format!(
                    "MacPlayer {}\nmpv: {}\n\n--- recent diagnostics ---\n{}",
                    env!("CARGO_PKG_VERSION"),
                    mpv,
                    engine.map(|e| e.log_lines().join("\n")).unwrap_or_default(),
                );
                ctx.copy_text(report);
                self.services
                    .open_url("https://github.com/hamidreza-tn/MacPlayer/issues/new");
                self.osd_ms(i18n::tr("Report copied to the clipboard"), 2500);
            }
            Action::ClearHistory => {
                self.settings.clear_history();
                self.save_settings();
            }
            Action::PlayHistory(path) => {
                self.pending.open_paths(vec![path], false);
            }
            Action::ExportPlaylist => self.export_playlist(),
            Action::ApplySettings { hwdec_changed } => {
                if let Some((settings, changed)) = self.prefs.result.take() {
                    self.settings = settings;
                    self.apply_appearance(ctx);
                    self.apply_settings(changed || hwdec_changed);
                    self.save_settings();
                }
                // Audio device options (passthrough/exclusive) and cache sizing
                // only take effect on a fresh engine; say so instead of failing
                // silently.
                self.osd_ms(
                    i18n::tr("Some audio and cache options apply after restart"),
                    2600,
                );
            }
            Action::SetThemeLive(theme) => {
                self.settings.theme = theme;
                self.apply_appearance(ctx);
                self.save_settings();
            }
            Action::SetLanguageLive(language) => {
                self.settings.language = language;
                self.apply_appearance(ctx);
                self.save_settings();
            }
            Action::Quit => {
                self.request_close(ctx);
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
            K::Comma if ctrl => (Action::ShowPreferences(None), false),
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
            K::K if ctrl => (Action::ShowCommandPalette, false),
            K::P if ctrl => (Action::TogglePlaylist, false),
            K::O if ctrl => (Action::OpenFiles { append: false }, false),
            K::F3 => (Action::OpenFiles { append: false }, false),
            K::F2 => (Action::OpenFolder { append: false }, false),
            K::U if ctrl => (Action::OpenUrl, false),
            K::V if ctrl && shift => (Action::OpenClipboard, false),
            K::F4 if alt => (Action::Quit, false),
            K::F4 => (Action::Stop, false),
            K::F5 => (Action::ShowPreferences(None), false),
            K::F6 => (Action::TogglePlaylist, false),
            K::F7 if alt => (Action::ShowControlPanel(None), false),
            K::F1 if ctrl => (Action::ShowMediaInfo, false),
            K::F1 => (Action::ShowAbout, false),
            K::Tab => (Action::ToggleInfo, false),
            K::E if ctrl && alt => (Action::Screenshot("subtitles", false), false),
            K::E if ctrl => (Action::Screenshot("video", false), false),
            K::C if ctrl && alt => (Action::Screenshot("subtitles", true), false),
            K::C if ctrl => (Action::Screenshot("video", true), false),
            K::T if ctrl => (Action::CycleOnTop, false),
            // Window scale moved to Alt+digits so the bare digits stay free for the
            // pan & scan grid, matching the shortcut labels the menus advertise.
            K::Num1 if alt => (Action::SetWindowScale(50), false),
            K::Num2 if alt => (Action::SetWindowScale(100), false),
            K::Num3 if alt => (Action::SetWindowScale(150), false),
            K::Num4 if alt => (Action::SetWindowScale(200), false),
            K::Num9 if none => (Action::Zoom(0.1), true),
            K::Num1 if none => (Action::Zoom(-0.1), true),
            K::Num4 if none => (Action::Pan(-0.02, 0.0), true),
            K::Num6 if none => (Action::Pan(0.02, 0.0), true),
            K::Num8 if none => (Action::Pan(0.0, -0.02), true),
            K::Num2 if none => (Action::Pan(0.0, 0.02), true),
            K::Num5 if none => (Action::ResetGeometry, false),
            K::W if ctrl => (Action::Close, false),
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
            K::Z if ctrl && alt && shift => (Action::RedoPlaylist, false),
            K::Z if ctrl && alt => (Action::UndoPlaylist, false),
            K::Z if ctrl => (Action::ToggleFlip("hflip"), false),
            K::V if ctrl => (Action::ToggleFlip("vflip"), false),
            K::Q if ctrl => (Action::Quit, false),
            // Hardware media keys arrive as F7/F8/F9 when "Use F1-F12 as
            // standard function keys" is enabled (or with fn held).
            // With the Now Playing integration enabled, the system media keys
            // are handled by MPRemoteCommandCenter — honoring them here too
            // would double-toggle.
            K::F7 if !s.now_playing => (Action::PrevFile, false),
            K::F8 if !s.now_playing => (Action::TogglePause, false),
            K::F9 if !s.now_playing => (Action::NextFile, false),
            _ => return None,
        };
        Some(action)
    }

    // -- main frame --------------------------------------------------------------------------------------

    fn frame(&mut self, ctx: &Context, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.closing {
            return;
        }
        self.now = self.started_at.elapsed().as_secs_f64();
        self.osd.tick();
        if !self.position_checked {
            self.position_checked = true;
            self.recover_offscreen_window(ctx);
        }

        if let Some(error) = self.startup_error.clone() {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                ui.heading("MacPlayer could not start");
                ui.add_space(12.0);
                ui.label(error);
                ui.add_space(16.0);
                if ui.button("Exit").clicked() {
                    self.request_close(ctx);
                }
            });
            ctx.request_repaint_after(Duration::from_millis(250));
            return;
        }

        // Pull mpv events.
        let messages: Vec<EngineMsg> = self.engine().map(|e| e.poll()).unwrap_or_default();
        for message in messages {
            self.on_engine_event(message);
        }

        if self.resume_dialog.open {
            let window = ctx.content_rect();
            let layout = self.layout(window);
            self.paint_video(ui, &layout);
            self.paint_chrome(ctx, ui, &layout);
            // Enter confirms the resume; Escape cancels it and starts over, so the
            // dialog is fully usable from the keyboard.
            let keyboard = ctx.input(|input| {
                if input.key_pressed(egui::Key::Enter) {
                    Some(ResumeChoice::Resume)
                } else if input.key_pressed(egui::Key::Escape) {
                    Some(ResumeChoice::StartOver)
                } else {
                    None
                }
            });
            if let Some(choice) = keyboard.or_else(|| self.resume_dialog.show(ctx)) {
                self.apply_resume_choice(choice);
            }
            // The chrome is still painted and clickable behind this dialog:
            // keep playback-relevant clicks for the next frame, drop the rest
            // (file opens etc. would race the resume choice).
            let pending = std::mem::take(&mut self.pending);
            for action in pending.actions {
                if matches!(
                    action,
                    Action::TogglePause
                        | Action::SeekRel(_)
                        | Action::SeekAbs(_)
                        | Action::SeekBar(..)
                        | Action::VolumeStep(_)
                        | Action::SetVolume(_)
                        | Action::ToggleMute
                        | Action::FrameStep(_)
                        | Action::ToggleRemaining
                ) {
                    self.pending.actions.push(action);
                }
            }
            ctx.request_repaint_after(Duration::from_millis(80));
            return;
        }

        self.handle_escape(ctx);

        while let Ok(message) = self.ipc_rx.try_recv() {
            match message {
                crate::IpcMessage::Open(paths) => {
                    if !paths.is_empty() {
                        self.pending.open_paths(paths, false);
                    }
                }
                crate::IpcMessage::SetSetting { key, value } => {
                    let map = serde_json::Map::from_iter([(key.clone(), value.clone())]);
                    match self.settings.apply_patch(map) {
                        crate::settings::PatchResult::Rejected => {
                            self.osd_error(format!("could not apply {key}"), 2500);
                        }
                        verdict => {
                            self.apply_runtime_setting(ctx, &key, &value);
                            self.save_settings();
                            if verdict == crate::settings::PatchResult::Applied {
                                self.osd(format!("{key} updated"));
                            }
                        }
                    }
                }
                crate::IpcMessage::ResetSettings => {
                    // "Reset" means data + data-driven settings: the user's
                    // feature switches and appearance survive.
                    let (continue_watching, cover_art, now_playing, update_check) = (
                        self.settings.continue_watching,
                        self.settings.cover_art,
                        self.settings.now_playing,
                        self.settings.update_check,
                    );
                    let (hover_thumbnails, row_previews, accent, accent_custom, language) = (
                        self.settings.hover_thumbnails,
                        self.settings.row_previews,
                        self.settings.accent.clone(),
                        self.settings.accent_custom.clone(),
                        self.settings.language.clone(),
                    );
                    self.settings = crate::settings::Settings::default();
                    self.settings.continue_watching = continue_watching;
                    self.settings.cover_art = cover_art;
                    self.settings.now_playing = now_playing;
                    self.settings.update_check = update_check;
                    self.settings.hover_thumbnails = hover_thumbnails;
                    self.settings.row_previews = row_previews;
                    self.settings.accent = accent;
                    self.settings.accent_custom = accent_custom;
                    self.settings.language = language;
                    self.apply_appearance(ctx);
                    self.apply_settings(true);
                    self.playlist.clear();
                    self.sync_playlist_settings();
                    self.save_settings();
                    self.osd("Settings reset to defaults");
                }
                crate::IpcMessage::PlaylistAdd(paths) => {
                    let first = self.playlist.add(&paths);
                    self.sync_playlist_settings();
                    self.save_settings();
                    self.update_title();
                    let _ = first;
                    self.osd(format!(
                        "{} {}",
                        paths.len(),
                        i18n::tr("files to the playlist")
                    ));
                }
                crate::IpcMessage::SaveNow => self.save_settings(),
                crate::IpcMessage::Clear(collection) => match collection.as_str() {
                    "history" => self.settings.clear_history(),
                    "recent" => self.settings.recent.clear(),
                    "positions" => self.settings.positions.clear(),
                    "bookmarks" => self.settings.bookmarks.clear(),
                    "playlist" => {
                        self.playlist.clear();
                        self.sync_playlist_settings();
                    }
                    _ => {}
                },
            }
        }

        // Folder-expansion answers from the worker thread.
        while let Ok((generation, expanded, append)) = self.expanded_rx.try_recv() {
            if generation == self.open_generation {
                self.expander_pending = None;
                self.open_expanded(&expanded, append);
            }
        }
        // Now Playing remote commands (system media keys / Control Center).
        if let Some(rx) = &self.now_playing_rx {
            while let Ok(command) = rx.try_recv() {
                match command {
                    crate::nowplaying::NowPlayingCommand::TogglePlayPause => {
                        self.pending.push(Action::TogglePause);
                    }
                    crate::nowplaying::NowPlayingCommand::NextTrack => {
                        self.pending.push(Action::NextFile);
                    }
                    crate::nowplaying::NowPlayingCommand::PreviousTrack => {
                        self.pending.push(Action::PrevFile);
                    }
                    crate::nowplaying::NowPlayingCommand::SeekTo(position) => {
                        self.pending.push(Action::SeekAbs(position.max(0.0)));
                    }
                }
            }
        }
        // Startup update check result.
        if let Some(rx) = &self.update_rx {
            while let Ok(tag) = rx.try_recv() {
                if self.update_available.as_deref() != Some(tag.as_str()) {
                    self.update_available = Some(tag);
                }
            }
        }
        // Throttled Now Playing publish (once per second while anything is open).
        if let Some(now_playing) = &self.now_playing
            && self.now - self.now_playing_last >= 1.0
        {
            self.now_playing_last = self.now;
            if self.has_file() {
                let buffering = self
                    .state
                    .get("paused-for-cache")
                    .map(|v| v.as_bool())
                    .unwrap_or(false);
                let title = self
                    .state
                    .get("media-title")
                    .and_then(|v| v.as_str().map(str::to_string))
                    .or_else(|| {
                        self.current_path
                            .as_deref()
                            .map(|path| crate::util::title_of(path).to_string())
                    })
                    .unwrap_or_else(|| "MacPlayer".into());
                // Buffering keeps the Playing card (so the controls stay
                // live) but reports rate 0, matching what is on screen.
                let playing = self.is_playing() || buffering;
                let rate = if self.is_playing() {
                    self.state
                        .get("speed")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0)
                } else {
                    0.0
                };
                now_playing.update(&crate::nowplaying::NowPlayingInfo {
                    title,
                    duration: self.duration(),
                    elapsed: self.position(),
                    playing,
                    rate,
                });
            } else if !self.now_playing_cleared {
                now_playing.clear();
                self.now_playing_cleared = true;
            }
        }
        if self.has_file() {
            // The card was wiped while idle; re-arm for the next file.
            self.now_playing_cleared = false;
        }
        // Still waiting on the expander? Announce it once, after a beat.
        if let Some((generation, started, shown)) = self.expander_pending
            && !shown
            && self.now - started > 0.15
        {
            if let Some((_, _, shown_ref)) = self.expander_pending.as_mut() {
                *shown_ref = true;
            }
            if generation == self.open_generation {
                self.osd(i18n::tr("Reading folder…"));
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
        self.sleep_timer_tick();
        self.thumbs.drain();
        // Theme "auto": egui resolves the system appearance; the hand-drawn
        // chrome reads the skin's atomic, so reconcile it every frame or the
        // two palettes drift apart.
        if self.settings.theme == "auto" {
            let resolved = match ctx.theme() {
                egui::Theme::Light => crate::skin::Theme::Light,
                _ => crate::skin::Theme::Dark,
            };
            if skin::current_theme() != resolved {
                skin::set_theme(resolved);
                ctx.request_repaint();
            }
        }
        let new_viewport_h = ctx.content_rect().height().max(1.0);
        if (self.viewport_h - new_viewport_h).abs() > 0.5 {
            self.viewport_h = new_viewport_h;
            self.update_sub_margin_for_height(new_viewport_h);
        }
        if self.info_visible && self.now >= self.next_info {
            self.next_info = self.now + 0.4;
            let text = self.info_text(true);
            self.osd.info = text;
        }
        self.update_chrome(ctx);
        // Eased drawer slide: drives both the drawer rect and how much of the
        // right-hand floating chrome is reserved for it (see `layout`).
        self.drawer_slide =
            ctx.animate_bool_with_time(Id::new("drawer-slide"), self.playlist_visible, 0.22);
        // Live progress of the playing file for the playlist row marker.
        let duration = self.duration();
        self.playlist.progress = if self.file_ready && duration > 0.0 {
            progress_fraction(self.position() / duration)
        } else {
            None
        };

        // Deferred autofit.
        if self.next_autofit > 0.0 && self.now >= self.next_autofit {
            self.next_autofit = 0.0;
            if self.settings.autofit && !self.file_is_audio_only {
                self.fit_window(None, ctx);
            }
        }

        self.handle_keyboard(ctx);
        self.handle_dropped_files(ctx);

        let window = ctx.content_rect();
        let layout = self.layout(window);
        self.handle_gestures(ctx, &layout);
        self.remember_window_geometry(ctx);

        // The video fills the window; chrome, drawer and panels float above it.
        self.paint_video(ui, &layout);
        if self.closing {
            return;
        }
        self.paint_chrome(ctx, ui, &layout);

        self.handle_menus(ui);
        self.handle_dialogs(ctx);

        // Drain the action queue built this frame.
        let pending = std::mem::take(&mut self.pending);
        for (paths, append) in pending.open {
            self.open_paths(&paths, append);
        }
        for action in pending.actions {
            self.dispatch(action, ctx);
        }

        // Keep the window level in sync — but only send the viewport command when the
        // value actually changed, so winit does not re-apply the level every frame.
        let want_on_top = self.mini_player || (self.on_top && !self.fullscreen);
        if self.applied_on_top != Some(want_on_top) {
            self.applied_on_top = Some(want_on_top);
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if want_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
        }

        let title = if self.title_text.is_empty() {
            "MacPlayer".to_string()
        } else {
            format!("{} - MacPlayer", self.title_text)
        };
        if self.applied_title != title {
            self.applied_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        self.gl_ready = true;
        if !self.pending_open.is_empty() {
            let pending_open = std::mem::take(&mut self.pending_open);
            for (paths, append) in pending_open {
                self.open_paths(&paths, append);
            }
        }

        // Repaint cadence is the app's main power lever. While playing, mpv's
        // frame callback already requests one repaint per decoded frame, so a
        // slow safety net is enough — a fixed 16 ms timer would repaint a 24 fps
        // video at 60 fps. When paused the UI is static: repaint only for the
        // pending OSD fade, active buffering or a coarse keepalive.
        let buffering = self
            .state
            .get("paused-for-cache")
            .map(|v| v.as_bool())
            .unwrap_or(false);
        let message_remaining_ms = self
            .osd
            .message_until
            .map(|until| until.saturating_duration_since(Instant::now()).as_millis() as u64);
        let flash_remaining_ms = (self.flash_until > self.now)
            .then(|| ((self.flash_until - self.now) * 1000.0).max(1.0) as u64);
        let interval = repaint_interval_ms(
            self.is_playing(),
            buffering,
            message_remaining_ms,
            flash_remaining_ms,
        );
        ctx.request_repaint_after(Duration::from_millis(interval));
    }

    fn any_dialog_open(&self) -> bool {
        self.goto_dialog.open
            || self.url_dialog.open
            || self.text_dialog.open
            || self.prefs.open
            || self.panel.open
            || self.history_dialog.open
    }

    fn close_top_dialog(&mut self) {
        if self.text_dialog.open {
            self.text_dialog.open = false;
        } else if self.goto_dialog.open {
            self.goto_dialog.open = false;
        } else if self.url_dialog.open {
            self.url_dialog.open = false;
        } else if self.prefs.open {
            self.prefs.open = false;
        } else if self.panel.open {
            self.panel.open = false;
        } else if self.history_dialog.open {
            self.history_dialog.open = false;
        }
    }

    fn handle_escape(&mut self, ctx: &Context) {
        if !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return;
        }
        if self.palette.open {
            self.palette.close();
            return;
        }
        if self.settings_pop.open {
            self.settings_pop.open = false;
            return;
        }
        // The drawer behaves like a lightweight panel: close it before the generic
        // focus/fullscreen fallbacks, even when its list does not hold focus.
        if self.playlist_visible
            && !self.menu.is_open()
            && !self.playlist.menu_open()
            && !self.any_dialog_open()
        {
            self.toggle_playlist(ctx);
            return;
        }
        match escape_target(
            self.menu.is_open() || self.playlist.menu_open(),
            self.any_dialog_open(),
            self.playlist.has_focus,
            self.fullscreen,
        ) {
            EscapeTarget::Menu => {
                self.menu.close();
                self.track_menu_kind = None;
                self.playlist.close_menu();
            }
            EscapeTarget::Dialog => self.close_top_dialog(),
            EscapeTarget::Playlist => self.playlist.clear_focus(),
            EscapeTarget::Fullscreen => self.pending.push(Action::ExitFullscreen),
            EscapeTarget::None => {}
        }
    }

    fn handle_keyboard(&mut self, ctx: &Context) {
        // Modals own the keyboard while they are visible.
        if self.palette.open
            || self.settings_pop.open
            || self.menu.is_open()
            || self.playlist.menu_open()
            || self.any_dialog_open()
            || ctx.egui_wants_keyboard_input()
        {
            return;
        }
        let playlist_keys = self.playlist.has_focus;
        // Borrow the event list in place — no per-frame clone of every input event.
        // Closure captures stay disjoint per field, so the &mut borrow is fine.
        ctx.input(|i| {
            for event in &i.events {
                let egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = event
                else {
                    continue;
                };
                if !pressed {
                    continue;
                }
                // The focused playlist owns list navigation keys.
                if playlist_keys
                    && matches!(
                        key,
                        egui::Key::ArrowUp
                            | egui::Key::ArrowDown
                            | egui::Key::Home
                            | egui::Key::End
                            | egui::Key::PageUp
                            | egui::Key::PageDown
                            | egui::Key::Enter
                            | egui::Key::Delete
                    )
                {
                    continue;
                }
                let mapped = self.keymap(*key, *modifiers).or_else(|| {
                    physical_key.and_then(|physical| self.keymap(physical, *modifiers))
                });
                if let Some((action, allows_repeat)) = mapped {
                    if *repeat && !allows_repeat {
                        continue;
                    }
                    self.pending.push(action);
                }
            }
        });
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
                self.osd.center = i18n::tr("Drop to load subtitle").into();
            } else if self.drop_appends(ctx) {
                self.osd.center = i18n::tr("Drop to add to playlist").into();
            } else {
                self.osd.center = i18n::tr("Drop to play").into();
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

    /// Reveal the floating chrome on pointer activity; hide it (with the cursor) while
    /// playback runs untouched. Panels pin the chrome open.
    fn update_chrome(&mut self, ctx: &Context) {
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        if pointer != self.last_pointer {
            self.last_pointer = pointer;
            if pointer.is_some() {
                self.schedule_chrome_hide();
            }
        }
        let was_visible = self.chrome_visible;
        if !self.is_playing() || self.panels_open() {
            self.chrome_visible = true;
            self.chrome_hide_at = f64::INFINITY;
        } else {
            // Playback started without pointer activity (e.g. after resume): start the
            // hide countdown once, so the chrome still fades away on its own.
            if self.chrome_hide_at == f64::INFINITY {
                self.chrome_hide_at = self.now + CHROME_HIDE_SECS;
            }
            if self.now >= self.chrome_hide_at {
                self.chrome_visible = false;
            }
        }
        if was_visible != self.chrome_visible {
            self.update_sub_margin();
        }
        apply_cursor(ctx, self.chrome_hidden());
    }

    /// Anything that should keep the floating chrome pinned open.
    fn panels_open(&self) -> bool {
        self.playlist_visible
            || self.settings_pop.open
            || self.menu.is_open()
            || self.track_menu_kind.is_some()
            || self.playlist.menu_open()
            || self.any_dialog_open()
            || self.palette.open
            || self.resume_dialog.open
            || self.mini_player
    }

    fn schedule_chrome_hide(&mut self) {
        self.chrome_visible = true;
        self.chrome_hide_at = if self.is_playing() && !self.panels_open() {
            self.now + CHROME_HIDE_SECS
        } else {
            f64::INFINITY
        };
    }

    fn chrome_hidden(&self) -> bool {
        self.is_playing() && !self.panels_open() && self.now >= self.chrome_hide_at
    }

    /// Floating chrome over the video: drawer, scrims, top-right actions, control bar.
    /// The controls and top actions fade in and out with pointer activity instead of
    /// snapping; the drawer slides separately (see `layout`).
    fn paint_chrome(&mut self, ctx: &Context, ui: &mut Ui, layout: &Layout) {
        let colors = skin::active_colors();
        let window = layout.video;
        let chrome_alpha =
            ctx.animate_bool_with_time(Id::new("chrome-fade"), self.chrome_visible, 0.18);

        // Bottom scrim + control bar.
        if chrome_alpha > 0.004
            && let Some(controls) = layout.controls
        {
            let scrim_top_y = (controls.top() - 80.0).max(window.top());
            skin::v_gradient(
                ui.painter(),
                Rect::from_min_max(pos2(window.left(), scrim_top_y), window.max),
                Color32::TRANSPARENT,
                colors.scrim_bottom.gamma_multiply(chrome_alpha),
            );
            self.paint_controls(ui, controls, chrome_alpha);
        }

        // Top scrim + floating actions.
        if chrome_alpha > 0.004
            && let Some(actions) = layout.top_actions
        {
            let scrim_bottom_y = (actions.bottom() + 56.0).min(window.bottom());
            skin::v_gradient(
                ui.painter(),
                Rect::from_min_max(window.min, pos2(window.right(), scrim_bottom_y)),
                colors.scrim_top.gamma_multiply(chrome_alpha),
                Color32::TRANSPARENT,
            );
            self.paint_top_actions(ui, window, actions, chrome_alpha);
        }

        // The drawer paints last, above every floating control (approved mockup
        // z-order); it brings its own close affordances (header ✕, Escape, ⌘P).
        if let Some(rect) = layout.playlist {
            self.paint_drawer(ctx, ui, rect);
        }
    }

    /// Drawer toggle + settings gear over the video's top edge; in full screen the
    /// native title bar is gone, so the media title is shown on the left.
    fn paint_top_actions(&mut self, ui: &mut Ui, window: Rect, rect: Rect, alpha: f32) {
        let colors = skin::active_colors();
        if self.fullscreen && !self.title_text.is_empty() && alpha > 0.05 {
            let painter = ui.painter_at(window);
            painter.text(
                pos2(window.left() + 16.0, rect.center().y),
                Align2::LEFT_CENTER,
                skin::elide_middle(&painter, &self.title_text, &skin::font(12.0, false), 340.0),
                skin::font(12.0, false),
                colors.text_bright.gamma_multiply(alpha),
            );
        }
        let btn = 32.0;
        let drawer_rect = Rect::from_min_size(pos2(rect.left(), rect.top()), vec2(btn, btn));
        let gear_rect =
            Rect::from_min_size(pos2(rect.left() + btn + 4.0, rect.top()), vec2(btn, btn));
        let drawer = IconBtn::new("playlist")
            .flat(true)
            .alpha(alpha)
            .tip(i18n::tr("Playlist"))
            .checked(self.playlist_visible)
            .ui(ui, ui.id().with("tb-drawer"), drawer_rect);
        if drawer.primary {
            self.pending.push(Action::TogglePlaylist);
        }
        let gear = IconBtn::new("gear")
            .flat(true)
            .alpha(alpha)
            .tip(i18n::tr("Settings"))
            .checked(self.settings_pop.open)
            .ui(ui, ui.id().with("tb-gear"), gear_rect);
        if gear.primary {
            self.settings_pop.open = !self.settings_pop.open;
            if self.settings_pop.open {
                self.settings_pop.just_opened = true;
            }
            self.settings_anchor = gear_rect;
            self.schedule_chrome_hide();
        }
    }

    /// Floating control bar: seek row on top, transport row below. `alpha` fades the
    /// whole bar while the chrome animates in or out.
    fn paint_controls(&mut self, ui: &mut Ui, rect: Rect, alpha: f32) {
        let colors = skin::active_colors();
        let seek_rect = Rect::from_min_max(
            pos2(rect.left() + 14.0, rect.top() + 3.0),
            pos2(
                (rect.right() - 14.0).max(rect.left() + 1.0),
                rect.top() + 25.0,
            ),
        );
        let row = Rect::from_min_max(
            pos2(rect.left() + 8.0, rect.top() + 27.0),
            pos2(rect.right() - 8.0, rect.bottom()),
        );

        // Seek bar (widget handles drag, wheel, chapters, A-B).
        let now = Instant::now();
        let out = seek_bar(ui, seek_rect, Id::new("seek"), &mut self.seek, now, alpha);
        if let Some((seconds, exact)) = out.seek {
            self.pending.push(Action::SeekBar(seconds, exact));
        }
        if let Some(seconds) = out.hover
            && seconds >= 0.0
        {
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
            // Hover preview thumbnail above the time bubble, when ready.
            let duration = self.duration();
            let ctx = ui.ctx().clone();
            let thumb = self
                .current_path
                .as_deref()
                .filter(|path| !is_url(path))
                .filter(|_| self.settings.hover_thumbnails)
                .and_then(|path| self.thumbs.texture(&ctx, path, seconds, duration))
                .map(|(handle, _)| {
                    let size = handle.size_vec2();
                    let scale = 128.0 / size.x.max(1.0);
                    (handle.id(), vec2(size.x * scale, size.y * scale))
                });
            let bubble = Rect::from_min_size(
                pos2(
                    (seek_rect.center().x - w / 2.0)
                        .clamp(rect.left() + 2.0, rect.right() - w - 2.0),
                    seek_rect.top() - 22.0 - thumb.as_ref().map_or(0.0, |(_, size)| size.y + 4.0),
                ),
                vec2(w, 20.0),
            );
            if let Some((tex, size)) = thumb {
                let thumb_rect = Rect::from_min_size(
                    pos2(
                        bubble.center().x - size.x / 2.0,
                        bubble.top() - size.y - 4.0,
                    ),
                    size,
                );
                let tp = ui.painter_at(thumb_rect.expand(1.0));
                let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                tp.image(
                    tex,
                    thumb_rect,
                    uv,
                    colors.text_bright.gamma_multiply(alpha),
                );
                tp.rect_stroke(
                    thumb_rect,
                    4.0,
                    Stroke::new(1.0, colors.dialog_line.gamma_multiply(alpha)),
                    egui::StrokeKind::Outside,
                );
            }
            let bp = ui.painter_at(bubble);
            bp.rect_filled(bubble, 6.0, colors.dialog_bg.gamma_multiply(alpha));
            bp.rect_stroke(
                bubble,
                6.0,
                Stroke::new(1.0, colors.dialog_line.gamma_multiply(alpha)),
                egui::StrokeKind::Outside,
            );
            bp.text(
                bubble.center(),
                egui::Align2::CENTER_CENTER,
                text,
                font_id,
                colors.text_bright.gamma_multiply(alpha),
            );
        }
        if out.wheel != 0 {
            let step = self.settings.seek_small;
            self.pending.push(Action::SeekRel(out.wheel as f64 * step));
        }

        // Transport row: back / play / forward / time … subs, audio, volume, fullscreen.
        let bw = 36.0;
        let back_r = Rect::from_min_size(pos2(row.left(), row.top()), vec2(bw, row.height()));
        let play_r = Rect::from_min_size(
            pos2(back_r.right() + 2.0, row.top()),
            vec2(bw + 8.0, row.height()),
        );
        let fwd_r = Rect::from_min_size(
            pos2(play_r.right() + 2.0, row.top()),
            vec2(bw, row.height()),
        );
        let playing = self.is_playing();
        let back = IconBtn::new("rewind")
            .flat(true)
            .alpha(alpha)
            .tip(i18n::tr("Seek back"))
            .ui(ui, ui.id().with("ctl-back"), back_r);
        if back.primary {
            self.pending
                .push(Action::SeekRel(-self.settings.seek_small));
        }
        let play = IconBtn::new(if playing { "pause" } else { "play" })
            .flat(true)
            .alpha(alpha)
            .tip(if playing {
                i18n::tr("Pause")
            } else {
                i18n::tr("Play")
            })
            .ui(ui, ui.id().with("ctl-play"), play_r);
        if play.primary {
            self.pending.push(Action::TogglePause);
        }
        let fwd = IconBtn::new("fast_forward")
            .flat(true)
            .alpha(alpha)
            .tip(i18n::tr("Seek forward"))
            .ui(ui, ui.id().with("ctl-fwd"), fwd_r);
        if fwd.primary {
            self.pending.push(Action::SeekRel(self.settings.seek_small));
        }

        // Right cluster, anchored to the row's right edge. The cluster compacts in
        // narrow windows (drawer open, small window): the volume slider narrows,
        // then the track buttons drop out, then the whole slider goes away — the
        // time labels degrade against whatever is leftmost so nothing ever overlaps.
        let row_w = row.width();
        let cluster = control_cluster(row_w);
        let vol_w = if cluster.show_volume {
            cluster.vol_w
        } else {
            0.0
        };
        let fs_r = Rect::from_min_size(pos2(row.right() - bw, row.top()), vec2(bw, row.height()));
        let vol_r = Rect::from_min_size(
            pos2(fs_r.left() - vol_w, row.top()),
            vec2(vol_w, row.height()),
        );
        let audio_r =
            Rect::from_min_size(pos2(vol_r.left() - bw, row.top()), vec2(bw, row.height()));
        let subs_r =
            Rect::from_min_size(pos2(audio_r.left() - bw, row.top()), vec2(bw, row.height()));
        let cluster_left = if !cluster.show_volume {
            fs_r.left()
        } else if cluster.show_track_buttons {
            subs_r.left()
        } else {
            vol_r.left()
        };

        // Elapsed / total time: both values, then elapsed only, then nothing.
        // Clicking switches to remaining time.
        let painter = ui.painter_at(row);
        let (elapsed, total) = self.time_parts();
        let mono = skin::mono(10.0);
        let elapsed_w = skin::text_width(&painter, &elapsed, &mono);
        let total_text = format!("/ {total}");
        let total_w = skin::text_width(&painter, &total_text, &mono);
        let time_x = fwd_r.right() + 12.0;
        let gap = 10.0;
        let show_total = time_x + elapsed_w + 6.0 + total_w + gap <= cluster_left;
        let show_elapsed = time_x + elapsed_w + gap <= cluster_left;
        if !self.has_file() {
            // Nothing loaded: a quiet dash instead of doubled zeros.
            painter.text(
                pos2(time_x, row.center().y),
                Align2::LEFT_CENTER,
                "—",
                mono,
                colors.text_dim.gamma_multiply(alpha),
            );
        } else if show_elapsed {
            painter.text(
                pos2(time_x, row.center().y),
                Align2::LEFT_CENTER,
                elapsed,
                mono.clone(),
                colors.text_bright.gamma_multiply(alpha),
            );
            if show_total {
                painter.text(
                    pos2(time_x + elapsed_w + 6.0, row.center().y),
                    Align2::LEFT_CENTER,
                    total_text,
                    mono,
                    colors.text_dim.gamma_multiply(alpha),
                );
            }
            let time_w = if show_total {
                elapsed_w + 6.0 + total_w
            } else {
                elapsed_w
            };
            let time_rect =
                Rect::from_min_size(pos2(time_x, row.top()), vec2(time_w, row.height()));
            if ui
                .interact(time_rect, ui.id().with("ctl-time"), Sense::click())
                .on_hover_text(i18n::tr(
                    "Click to switch between elapsed and remaining time",
                ))
                .clicked()
            {
                self.pending.push(Action::ToggleRemaining);
            }
        }

        if cluster.show_track_buttons {
            let subs = IconBtn::new("subs")
                .flat(true)
                .alpha(alpha)
                .tip(i18n::tr("Subtitles"))
                .enabled(self.has_file())
                .ui(ui, ui.id().with("ctl-subs"), subs_r);
            if subs.primary {
                self.pending.push(Action::ShowTrackMenu(TrackKind::Sub));
            }
            let audio = IconBtn::new("note")
                .flat(true)
                .alpha(alpha)
                .tip(i18n::tr("Audio"))
                .enabled(self.has_file())
                .ui(ui, ui.id().with("ctl-audio"), audio_r);
            if audio.primary {
                self.pending.push(Action::ShowTrackMenu(TrackKind::Audio));
            }
        }
        if cluster.show_volume {
            let vol_out = volume_bar(ui, vol_r, Id::new("volume"), &mut self.volume, alpha);
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
        let fs = IconBtn::new(if self.fullscreen {
            "fullscreen_exit"
        } else {
            "fullscreen"
        })
        .flat(true)
        .alpha(alpha)
        .tip(i18n::tr("Fullscreen"))
        .ui(ui, ui.id().with("ctl-fs"), fs_r);
        if fs.primary {
            self.pending.push(Action::ToggleFullscreen);
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

    /// Translucent playlist drawer sliding over the video from the right.
    fn paint_drawer(&mut self, ctx: &Context, ui: &mut Ui, rect: Rect) {
        let colors = skin::active_colors();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, colors.drawer_fill);
        painter.line_segment(
            [
                pos2(rect.left() + 0.5, rect.top()),
                pos2(rect.left() + 0.5, rect.bottom()),
            ],
            Stroke::new(1.0, colors.dialog_line),
        );
        // Drag the drawer's left edge to widen/narrow it.
        let handle = Rect::from_min_max(
            pos2(rect.left() - 4.0, rect.top()),
            pos2(rect.left() + 4.0, rect.bottom()),
        );
        let resize = ui.interact(handle, Id::new("drawer-resize"), Sense::click_and_drag());
        if resize.dragged()
            && let Some(pos) = resize.interact_pointer_pos()
        {
            let window = ctx.content_rect();
            let width = (window.right() - pos.x)
                .clamp(MIN_PLAYLIST_W, DRAWER_MAX_W)
                .min(max_drawer_width(window.width()));
            self.settings.playlist_width = width as f64;
        }
        if resize.hovered() || resize.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resize.drag_stopped() {
            self.save_settings();
        }
        let ctx2 = ctx.clone();
        let features = crate::playlist::PlaylistFeatures {
            row_previews: self.settings.row_previews,
            cover_art: self.settings.cover_art,
        };
        let out = self
            .playlist
            .ui(ui, rect, &ctx2, &mut self.thumbs, features);
        self.apply_playlist_out(out, ctx);
    }

    // Calls into the unsafe render API at the mpv/GL boundary.
    #[allow(unsafe_code)]
    fn paint_video(&mut self, ui: &mut Ui, layout: &Layout) {
        let rect = layout.video;
        let idle = self
            .state
            .get("idle-active")
            .map(|v| v.as_bool())
            .unwrap_or(true);
        let audio_only = self.file_ready && !self.has_video();

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
                if renderer.error_changed(&format!("render context: {error}")) {
                    eprintln!("render context: {error}");
                }
                return;
            }
            match unsafe { renderer.render(gl, rect_px) } {
                Ok(()) => renderer.note_ok(),
                Err(error) => {
                    if renderer.error_changed(&format!("render: {error}")) {
                        eprintln!("render: {error}");
                    }
                }
            }
        });
        ui.painter().add(egui::epaint::PaintCallback {
            rect,
            callback: Arc::new(callback),
        });

        let colors = skin::active_colors();
        let painter = ui.painter_at(rect);

        // Right-click anywhere on the video opens the main context menu; plain
        // click and double-click are handled further down. Registered BEFORE the
        // idle-screen widgets so egui's hit test (last widget wins ties) gives the
        // idle play affordance precedence over this full-window rect.
        let response = ui.interact(rect, Id::new("video"), Sense::click());
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.video_click_at = None;
            self.menu.open_at(pos);
            self.track_menu_kind = None;
        }

        if idle && !self.file_ready {
            // Idle backdrop: a soft four-corner gradient with a faint dot grid —
            // the palette for both ships in the skin (`logo_grad_*`, `logo_dot`).
            let mut backdrop = egui::Mesh::default();
            backdrop.colored_vertex(rect.left_top(), colors.logo_grad_tl);
            backdrop.colored_vertex(rect.right_top(), colors.logo_grad_tr);
            backdrop.colored_vertex(rect.right_bottom(), colors.logo_grad_br);
            backdrop.colored_vertex(rect.left_bottom(), colors.logo_grad_bl);
            backdrop.colored_vertex(rect.center(), colors.logo_grad_center);
            backdrop.add_triangle(0, 4, 3);
            backdrop.add_triangle(3, 4, 2);
            backdrop.add_triangle(2, 4, 1);
            backdrop.add_triangle(1, 4, 0);
            painter.add(egui::Shape::Mesh(Arc::new(backdrop)));
            let spacing = 30.0;
            let offset_x = (rect.center().x - rect.left()) % spacing;
            let offset_y = (rect.center().y - rect.top()) % spacing;
            let mut dot_y = rect.top() + offset_y;
            while dot_y <= rect.bottom() {
                let mut dot_x = rect.left() + offset_x;
                while dot_x <= rect.right() {
                    painter.circle_filled(pos2(dot_x, dot_y), 1.0, colors.logo_dot);
                    dot_x += spacing;
                }
                dot_y += spacing;
            }

            // Centered composition: wordmark, play affordance, two hint lines.
            let center = rect.center();
            painter.text(
                pos2(center.x, center.y - 80.0),
                egui::Align2::CENTER_TOP,
                "M A C P L A Y E R",
                skin::font(11.0, false),
                colors.text_dim,
            );

            // The play affordance: fixed hit area, gently growing circle on hover.
            let hit = Rect::from_center_size(pos2(center.x, center.y - 14.0), vec2(96.0, 96.0));
            let response = ui
                .interact(hit, Id::new("idle-play"), Sense::click())
                .on_hover_text(i18n::tr("Play"));
            let hover = ui.ctx().animate_bool_with_time(
                Id::new("idle-play-hover"),
                response.hovered(),
                0.16,
            );
            let circle_center = pos2(center.x, center.y - 14.0);
            let radius = 42.0 + hover * 5.0;
            let fill_alpha = 12.0 + hover * 16.0;
            // Tint follows the theme: translucent white on the dark gradient,
            // translucent black on the light one.
            let tint = if skin::current_theme() == Theme::Light {
                (0, 0, 0)
            } else {
                (255, 255, 255)
            };
            let fill = Color32::from_rgba_unmultiplied(tint.0, tint.1, tint.2, fill_alpha as u8);
            painter.circle_filled(circle_center, radius, fill);
            painter.circle_stroke(
                circle_center,
                radius,
                Stroke::new(1.0 + hover, colors.logo.gamma_multiply(0.35 + 0.65 * hover)),
            );
            // Big play triangle, optically centred (mass sits slightly left).
            let w = radius * 0.42;
            let h = radius * 0.56;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(circle_center.x - w * 0.55, circle_center.y - h),
                    pos2(circle_center.x - w * 0.55, circle_center.y + h),
                    pos2(circle_center.x + w * 0.9, circle_center.y),
                ],
                colors.text_bright,
                Stroke::NONE,
            ));
            if response.clicked() {
                self.video_click_at = None;
                self.pending.push(Action::OpenFiles { append: false });
            }
            painter.text(
                pos2(center.x, center.y + 44.0),
                egui::Align2::CENTER_TOP,
                i18n::tr("Open a video"),
                skin::font(16.0, false),
                colors.text_bright,
            );
            painter.text(
                pos2(center.x, center.y + 72.0),
                egui::Align2::CENTER_TOP,
                i18n::tr("Drop a file here · ⌘O · or press play"),
                skin::font(12.0, false),
                colors.text_dim,
            );

            // Update-available pill (fed by the startup checker); pinned to
            // the top-right corner so it never collides with the card row.
            if let Some(tag) = &self.update_available {
                let pill = Rect::from_min_size(
                    pos2(rect.left() + 16.0, rect.top() + 14.0),
                    vec2(258.0, 26.0),
                );
                let pill_resp = ui
                    .interact(pill, Id::new("update-pill"), Sense::click())
                    .on_hover_text(i18n::tr("Download"));
                let p = ui.painter_at(pill);
                p.rect_filled(pill, 13.0, colors.panel);
                p.rect_stroke(
                    pill,
                    13.0,
                    Stroke::new(1.0, colors.logo),
                    egui::StrokeKind::Outside,
                );
                p.text(
                    pill.center(),
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{} {tag} · {}",
                        i18n::tr("New version available"),
                        i18n::tr("Download")
                    ),
                    skin::font(11.0, false),
                    colors.text_bright,
                );
                if pill_resp.clicked() {
                    self.services.open_url(crate::update::releases_page());
                }
            }

            // Continue-watching cards: partially watched recent files, one
            // click resumes (play_row restores the saved position). Only
            // drawn when there is room and the feature is enabled.
            if self.settings.continue_watching && rect.height() > 380.0 {
                let entries: Vec<&crate::settings::HistoryEntry> = self
                    .settings
                    .history
                    .iter()
                    .filter(|entry| {
                        entry.duration > 60.0
                            && entry.position > 15.0
                            && entry.position < entry.duration * 0.92
                            && self
                                .settings
                                .get_position(&entry.path)
                                .map(|saved| saved > 0.0)
                                .unwrap_or(false)
                    })
                    .take(6)
                    .collect();
                if !entries.is_empty() {
                    // Never let the row overflow narrow windows.
                    let max_cards = (((rect.width() - 40.0) / 160.0).floor() as usize).max(1);
                    let entries: Vec<_> = entries.into_iter().take(max_cards).collect();
                    let card_w = 148.0f32;
                    let card_h = 96.0f32;
                    let gap = 12.0f32;
                    let row_w = entries.len() as f32 * card_w
                        + (entries.len().saturating_sub(1)) as f32 * gap;
                    let row_left = center.x - row_w / 2.0;
                    let row_top = center.y + 104.0;
                    let label = i18n::tr("Continue watching");
                    painter.text(
                        pos2(center.x, row_top - 22.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        skin::font(11.0, false),
                        colors.text_dim,
                    );
                    for (index, entry) in entries.iter().enumerate() {
                        let card_rect = Rect::from_min_size(
                            pos2(row_left + index as f32 * (card_w + gap), row_top),
                            vec2(card_w, card_h),
                        );
                        let card = ui
                            .interact(
                                card_rect,
                                Id::new("continue-card").with(index),
                                Sense::click(),
                            )
                            .on_hover_text(&entry.title);
                        let hover = ui.ctx().animate_bool_with_time(
                            Id::new("continue-hover").with(index),
                            card.hovered(),
                            0.15,
                        );
                        let p = ui.painter_at(card_rect);
                        let art_rect =
                            Rect::from_min_size(card_rect.min, vec2(card_w, card_w * 9.0 / 16.0));
                        // Art: live frame at the resume position (video) or the
                        // embedded cover (audio); tinted block as fallback.
                        let mut drew_art = false;
                        if !is_url(&entry.path) {
                            if crate::util::is_audio(&entry.path) {
                                if self.settings.cover_art
                                    && let Some((handle, _)) =
                                        self.thumbs.cover(ui.ctx(), &entry.path)
                                {
                                    let size = handle.size_vec2();
                                    let scale =
                                        (art_rect.width() / size.x).min(art_rect.height() / size.y);
                                    let draw = Rect::from_center_size(
                                        art_rect.center(),
                                        vec2(size.x * scale, size.y * scale),
                                    );
                                    p.image(
                                        handle.id(),
                                        draw,
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                    drew_art = true;
                                }
                            } else {
                                self.thumbs.ensure(&entry.path, entry.duration.max(60.0));
                                if let Some((handle, _)) = self.thumbs.texture(
                                    ui.ctx(),
                                    &entry.path,
                                    entry.position,
                                    entry.duration,
                                ) {
                                    p.image(
                                        handle.id(),
                                        art_rect,
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                    drew_art = true;
                                }
                            }
                        }
                        if !drew_art {
                            p.rect_filled(art_rect, 6.0, colors.panel);
                            skin::draw_icon(
                                &p,
                                "play",
                                art_rect.center().x,
                                art_rect.center().y,
                                colors.logo,
                                colors.panel,
                            );
                        }
                        p.rect_stroke(
                            art_rect,
                            6.0,
                            Stroke::new(
                                1.0 + hover,
                                colors.logo.gamma_multiply(0.35 + 0.5 * hover),
                            ),
                            egui::StrokeKind::Outside,
                        );
                        // Resume-progress line under the art.
                        let track = Rect::from_min_max(
                            pos2(art_rect.left() + 2.0, art_rect.bottom() - 5.0),
                            pos2(art_rect.right() - 2.0, art_rect.bottom() - 3.0),
                        );
                        p.rect_filled(track, 1.0, colors.line);
                        let fraction = (entry.position / entry.duration).clamp(0.0, 1.0) as f32;
                        p.rect_filled(
                            Rect::from_min_max(
                                track.min,
                                pos2(track.left() + track.width() * fraction, track.max.y),
                            ),
                            1.0,
                            colors.accent,
                        );
                        let title = skin::elide(&p, &entry.title, &skin::font(11.0, false), card_w);
                        p.text(
                            pos2(card_rect.center().x, art_rect.bottom() + 10.0),
                            egui::Align2::CENTER_TOP,
                            title,
                            skin::font(11.0, false),
                            if card.hovered() {
                                colors.text_bright
                            } else {
                                colors.text
                            },
                        );
                        if card.clicked() {
                            self.video_click_at = None;
                            self.pending.open_paths(vec![entry.path.clone()], false);
                        }
                    }
                }
            }
        } else if audio_only {
            // Audio-only media: embedded cover art when available (and enabled),
            // otherwise a quiet disc mark — plus the title and the elapsed time.
            let center = rect.center();
            let art_size = 150.0f32.min(rect.height() * 0.34);
            let art_center = pos2(center.x, center.y - art_size * 0.42);
            let mut drew_cover = false;
            if self.settings.cover_art
                && let Some(path) = self.current_path.as_deref().filter(|p| !is_url(p))
                && let Some((handle, _)) = self.thumbs.cover(ui.ctx(), path)
            {
                let size = handle.size_vec2();
                let scale = (art_size / size.x).min(art_size / size.y);
                let draw = vec2(size.x * scale, size.y * scale);
                let draw_rect = Rect::from_center_size(art_center, draw);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(handle.id(), draw_rect, uv, Color32::WHITE);
                drew_cover = true;
            }
            if !drew_cover {
                let disc_r = art_size * 0.42;
                let disc = art_center;
                painter.circle_filled(disc, disc_r, colors.panel);
                painter.circle_stroke(disc, disc_r, Stroke::new(1.0, colors.line));
                painter.circle_stroke(disc, disc_r * 0.26, Stroke::new(1.0, colors.line));
                skin::draw_icon(&painter, "note", disc.x, disc.y, colors.logo, colors.panel);
            }
            let font_id = skin::font(15.0, false);
            let text = skin::elide(
                &painter,
                &self.title_text,
                &font_id,
                (rect.width() * 0.7).max(0.0),
            );
            painter.text(
                pos2(center.x, center.y + 16.0),
                egui::Align2::CENTER_TOP,
                text,
                font_id,
                colors.text_bright,
            );
            let dur = self.duration();
            if dur > 0.0 {
                painter.text(
                    pos2(center.x, center.y + 42.0),
                    egui::Align2::CENTER_TOP,
                    format!("{} / {}", fmt_time(self.position()), fmt_time(dur)),
                    skin::mono(11.0),
                    colors.text_dim,
                );
            }
        } else if self.now < self.flash_until {
            // Big play/pause flash, fading out.
            let t = ((self.flash_until - self.now) / FLASH_SECS).clamp(0.0, 1.0) as f32;
            let alpha = (t * 200.0) as u8;
            let radius = 34.0 + (1.0 - t) * 12.0;
            painter.circle_filled(
                rect.center(),
                radius,
                Color32::from_rgba_unmultiplied(12, 15, 22, (alpha / 2).min(120)),
            );
            skin::draw_icon(
                &painter,
                &self.flash_glyph,
                rect.center().x,
                rect.center().y,
                Color32::from_rgba_unmultiplied(255, 255, 255, alpha.max(40)),
                Color32::TRANSPARENT,
            );
        }

        // Video surface: a plain click toggles pause (or opens media when empty), a
        // double-click toggles full screen. The single click is deferred briefly so the
        // first press of a double-click never pauses the video.
        if let Some(first) = self.video_click_at
            && self.now - first > VIDEO_DOUBLE_CLICK_SECS
        {
            self.video_click_at = None;
            if self.has_file() {
                self.pending.push(Action::TogglePause);
            } else {
                self.pending.push(Action::OpenFiles { append: false });
            }
        }
        if response.double_clicked() {
            self.video_click_at = None;
            if self.has_file() {
                self.pending.push(Action::ToggleFullscreen);
            } else {
                self.pending.push(Action::OpenFiles { append: false });
            }
        } else if response.clicked() {
            self.video_click_at = Some(self.now);
        }

        // Drag-and-drop affordance: a faint wash plus an accent ring while files
        // hover over the window (the "Drop to play" OSD explains the action).
        if !self.hovered_files.is_empty() {
            painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, 6));
            painter.rect_stroke(
                rect.shrink(6.0),
                12.0,
                Stroke::new(2.0, colors.accent.gamma_multiply(0.85)),
                egui::StrokeKind::Inside,
            );
        }

        // OSD on top of the video.
        self.osd.paint(ui, rect);
    }

    fn handle_menus(&mut self, ui: &mut Ui) {
        if !self.menu.is_open() {
            return;
        }
        let entries = match self.track_menu_kind {
            Some(kind) => {
                let tracks = self.tracks().to_vec();
                menus::track_menu(kind, &tracks, self.has_file())
            }
            None => {
                let snapshot = self.menu_snapshot();
                menus::main_menu(&self.menu_context(&snapshot))
            }
        };
        let mut menu = std::mem::take(&mut self.menu);
        let action = menus::show(ui.ctx(), &mut menu, "main-menu", &entries);
        self.menu = menu;
        if let Some(action) = action {
            self.menu.close();
            self.track_menu_kind = None;
            self.pending.push(action);
        } else if !self.menu.is_open() {
            self.track_menu_kind = None;
        }
    }

    /// mpv state arrays the menu model needs; kept beside the borrowed [`MenuCtx`].
    fn menu_snapshot(&self) -> MenuSnapshot {
        let engine = self.engine();
        let state = &self.state;
        MenuSnapshot {
            tracks: engine
                .and_then(|e| e.get("track-list"))
                .map(|v| v.as_array().to_vec())
                .unwrap_or_default(),
            chapters: state
                .get("chapter-list")
                .map(|v| v.as_array().to_vec())
                .unwrap_or_default(),
            audio_devices: engine
                .and_then(|e| e.get("audio-device-list"))
                .map(|v| v.as_array().to_vec())
                .unwrap_or_default(),
        }
    }

    /// Live menu model snapshot, shared by the right-click menu and the command palette.
    fn menu_context<'a>(&'a self, snapshot: &'a MenuSnapshot) -> MenuCtx<'a> {
        let state = &self.state;
        MenuCtx {
            has_file: self.has_file(),
            playlist_count: self.playlist.count(),
            recent: &self.settings.recent,
            bookmarks: &self.settings.bookmarks,
            repeat: &self.settings.repeat_mode,
            loop_mode: &self.settings.loop_mode,
            sleep_minutes: match (self.sleep_deadline, self.sleep_after_playlist) {
                (_, true) => -1,
                (Some(deadline), _) => ((deadline - self.now) / 60.0).ceil() as i32,
                (None, false) => 0,
            },
            ytdl_quality: &self.settings.ytdl_quality,
            video_sync: &self.settings.video_sync,
            interpolation: self.settings.interpolation,
            scale_profile: &self.settings.scale_profile,
            filters: self.video_filters,
            visualizer: &self.settings.visualizer,
            mini_player: self.mini_player,
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
            tracks: &snapshot.tracks,
            secondary_sid: state.get("secondary-sid").and_then(|v| v.as_i64()),
            chapters: &snapshot.chapters,
            current_chapter: state.get("chapter").and_then(|v| v.as_i64()).unwrap_or(-1),
            ab: (self.seek.ab.0, self.seek.ab.1),
            audio_devices: &snapshot.audio_devices,
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
            can_undo_playlist: self.playlist.can_undo(),
            can_redo_playlist: self.playlist.can_redo(),
        }
    }

    /// The Ctrl+K command palette: a filterable, flat view over the menu tree.
    fn show_command_palette(&mut self, ctx: &Context) {
        if !self.palette.open {
            return;
        }
        let snapshot = self.menu_snapshot();
        let entries = menus::main_menu(&self.menu_context(&snapshot));
        let mut flat: Vec<(String, String, Action)> = Vec::new();
        menus::flatten_entries(&entries, "", &mut flat);

        let query = self.palette.query.trim().to_lowercase();
        let visible: Vec<(String, String, Action)> = flat
            .into_iter()
            .filter(|(path, _, _)| {
                query
                    .split_whitespace()
                    .all(|token| path.to_lowercase().contains(token))
            })
            .take(PALETTE_MAX_RESULTS)
            .collect();
        if self.palette.selected >= visible.len() {
            self.palette.selected = visible.len().saturating_sub(1);
        }

        let mut accept = false;
        ctx.input(|input| {
            if input.key_pressed(egui::Key::ArrowDown) {
                self.palette.selected =
                    (self.palette.selected + 1).min(visible.len().saturating_sub(1));
            }
            if input.key_pressed(egui::Key::ArrowUp) {
                self.palette.selected = self.palette.selected.saturating_sub(1);
            }
            accept = !visible.is_empty() && input.key_pressed(egui::Key::Enter);
        });

        let colors = active_colors();
        egui::Area::new(Id::new("command-palette"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_TOP, [0.0, 72.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.dialog_bg)
                    .stroke(Stroke::new(1.0, colors.dialog_line))
                    .corner_radius(10.0)
                    .outer_margin(6.0)
                    .inner_margin(8.0)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 2,
                        color: colors.shadow,
                    })
                    .show(ui, |ui| {
                        // Pin both ends: an Area's available width runs to the
                        // screen edge, so the rows would stretch otherwise.
                        ui.set_min_width(460.0);
                        ui.set_max_width(460.0);
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.palette.query)
                                .hint_text(
                                    egui::RichText::new(format!(
                                        "{}  (Ctrl+K)",
                                        i18n::tr("Search commands…")
                                    ))
                                    .color(colors.text_9a),
                                )
                                .desired_width(440.0)
                                .frame(egui::Frame::NONE),
                        );
                        if self.palette.focused {
                            response.request_focus();
                            self.palette.focused = false;
                        }
                        ui.add_space(4.0);
                        if visible.is_empty() {
                            ui.label(
                                egui::RichText::new(i18n::tr("No matching commands"))
                                    .size(12.0)
                                    .color(colors.text_dim),
                            );
                        }
                        for (index, (path, shortcut, _)) in visible.iter().enumerate() {
                            let selected = index == self.palette.selected;
                            // Full-width command row: breadcrumb left, shortcut hint right.
                            let row_w = (ui.available_width()).max(120.0);
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(row_w, 22.0), Sense::click());
                            let resp = ui.interact(
                                rect,
                                Id::new("palette-row").with(index),
                                Sense::click(),
                            );
                            let p = ui.painter_at(rect);
                            if selected {
                                p.rect_filled(
                                    rect,
                                    5.0,
                                    Color32::from_rgba_unmultiplied(
                                        colors.accent.r(),
                                        colors.accent.g(),
                                        colors.accent.b(),
                                        28,
                                    ),
                                );
                                // Accent marker on the active row.
                                p.rect_filled(
                                    Rect::from_min_size(
                                        pos2(rect.left(), rect.center().y - 7.0),
                                        vec2(2.5, 14.0),
                                    ),
                                    1.0,
                                    colors.accent,
                                );
                            } else if resp.hovered() {
                                p.rect_filled(rect, 5.0, colors.menu_sel);
                            }
                            let path_font = skin::font(13.0, false);
                            let short_font = skin::mono(10.0);
                            let shortcut_w = if shortcut.is_empty() {
                                0.0
                            } else {
                                skin::text_width(&p, shortcut, &short_font) + 14.0
                            };
                            let path_w = (rect.width() - shortcut_w - 18.0).max(40.0);
                            let label = skin::elide(&p, path, &path_font, path_w);
                            p.text(
                                pos2(rect.left() + 12.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                label,
                                path_font,
                                if selected {
                                    colors.text_bright
                                } else {
                                    colors.text
                                },
                            );
                            if !shortcut.is_empty() {
                                p.text(
                                    pos2(rect.right() - 12.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    shortcut,
                                    short_font,
                                    colors.text_dim,
                                );
                            }
                            if resp.hovered() && !selected {
                                self.palette.selected = index;
                            }
                            if resp.clicked() {
                                self.palette.selected = index;
                                accept = true;
                            }
                        }
                        ui.add_space(2.0);
                        ui.separator();
                        ui.label(
                            egui::RichText::new(i18n::tr("↑↓ navigate · ↵ run · esc close"))
                                .size(10.0)
                                .color(colors.text_dim),
                        );
                    });
            });

        if accept && let Some((_, _, action)) = visible.get(self.palette.selected) {
            self.pending.push(action.clone());
            self.palette.close();
        }
    }

    fn handle_dialogs(&mut self, ctx: &Context) {
        self.show_command_palette(ctx);
        let mut actions = Vec::new();
        self.settings_pop
            .show(ctx, self.settings_anchor, &mut self.settings, &mut actions);
        for action in actions {
            self.pending.push(action);
        }
        let duration = self.duration();
        let mut actions = Vec::new();
        self.goto_dialog.show(ctx, duration, &mut actions);
        self.url_dialog.show(ctx, &mut actions);
        let update_available = self.update_available.clone();
        self.prefs
            .show(ctx, update_available.as_deref(), &mut actions);
        {
            let mut panel_actions = Vec::new();
            let settings = &mut self.settings;
            self.panel.show(ctx, settings, &mut panel_actions);
            actions.extend(panel_actions);
        }
        {
            let mut history_actions = Vec::new();
            let history = self.settings.history.clone();
            self.history_dialog
                .show(ctx, &history, &mut history_actions);
            actions.extend(history_actions);
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
            // From the playlist the URL dialog opens in append mode, matching
            // the "Add Files..." / "Add Folder..." entries next to it.
            self.url_dialog.open(true);
        }
        if out.close {
            self.toggle_playlist(ctx);
        }
        if out.repeat {
            self.cycle_repeat();
        }
        if let Some(row) = out.show_in_folder {
            self.pending.push(Action::PlaylistShowInFolder(row));
        }
        if out.shuffle {
            self.pending.push(Action::ToggleShuffle);
        }
        if out.changed {
            self.sync_playlist_settings();
            self.update_title();
            self.save_settings();
        }
    }
}

impl eframe::App for Player {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame(&ctx, ui, frame);
    }

    // Tears down the GL-side renderer while the context is still current.
    #[allow(unsafe_code)]
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.closing = true;
        if let Some(now_playing) = self.now_playing.take() {
            now_playing.clear();
        }
        self.remember_position();
        self.sync_playlist_settings();
        self.settings.first_run = false;
        self.save_settings();
        if let Some(renderer) = self.renderer.take()
            && let Ok(mut renderer) = renderer.lock()
            && let Some(gl) = gl
        {
            unsafe { renderer.shutdown(gl) };
        }
        if let Some(mut engine) = self.engine.take() {
            engine.shutdown();
        }
    }
}

// -- layout -------------------------------------------------------------------------------

#[derive(Debug)]
struct Layout {
    /// The video surface — the whole window.
    video: Rect,
    /// Floating bottom control bar (seek row + transport row).
    controls: Option<Rect>,
    /// Floating top-right actions (drawer + settings).
    top_actions: Option<Rect>,
    /// Playlist drawer overlay.
    playlist: Option<Rect>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            video: Rect::NOTHING,
            controls: None,
            top_actions: None,
            playlist: None,
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

/// Clamp a playback rate into the supported range and snap it to two decimals, so
/// accumulated float noise from repeated +/- steps can never drift the displayed
/// speed away from what mpv actually plays.
fn normalize_speed(value: f64) -> f64 {
    if !value.is_finite() {
        return 1.0;
    }
    (value.clamp(0.2, 12.0) * 100.0).round() / 100.0
}

/// Next-repaint delay in milliseconds — the app's main power lever.
///
/// * Playing: mpv's frame callback already requests one repaint per decoded
///   frame; the 200 ms value is only a safety net for property-driven UI
///   (time text, progress) in case frame events stall.
/// * Buffering: the percentage readout must tick smoothly.
/// * OSD message pending: repaint fast enough for the 300 ms fade-out, never
///   past the deadline itself.
/// * Otherwise (paused / idle): the UI is static — a coarse 250 ms keepalive.
fn repaint_interval_ms(
    playing: bool,
    buffering: bool,
    message_remaining_ms: Option<u64>,
    flash_remaining_ms: Option<u64>,
) -> u64 {
    if playing {
        200
    } else if buffering {
        33
    } else if let Some(remaining) = message_remaining_ms {
        remaining.clamp(1, 33)
    } else if let Some(remaining) = flash_remaining_ms {
        // The big play/pause flash fades over 0.55 s — keep it smooth when
        // the surrounding UI is at the paused keepalive rate.
        remaining.clamp(1, 33)
    } else {
        250
    }
}

/// Right-cluster compaction of the transport row for a given row width.
#[derive(Debug, Clone, Copy)]
struct ControlCluster {
    /// Volume slider visible at all.
    show_volume: bool,
    /// Width of the volume slider when visible.
    vol_w: f32,
    /// Subtitle/audio track buttons visible.
    show_track_buttons: bool,
}

/// Transport geometry degrades in three steps: the volume slider narrows below
/// 500px, the track buttons drop out below 430px, and the slider disappears
/// below 360px — so back/play/forward/fullscreen never overlap anything.
fn control_cluster(row_w: f32) -> ControlCluster {
    ControlCluster {
        show_volume: row_w >= 360.0,
        vol_w: if row_w < 500.0 { 60.0 } else { 104.0 },
        show_track_buttons: row_w >= 430.0,
    }
}

fn home_dir() -> Option<String> {
    std::env::var("HOME").ok()
}

/// Body of the open-expander thread: expand request paths (following folders
/// recursively) and hand the result back to the UI thread.
fn expand_worker(
    requests: Receiver<(u64, Vec<String>, bool)>,
    results: Sender<(u64, Vec<String>, bool)>,
) {
    while let Ok((generation, paths, append)) = requests.recv() {
        let expanded = crate::util::expand_paths(&paths);
        if results.send((generation, expanded, append)).is_err() {
            break;
        }
    }
}

fn shellexpand(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}

fn media_extensions() -> Vec<String> {
    VIDEO_EXT
        .iter()
        .chain(AUDIO_EXT)
        .chain(PLAYLIST_EXT)
        .map(|extension| extension.trim_start_matches('.').to_string())
        .collect()
}

fn sub_extensions() -> Vec<String> {
    ["srt", "ass", "ssa", "sub", "vtt", "idx", "sup"]
        .iter()
        .map(|e| e.to_string())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EscapeTarget {
    Menu,
    Dialog,
    Playlist,
    Fullscreen,
    None,
}

/// Escape unwinds transient UI in a fixed order: menu, dialog, list focus, fullscreen.
fn escape_target(
    menu_open: bool,
    dialog_open: bool,
    playlist_focused: bool,
    fullscreen: bool,
) -> EscapeTarget {
    if menu_open {
        EscapeTarget::Menu
    } else if dialog_open {
        EscapeTarget::Dialog
    } else if playlist_focused {
        EscapeTarget::Playlist
    } else if fullscreen {
        EscapeTarget::Fullscreen
    } else {
        EscapeTarget::None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ControlCluster, EscapeTarget, control_cluster, escape_target, expand_worker,
        normalize_speed,
    };

    #[test]
    fn expander_worker_expands_folders_and_keeps_append_flag() {
        let (req_tx, req_rx) = std::sync::mpsc::channel();
        let (res_tx, res_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || expand_worker(req_rx, res_tx));
        let dir = std::env::temp_dir().join(format!("mp-expander-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clip.mkv"), b"x").unwrap();
        req_tx
            .send((7, vec![dir.to_string_lossy().into_owned()], true))
            .unwrap();
        let (generation, expanded, append) = res_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(generation, 7);
        assert!(append);
        assert_eq!(expanded.len(), 1);
        assert!(expanded[0].ends_with("clip.mkv"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transport_cluster_never_overflows_narrow_rows() {
        // back(36) + 2 + play(44) + 2 + fwd(36) on the left, then the right cluster.
        let left = 36.0 + 2.0 + 44.0 + 2.0 + 36.0;
        for row_w in [
            172.0, // narrowest case: 480px window with the 292px drawer open
            220.0, 300.0, 359.0, 360.0, 430.0, 500.0, 700.0,
        ] {
            let ControlCluster {
                show_volume,
                vol_w,
                show_track_buttons,
            } = control_cluster(row_w);
            let right = 36.0 // fullscreen toggle
                + if show_volume { vol_w } else { 0.0 }
                + if show_track_buttons { 36.0 + 36.0 } else { 0.0 };
            assert!(
                left + right <= row_w,
                "row_w={row_w}: left={left} right={right}"
            );
        }
    }

    #[test]
    fn escape_closes_only_the_topmost_transient_layer() {
        assert_eq!(escape_target(true, true, true, true), EscapeTarget::Menu);
        assert_eq!(escape_target(false, true, true, true), EscapeTarget::Dialog);
        assert_eq!(
            escape_target(false, false, true, true),
            EscapeTarget::Playlist
        );
        assert_eq!(
            escape_target(false, false, false, true),
            EscapeTarget::Fullscreen
        );
        assert_eq!(
            escape_target(false, false, false, false),
            EscapeTarget::None
        );
    }

    #[test]
    fn chrome_hides_only_while_playing_without_panels() {
        use super::CHROME_HIDE_SECS;
        // Pure deadline arithmetic behind update_chrome()/chrome_hidden().
        let deadline = |playing: bool, panels_open: bool, now: f64| {
            if playing && !panels_open {
                now + CHROME_HIDE_SECS
            } else {
                f64::INFINITY
            }
        };
        let hidden = |now: f64, deadline: f64, playing: bool, panels_open: bool| {
            playing && !panels_open && now >= deadline
        };

        let d = deadline(true, false, 10.0);
        assert!(!hidden(10.5, d, true, false));
        assert!(hidden(10.0 + CHROME_HIDE_SECS, d, true, false));

        // Paused: never hides.
        let d = deadline(false, false, 10.0);
        assert!(!hidden(100.0, d, false, false));

        // An open panel pins the chrome.
        let d = deadline(true, true, 10.0);
        assert!(!hidden(100.0, d, true, true));
    }

    #[test]
    fn speed_steps_reach_fractional_rates() {
        assert_eq!(normalize_speed(1.0), 1.0);
        assert_eq!(normalize_speed(1.0 + 0.1), 1.1);
        assert_eq!(normalize_speed(0.95 + 0.05), 1.0);
        assert_eq!(normalize_speed(4.0), 4.0);
        assert_eq!(normalize_speed(0.05), 0.2);
        assert_eq!(normalize_speed(50.0), 12.0);
        assert_eq!(normalize_speed(f64::NAN), 1.0);
        assert_eq!(normalize_speed(f64::INFINITY), 1.0);
    }

    #[test]
    fn drawer_width_never_swallows_the_transport_cluster() {
        use super::max_drawer_width;
        // Wide window: the full maximum is fine.
        assert_eq!(max_drawer_width(1920.0), 420.0);
        // 480 px window: capped so ~172 px of chrome stays clickable.
        assert_eq!(max_drawer_width(480.0), 308.0);
        // Smaller still: chrome wins, the drawer shrinks below its minimum.
        assert_eq!(max_drawer_width(400.0), 292.0);
    }

    #[test]
    fn repaint_cadence_minimizes_work_per_state() {
        use super::repaint_interval_ms;
        // Playing: mpv's frame callback drives the real cadence; 200 ms is a net.
        assert_eq!(repaint_interval_ms(true, false, None, None), 200);
        assert_eq!(repaint_interval_ms(true, false, Some(10), None), 200);
        // Buffering: smooth percentage ticking.
        assert_eq!(repaint_interval_ms(false, true, None, None), 33);
        // OSD message: repaint at fade rate, capped by the remaining time.
        assert_eq!(repaint_interval_ms(false, false, Some(2_000), None), 33);
        assert_eq!(repaint_interval_ms(false, false, Some(8), None), 8);
        assert_eq!(repaint_interval_ms(false, false, Some(0), None), 1);
        // Play/pause flash: smooth fade while the UI is otherwise paused.
        assert_eq!(repaint_interval_ms(false, false, None, Some(400)), 33);
        assert_eq!(repaint_interval_ms(false, false, None, Some(9)), 9);
        // Static UI: coarse keepalive.
        assert_eq!(repaint_interval_ms(false, false, None, None), 250);
    }
}
