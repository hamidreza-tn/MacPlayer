//! The right-click / title menu, rebuilt from live player state every time it opens.
//!
//! The MacPlayer menu model and renderer, drawn as egui areas on the tooltip layer so the
//! menu floats above every other widget.

use egui::{Color32, Context, Id, Order, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::i18n;
use crate::settings::Bookmark;
use crate::skin;
use crate::value::Value;

// -- actions -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Sub,
    Audio,
    Video,
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            TrackKind::Sub => "Subtitle",
            TrackKind::Audio => "Audio",
            TrackKind::Video => "Video",
        }
    }

    pub fn prop(self) -> &'static str {
        match self {
            TrackKind::Sub => "sid",
            TrackKind::Audio => "aid",
            TrackKind::Video => "vid",
        }
    }

    pub fn type_name(self) -> &'static str {
        match self {
            TrackKind::Sub => "sub",
            TrackKind::Audio => "audio",
            TrackKind::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackSel {
    Auto,
    Id(i64),
    No,
}

impl TrackSel {
    pub fn as_str(self) -> String {
        match self {
            TrackSel::Auto => "auto".into(),
            TrackSel::Id(id) => id.to_string(),
            TrackSel::No => "no".into(),
        }
    }
}

/// Everything the chrome, menus and dialogs can ask the player to do.
#[derive(Debug, Clone)]
pub enum Action {
    // -- files ---------------------------------------------------------------------------------
    OpenFiles {
        append: bool,
    },
    OpenFolder {
        append: bool,
    },
    OpenUrl,
    OpenClipboard,
    OpenRecent(String),
    ClearRecent,
    PlayRow(usize),
    Stop,
    // -- playback ------------------------------------------------------------------------------
    TogglePause,
    PrevFile,
    NextFile,
    SeekRel(f64),
    SeekAbs(f64),
    SeekBar(f64, bool),
    SeekTo {
        seconds: Option<f64>,
        percent: Option<f64>,
    },
    FrameStep(i32),
    ChangeSpeed(f64),
    SetSpeed(f64),
    OpenGoTo,
    AbSet(char),
    AbClear,
    ChapterStep(i32),
    SetChapter(i64),
    SetRepeat(String),
    SetLoopMode(String),
    ToggleShuffle,
    ToggleResume,
    SetSleepTimer(i32),
    AddBookmark,
    RemoveBookmark {
        path: String,
        time: f64,
    },
    SeekBookmark {
        path: String,
        time: f64,
    },
    ToggleAutofit,
    SetWindowScale(i32),
    // -- subtitles -----------------------------------------------------------------------------
    ToggleSubVisibility,
    SetTrack(TrackKind, TrackSel),
    CycleTrack(TrackKind),
    SetSecondarySub(TrackSel),
    SubDelay(Option<f64>),
    SetSubDelay(f64),
    SubScale(f64),
    SetSubScale(f64),
    SubMove(i32),
    SubReset,
    SubSeek(i32),
    SetSubCodepage(String),
    LoadSubtitleDialog,
    LoadAudioDialog,
    // -- video ---------------------------------------------------------------------------------
    SetAspect(String, String),
    SetDisplayMode(String),
    Zoom(f64),
    Pan(f64, f64),
    ResetGeometry,
    Rotate(Option<i64>),
    ToggleFlip(&'static str),
    ImageAdjust(&'static str, i32),
    SetImageAdjust(&'static str, f64),
    ImageReset,
    ToggleDeinterlace,
    SetHwdec(bool),
    SetVideoSync(String),
    SetInterpolation(bool),
    SetScaleProfile(&'static str),
    SetVideoFilter {
        filter: &'static str,
        on: bool,
    },
    SetVisualizer(&'static str),
    // -- audio ---------------------------------------------------------------------------------
    SetVolume(i64),
    VolumeStep(i32),
    ToggleMute,
    AudioDelay(Option<f64>),
    SetAudioDelay(f64),
    SetAudioDevice(String),
    ToggleNormalize,
    SetNormalize(bool),
    SetEqualizer(Vec<f64>, bool),
    SetSubPos(f64),
    PlayUrl {
        url: String,
        loop_s: i64,
    },
    OpenDisc,
    SetYtdlQuality(&'static str),
    // -- playlist -------------------------------------------------------------------------------
    PlaylistRemove,
    PlaylistRemoveMissing,
    PlaylistRemoveDuplicates,
    PlaylistClear,
    PlaylistSort(&'static str),
    UndoPlaylist,
    RedoPlaylist,
    PlaylistCopyPath(usize),
    PlaylistShowInFolder(usize),
    // -- capture -------------------------------------------------------------------------------
    Screenshot(&'static str, bool),
    OpenScreenshotFolder,
    // -- window --------------------------------------------------------------------------------
    SetOnTop(String),
    CycleOnTop,
    ToggleInfo,
    ToggleRemaining,
    ToggleFullscreen,
    ExitFullscreen,
    ToggleMiniPlayer,
    Close,
    TogglePlaylist,
    // -- appearance ----------------------------------------------------------------------------
    SetThemeLive(String),
    SetLanguageLive(String),
    // -- dialogs -------------------------------------------------------------------------------
    ShowPreferences(Option<&'static str>),
    ShowControlPanel(Option<&'static str>),
    ShowCommandPalette,
    ShowMediaInfo,
    ShowAbout,
    ShowHistory,
    ClearHistory,
    PlayHistory(String),
    ExportPlaylist,
    ShowTrackMenu(TrackKind),
    ApplySettings {
        hwdec_changed: bool,
    },
    Quit,
}

/// Stable metadata for the application's command surface. UI labels may change without changing
/// these ids, which keeps shortcuts, tests and automation targets durable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub mutation: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandContext {
    pub has_file: bool,
    pub playlist_count: usize,
    pub can_undo_playlist: bool,
    pub can_redo_playlist: bool,
}

impl CommandSpec {
    pub fn enabled(self, context: CommandContext) -> bool {
        match self.id {
            // playPause keeps an empty-state behavior (open media / play the queued
            // row), so it stays enabled without a file; stop and seek do not.
            "media.stop" | "media.seek" => context.has_file,
            "media.next" | "media.previous" => context.playlist_count > 1,
            "playlist.remove" | "playlist.clear" | "playlist.sort" | "playlist.export" => {
                context.playlist_count > 0
            }
            "playlist.undo" => context.can_undo_playlist,
            "playlist.redo" => context.can_redo_playlist,
            _ => true,
        }
    }
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "file.open",
        label: "Open media",
        category: "File",
        mutation: false,
    },
    CommandSpec {
        id: "media.playPause",
        label: "Play / Pause",
        category: "Playback",
        mutation: true,
    },
    CommandSpec {
        id: "media.stop",
        label: "Stop",
        category: "Playback",
        mutation: true,
    },
    CommandSpec {
        id: "media.seek",
        label: "Seek",
        category: "Playback",
        mutation: true,
    },
    CommandSpec {
        id: "media.next",
        label: "Next file",
        category: "Playback",
        mutation: true,
    },
    CommandSpec {
        id: "media.previous",
        label: "Previous file",
        category: "Playback",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.add",
        label: "Add to playlist",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.remove",
        label: "Remove from playlist",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.clear",
        label: "Clear playlist",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.sort",
        label: "Sort playlist",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.export",
        label: "Export playlist",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.undo",
        label: "Undo playlist change",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "playlist.redo",
        label: "Redo playlist change",
        category: "Playlist",
        mutation: true,
    },
    CommandSpec {
        id: "view.fullscreen",
        label: "Toggle fullscreen",
        category: "Window",
        mutation: true,
    },
    CommandSpec {
        id: "window.togglePlaylist",
        label: "Toggle playlist panel",
        category: "Window",
        mutation: true,
    },
];

pub fn command_spec(id: &str) -> Option<CommandSpec> {
    COMMANDS.iter().copied().find(|command| command.id == id)
}

pub fn action_id(action: &Action) -> &'static str {
    match action {
        Action::OpenFiles { .. } | Action::OpenFolder { .. } | Action::OpenUrl => "file.open",
        Action::TogglePause => "media.playPause",
        Action::Stop => "media.stop",
        Action::SeekRel(_) | Action::SeekAbs(_) | Action::SeekBar(_, _) | Action::SeekTo { .. } => {
            "media.seek"
        }
        Action::NextFile => "media.next",
        Action::PrevFile => "media.previous",
        Action::PlaylistRemove
        | Action::PlaylistRemoveMissing
        | Action::PlaylistRemoveDuplicates => "playlist.remove",
        Action::PlaylistClear => "playlist.clear",
        Action::PlaylistSort(_) => "playlist.sort",
        Action::ExportPlaylist => "playlist.export",
        Action::UndoPlaylist => "playlist.undo",
        Action::RedoPlaylist => "playlist.redo",
        Action::ToggleFullscreen | Action::ExitFullscreen => "view.fullscreen",
        Action::TogglePlaylist => "window.togglePlaylist",
        _ => "app.action",
    }
}

// -- menu model ----------------------------------------------------------------------------------

pub enum Entry {
    Item {
        label: String,
        shortcut: String,
        checked: Option<bool>,
        enabled: bool,
        action: Action,
    },
    Sub {
        label: String,
        enabled: bool,
        entries: Vec<Entry>,
    },
    Sep,
}

impl Entry {
    pub fn item(label: impl Into<String>, action: Action) -> Self {
        Entry::Item {
            label: label.into(),
            shortcut: String::new(),
            checked: None,
            enabled: true,
            action,
        }
    }

    fn key(label: impl Into<String>, shortcut: impl Into<String>, action: Action) -> Self {
        let shortcut = shortcut.into();
        Entry::Item {
            label: label.into(),
            shortcut: display_shortcut(&shortcut),
            checked: None,
            enabled: true,
            action,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        if let Entry::Item { enabled: e, .. } | Entry::Sub { enabled: e, .. } = &mut self {
            *e = enabled;
        }
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        if let Entry::Item { checked: c, .. } = &mut self {
            *c = Some(checked);
        }
        self
    }

    pub fn sub(label: impl Into<String>, entries: Vec<Entry>) -> Self {
        Entry::Sub {
            label: label.into(),
            enabled: true,
            entries,
        }
    }
}

fn tracks_menu(kind: TrackKind, tracks: &[Value], with_off: bool, has_file: bool) -> Vec<Entry> {
    let selected: Vec<Value> = tracks
        .iter()
        .filter(|t| t.get("type").as_str() == Some(kind_name(kind)))
        .cloned()
        .collect();
    let any_selected = selected.iter().any(|t| t.get("selected").as_bool());
    let mut entries = Vec::new();
    if with_off {
        entries.push(
            Entry::item(i18n::tr("Off"), Action::SetTrack(kind, TrackSel::No))
                .checked(!any_selected)
                .enabled(has_file),
        );
    }
    for track in &selected {
        let id = track.get("id").as_i64().unwrap_or(0);
        entries.push(
            Entry::item(
                crate::skin::track_label(track),
                Action::SetTrack(kind, TrackSel::Id(id)),
            )
            .checked(track.get("selected").as_bool()),
        );
    }
    if entries.is_empty() && !with_off {
        entries.push(Entry::item(i18n::tr("(none)"), Action::TogglePause).enabled(false));
    }
    entries
}

fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Sub => "sub",
        TrackKind::Audio => "audio",
        TrackKind::Video => "video",
    }
}

/// Live player snapshot the menus read. Built fresh each time a menu opens.
pub struct MenuCtx<'a> {
    pub has_file: bool,
    pub playlist_count: usize,
    pub recent: &'a [String],
    pub bookmarks: &'a [Bookmark],
    pub repeat: &'a str,
    pub loop_mode: &'a str,
    /// 0 = off, -1 = after playlist, otherwise minutes left.
    pub sleep_minutes: i32,
    pub ytdl_quality: &'a str,
    pub video_sync: &'a str,
    pub interpolation: bool,
    pub scale_profile: &'a str,
    /// (deband, denoise, sharpen)
    pub filters: (bool, bool, bool),
    pub visualizer: &'a str,
    pub mini_player: bool,
    pub shuffle: bool,
    pub resume: bool,
    pub autofit: bool,
    pub on_top: &'a str,
    pub hwdec: bool,
    pub sub_codepage: &'a str,
    pub sub_visible: bool,
    pub info_visible: bool,
    pub fullscreen: bool,
    pub aspect_name: &'a str,
    pub display_mode: &'a str,
    pub flips: (bool, bool),
    pub normalize: bool,
    pub speed: f64,
    pub mute: bool,
    pub tracks: &'a [Value],
    pub secondary_sid: Option<i64>,
    pub chapters: &'a [Value],
    pub current_chapter: i64,
    pub ab: (Option<f64>, Option<f64>),
    pub audio_devices: &'a [Value],
    pub audio_device: &'a str,
    pub hwdec_current: &'a str,
    pub seek_small: f64,
    pub seek_medium: f64,
    pub seek_large: f64,
    pub seek_huge: f64,
    pub deinterlace: bool,
    pub rotation: i64,
    pub can_undo_playlist: bool,
    pub can_redo_playlist: bool,
}

pub fn main_menu(c: &MenuCtx<'_>) -> Vec<Entry> {
    let has = c.has_file;
    let mut root = Vec::new();

    root.push(Entry::key(
        i18n::tr("Open File(s)..."),
        "F3",
        Action::OpenFiles { append: false },
    ));
    let open = vec![
        Entry::key(
            i18n::tr("Open File(s)..."),
            "Ctrl+O",
            Action::OpenFiles { append: false },
        ),
        Entry::key(
            i18n::tr("Open Folder..."),
            "F2",
            Action::OpenFolder { append: false },
        ),
        Entry::key(i18n::tr("Open URL..."), "Ctrl+U", Action::OpenUrl),
        Entry::key(
            i18n::tr("Open from Clipboard"),
            "Ctrl+Shift+V",
            Action::OpenClipboard,
        ),
        Entry::Sep,
        Entry::item(i18n::tr("Open Disc (DVD/Blu-ray)..."), Action::OpenDisc),
        quality_menu(c.ytdl_quality),
        Entry::Sep,
        Entry::key(
            i18n::tr("Load Subtitle..."),
            "Alt+O",
            Action::LoadSubtitleDialog,
        )
        .enabled(has),
        Entry::item(i18n::tr("Load External Audio..."), Action::LoadAudioDialog).enabled(has),
    ];
    root.push(Entry::sub(i18n::tr("Open"), open));
    let mut recent = Vec::new();
    for (index, path) in c.recent.iter().take(20).enumerate() {
        recent.push(Entry::item(
            format!("{}. {}", index + 1, crate::util::title_of(path)),
            Action::OpenRecent(path.clone()),
        ));
    }
    if !c.recent.is_empty() {
        recent.push(Entry::Sep);
    }
    recent.push(
        Entry::item(i18n::tr("Clear Recent Files"), Action::ClearRecent)
            .enabled(!c.recent.is_empty()),
    );
    root.push(Entry::sub(i18n::tr("Recent Files"), recent));

    let mut bookmarks = Vec::new();
    let mut removable = Vec::new();
    for bookmark in c.bookmarks.iter().rev().take(100) {
        let label = format!(
            "{}   {}",
            bookmark.title,
            crate::util::fmt_time(bookmark.time)
        );
        bookmarks.push(Entry::item(
            label.clone(),
            Action::SeekBookmark {
                path: bookmark.path.clone(),
                time: bookmark.time,
            },
        ));
        removable.push(Entry::item(
            label,
            Action::RemoveBookmark {
                path: bookmark.path.clone(),
                time: bookmark.time,
            },
        ));
    }
    bookmarks.push(Entry::Sep);
    bookmarks.push(Entry::item(i18n::tr("Add Bookmark"), Action::AddBookmark).enabled(has));
    // Removal lives in its own submenu so the main list stays one row per
    // bookmark instead of doubling in size.
    if !removable.is_empty() {
        bookmarks.push(Entry::sub(i18n::tr("Remove Bookmark"), removable));
    }
    root.push(Entry::sub(i18n::tr("Bookmarks"), bookmarks));

    root.push(Entry::key(i18n::tr("Close Playback"), "F4", Action::Stop).enabled(has));
    root.push(Entry::Sep);
    root.push(
        Entry::key(
            i18n::tr("Undo Playlist Change"),
            "Ctrl+Alt+Z",
            Action::UndoPlaylist,
        )
        .enabled(c.can_undo_playlist),
    );
    root.push(
        Entry::key(
            i18n::tr("Redo Playlist Change"),
            "Ctrl+Shift+Alt+Z",
            Action::RedoPlaylist,
        )
        .enabled(c.can_redo_playlist),
    );
    root.push(Entry::Sep);

    // -- Playback ------------------------------------------------------------------------------
    let mut playback = vec![
        Entry::key(i18n::tr("Play / Pause"), "Space", Action::TogglePause)
            .enabled(has || c.playlist_count > 0),
        Entry::key(i18n::tr("Stop"), "F4", Action::Stop).enabled(has),
        Entry::key(i18n::tr("Previous File"), "PgUp", Action::PrevFile)
            .enabled(c.playlist_count > 1),
        Entry::key(i18n::tr("Next File"), "PgDn", Action::NextFile).enabled(c.playlist_count > 1),
        Entry::Sep,
    ];
    let jump = vec![
        Entry::key(
            format!("{} {} {}", i18n::tr("Back"), c.seek_small, i18n::tr("sec")),
            "Left",
            Action::SeekRel(-c.seek_small),
        ),
        Entry::key(
            format!(
                "{} {} {}",
                i18n::tr("Forward"),
                c.seek_small,
                i18n::tr("sec")
            ),
            "Right",
            Action::SeekRel(c.seek_small),
        ),
        Entry::key(
            format!("{} {} {}", i18n::tr("Back"), c.seek_medium, i18n::tr("sec")),
            "Ctrl+Left",
            Action::SeekRel(-c.seek_medium),
        ),
        Entry::key(
            format!(
                "{} {} {}",
                i18n::tr("Forward"),
                c.seek_medium,
                i18n::tr("sec")
            ),
            "Ctrl+Right",
            Action::SeekRel(c.seek_medium),
        ),
        Entry::key(
            format!("{} {} {}", i18n::tr("Back"), c.seek_large, i18n::tr("sec")),
            "Shift+Left",
            Action::SeekRel(-c.seek_large),
        ),
        Entry::key(
            format!(
                "{} {} {}",
                i18n::tr("Forward"),
                c.seek_large,
                i18n::tr("sec")
            ),
            "Shift+Right",
            Action::SeekRel(c.seek_large),
        ),
        Entry::key(
            format!("{} {} {}", i18n::tr("Back"), c.seek_huge, i18n::tr("sec")),
            "Ctrl+Alt+Left",
            Action::SeekRel(-c.seek_huge),
        ),
        Entry::key(
            format!(
                "{} {} {}",
                i18n::tr("Forward"),
                c.seek_huge,
                i18n::tr("sec")
            ),
            "Ctrl+Alt+Right",
            Action::SeekRel(c.seek_huge),
        ),
        Entry::Sep,
        Entry::key(
            i18n::tr("Jump to Beginning"),
            "Backspace",
            Action::SeekAbs(0.0),
        ),
        Entry::key(i18n::tr("Go to Time..."), "G", Action::OpenGoTo),
        Entry::Sep,
        Entry::item(i18n::tr("Previous Subtitle Line"), Action::SubSeek(-1)),
        Entry::item(i18n::tr("Next Subtitle Line"), Action::SubSeek(1)),
    ];
    playback.push(Entry::sub(i18n::tr("Jump"), jump).enabled(has));
    let speed = vec![
        Entry::key(i18n::tr("Faster"), "C", Action::ChangeSpeed(0.1)),
        Entry::key(i18n::tr("Slower"), "X", Action::ChangeSpeed(-0.1)),
        Entry::key(i18n::tr("Normal"), "Z", Action::SetSpeed(1.0)),
        Entry::Sep,
    ]
    .into_iter()
    .chain(crate::skin::SPEEDS.iter().map(|value| {
        Entry::item(format!("{value}x"), Action::SetSpeed(*value))
            .checked((c.speed - value).abs() < 0.005)
    }))
    .collect();
    playback.push(Entry::sub(i18n::tr("Speed"), speed));
    let frame_step = vec![
        Entry::key(i18n::tr("Next Frame"), "F", Action::FrameStep(1)),
        Entry::key(i18n::tr("Previous Frame"), "D", Action::FrameStep(-1)),
    ];
    playback.push(Entry::sub(i18n::tr("Frame Step"), frame_step).enabled(has));
    let (a, b) = c.ab;
    let ab = vec![
        Entry::key(
            format!("{}{}", i18n::tr("Set Start Point (A)"), suffix_time(a)),
            "[",
            Action::AbSet('a'),
        ),
        Entry::key(
            format!("{}{}", i18n::tr("Set End Point (B)"), suffix_time(b)),
            "]",
            Action::AbSet('b'),
        ),
        Entry::Sep,
        Entry::key(i18n::tr("Clear A-B Repeat"), "\\", Action::AbClear)
            .enabled(a.is_some() || b.is_some()),
    ];
    playback.push(Entry::sub(i18n::tr("A-B Repeat"), ab).enabled(has));
    if !c.chapters.is_empty() {
        let mut chapter_entries: Vec<Entry> = c
            .chapters
            .iter()
            .enumerate()
            .map(|(index, chapter)| {
                let title = chapter
                    .get("title")
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{} {}", i18n::tr("Chapter"), index + 1));
                Entry::item(
                    format!(
                        "{}. {}   {}",
                        index + 1,
                        title,
                        crate::util::fmt_time(chapter.get("time").as_f64().unwrap_or(0.0))
                    ),
                    Action::SetChapter(index as i64),
                )
                .checked(index as i64 == c.current_chapter)
            })
            .collect();
        chapter_entries.push(Entry::Sep);
        chapter_entries.push(Entry::key(
            i18n::tr("Previous Chapter"),
            "Shift+PgUp",
            Action::ChapterStep(-1),
        ));
        chapter_entries.push(Entry::key(
            i18n::tr("Next Chapter"),
            "Shift+PgDn",
            Action::ChapterStep(1),
        ));
        playback.push(Entry::sub(i18n::tr("Chapters"), chapter_entries));
    }
    playback.push(Entry::Sep);
    let repeat = vec![
        Entry::item(i18n::tr("Off"), Action::SetRepeat("off".into())).checked(c.repeat == "off"),
        Entry::item(i18n::tr("Current File"), Action::SetRepeat("one".into()))
            .checked(c.repeat == "one"),
        Entry::item(i18n::tr("Playlist"), Action::SetRepeat("all".into()))
            .checked(c.repeat == "all"),
    ];
    playback.push(Entry::sub(i18n::tr("Repeat"), repeat));
    let looper = vec![
        Entry::item(i18n::tr("Off"), Action::SetLoopMode("none".into()))
            .checked(c.loop_mode == "none"),
        Entry::item(i18n::tr("Current File"), Action::SetLoopMode("file".into()))
            .checked(c.loop_mode == "file"),
        Entry::item(i18n::tr("Playlist"), Action::SetLoopMode("playlist".into()))
            .checked(c.loop_mode == "playlist"),
    ];
    playback.push(Entry::sub(i18n::tr("Loop"), looper));
    playback.push(Entry::item(i18n::tr("Shuffle"), Action::ToggleShuffle).checked(c.shuffle));
    playback.push(
        Entry::item(i18n::tr("Remember Playback Position"), Action::ToggleResume).checked(c.resume),
    );
    root.push(Entry::sub(i18n::tr("Playback"), playback));

    // -- Subtitles -----------------------------------------------------------------------------
    let mut subtitles = vec![
        Entry::key(
            i18n::tr("Show Subtitles"),
            "Alt+H",
            Action::ToggleSubVisibility,
        )
        .checked(c.sub_visible),
        Entry::key(
            i18n::tr("Load Subtitle..."),
            "Alt+O",
            Action::LoadSubtitleDialog,
        )
        .enabled(has),
        Entry::Sep,
    ];
    subtitles.extend(tracks_menu(TrackKind::Sub, c.tracks, true, has));
    subtitles.push(Entry::Sep);
    let subs: Vec<&Value> = c
        .tracks
        .iter()
        .filter(|t| t.get("type").as_str() == Some("sub"))
        .collect();
    let mut secondary = vec![
        Entry::item(i18n::tr("Off"), Action::SetSecondarySub(TrackSel::No))
            .checked(c.secondary_sid.is_none()),
    ];
    for track in &subs {
        let id = track.get("id").as_i64().unwrap_or(0);
        secondary.push(
            Entry::item(
                crate::skin::track_label(track),
                Action::SetSecondarySub(TrackSel::Id(id)),
            )
            .checked(c.secondary_sid == Some(id)),
        );
    }
    subtitles.push(Entry::sub(i18n::tr("Secondary Subtitle"), secondary).enabled(has));
    let sync = vec![
        Entry::key(
            i18n::tr("0.5 sec Faster"),
            ",",
            Action::SubDelay(Some(-0.5)),
        ),
        Entry::key(i18n::tr("0.5 sec Slower"), ".", Action::SubDelay(Some(0.5))),
        Entry::key(i18n::tr("Reset Sync"), "/", Action::SubDelay(None)),
    ];
    subtitles.push(Entry::sub(i18n::tr("Subtitle Sync"), sync).enabled(has));
    let size = vec![
        Entry::key(i18n::tr("Larger"), "Alt+PgUp", Action::SubScale(0.1)),
        Entry::key(i18n::tr("Smaller"), "Alt+PgDn", Action::SubScale(-0.1)),
        Entry::key(i18n::tr("Move Up"), "Alt+Up", Action::SubMove(-1)),
        Entry::key(i18n::tr("Move Down"), "Alt+Down", Action::SubMove(1)),
        Entry::Sep,
        Entry::item(i18n::tr("Reset"), Action::SubReset),
    ];
    subtitles.push(Entry::sub(i18n::tr("Size and Position"), size));
    let encoding: Vec<Entry> = crate::skin::ENCODINGS
        .iter()
        .map(|(label, codepage)| {
            Entry::item(*label, Action::SetSubCodepage((*codepage).to_string()))
                .checked(c.sub_codepage == *codepage)
        })
        .collect();
    subtitles.push(Entry::sub(i18n::tr("Encoding"), encoding));
    subtitles.push(Entry::Sep);
    subtitles.push(Entry::item(
        i18n::tr("Subtitle Settings..."),
        Action::ShowPreferences(Some("Subtitles")),
    ));
    root.push(Entry::sub(i18n::tr("Subtitles"), subtitles));

    // -- Video ---------------------------------------------------------------------------------
    let aspect: Vec<Entry> = crate::skin::ASPECTS
        .iter()
        .map(|(label, value)| {
            Entry::item(
                *label,
                Action::SetAspect((*label).to_string(), (*value).to_string()),
            )
            .checked(c.aspect_name == *label)
        })
        .collect();
    let display: Vec<Entry> = [
        (i18n::tr("Keep Aspect Ratio"), "fit"),
        (i18n::tr("Fill Window (Crop)"), "fill"),
        (i18n::tr("Stretch to Window"), "stretch"),
        (i18n::tr("Original Size"), "original"),
    ]
    .iter()
    .map(|(label, mode)| {
        Entry::item(*label, Action::SetDisplayMode((*mode).to_string()))
            .checked(c.display_mode == *mode)
    })
    .collect();
    let pan_scan = vec![
        Entry::key(i18n::tr("Zoom In"), "Num 9", Action::Zoom(0.1)),
        Entry::key(i18n::tr("Zoom Out"), "Num 1", Action::Zoom(-0.1)),
        Entry::Sep,
        Entry::key(i18n::tr("Move Left"), "Num 4", Action::Pan(-0.02, 0.0)),
        Entry::key(i18n::tr("Move Right"), "Num 6", Action::Pan(0.02, 0.0)),
        Entry::key(i18n::tr("Move Up"), "Num 8", Action::Pan(0.0, -0.02)),
        Entry::key(i18n::tr("Move Down"), "Num 2", Action::Pan(0.0, 0.02)),
        Entry::Sep,
        Entry::key(i18n::tr("Reset"), "Num 5", Action::ResetGeometry),
    ];
    let rotation = vec![
        Entry::item(i18n::tr("0°"), Action::Rotate(Some(0))).checked(c.rotation == 0),
        Entry::item(i18n::tr("90°"), Action::Rotate(Some(90))).checked(c.rotation == 90),
        Entry::item(i18n::tr("180°"), Action::Rotate(Some(180))).checked(c.rotation == 180),
        Entry::item(i18n::tr("270°"), Action::Rotate(Some(270))).checked(c.rotation == 270),
        Entry::Sep,
        Entry::key(
            i18n::tr("Rotate 90° Clockwise"),
            "Alt+K",
            Action::Rotate(None),
        ),
        Entry::key(
            i18n::tr("Flip Horizontally"),
            "Ctrl+Z",
            Action::ToggleFlip("hflip"),
        )
        .checked(c.flips.0),
        Entry::key(
            i18n::tr("Flip Vertically"),
            "Ctrl+V",
            Action::ToggleFlip("vflip"),
        )
        .checked(c.flips.1),
    ];
    let mut image: Vec<Entry> = Vec::new();
    for (label, prop, minus, plus) in [
        (i18n::tr("Brightness"), "brightness", "W", "E"),
        (i18n::tr("Contrast"), "contrast", "R", "T"),
        (i18n::tr("Saturation"), "saturation", "Y", "U"),
        (i18n::tr("Hue"), "hue", "I", "O"),
    ] {
        image.push(Entry::key(
            format!("{label} -"),
            minus,
            Action::ImageAdjust(prop, -1),
        ));
        image.push(Entry::key(
            format!("{label} +"),
            plus,
            Action::ImageAdjust(prop, 1),
        ));
    }
    image.push(Entry::Sep);
    image.push(Entry::key(i18n::tr("Reset"), "Q", Action::ImageReset));
    let hw = vec![
        Entry::item(i18n::tr("Use Hardware Decoding"), Action::SetHwdec(true)).checked(c.hwdec),
        Entry::item(i18n::tr("Software Decoding"), Action::SetHwdec(false)).checked(!c.hwdec),
        Entry::Sep,
        Entry::item(
            format!(
                "{}: {}",
                i18n::tr("Active"),
                if c.hwdec_current == "no" {
                    i18n::tr("none (software)").to_string()
                } else {
                    c.hwdec_current.to_string()
                }
            ),
            Action::TogglePause,
        )
        .enabled(false),
    ];
    let videos: Vec<&Value> = c
        .tracks
        .iter()
        .filter(|t| t.get("type").as_str() == Some("video") && !t.get("albumart").as_bool())
        .collect();
    let mut video = vec![
        Entry::sub(i18n::tr("Aspect Ratio"), aspect),
        Entry::sub(i18n::tr("Frame Size"), display),
        Entry::sub(i18n::tr("Pan & Scan"), pan_scan),
        Entry::sub(i18n::tr("Rotate and Flip"), rotation),
        Entry::sub(i18n::tr("Image Adjustments"), image),
        Entry::Sep,
        Entry::item(i18n::tr("Deinterlace"), Action::ToggleDeinterlace).checked(c.deinterlace),
        Entry::sub(i18n::tr("Hardware Acceleration"), hw),
        Entry::sub(i18n::tr("Motion & Sync"), motion_menu(c)),
        Entry::sub(i18n::tr("Image Scaling"), scaling_menu(c.scale_profile)),
        Entry::sub(i18n::tr("Filters"), filter_menu(c)),
        Entry::sub(i18n::tr("Audio Visualizer"), visualizer_menu(c.visualizer)),
    ];
    if videos.len() > 1 {
        video.push(Entry::sub(
            i18n::tr("Video Track"),
            tracks_menu(TrackKind::Video, c.tracks, false, has),
        ));
    }
    root.push(Entry::sub(i18n::tr("Video"), video));

    // -- Audio ---------------------------------------------------------------------------------
    let mut audio_out = vec![
        Entry::sub(
            i18n::tr("Select Audio Stream"),
            tracks_menu(TrackKind::Audio, c.tracks, false, has),
        ),
        Entry::item(i18n::tr("Load External Audio..."), Action::LoadAudioDialog).enabled(has),
        Entry::Sep,
        Entry::key(i18n::tr("Volume Up"), "Up", Action::VolumeStep(1)),
        Entry::key(i18n::tr("Volume Down"), "Down", Action::VolumeStep(-1)),
        Entry::key(i18n::tr("Mute"), "M", Action::ToggleMute).checked(c.mute),
        Entry::Sep,
    ];
    let audio_sync = vec![
        Entry::key(
            i18n::tr("0.05 sec Faster"),
            "Shift+,",
            Action::AudioDelay(Some(-0.05)),
        ),
        Entry::key(
            i18n::tr("0.05 sec Slower"),
            "Shift+.",
            Action::AudioDelay(Some(0.05)),
        ),
        Entry::key(i18n::tr("Reset Sync"), "Shift+/", Action::AudioDelay(None)),
    ];
    audio_out.push(Entry::sub(i18n::tr("Audio Sync"), audio_sync).enabled(has));
    let devices: Vec<Entry> = c
        .audio_devices
        .iter()
        .filter_map(|device| {
            let name = device.get("name").as_str()?.to_string();
            let label = if name == "auto" {
                i18n::tr("Auto (system default)").to_string()
            } else {
                device
                    .get("description")
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .unwrap_or(&name)
                    .to_string()
            };
            Some(
                Entry::item(label, Action::SetAudioDevice(name.clone()))
                    .checked(name == c.audio_device),
            )
        })
        .collect();
    audio_out.push(Entry::sub(i18n::tr("Output Device"), devices));
    audio_out.push(
        Entry::item(i18n::tr("Normalize Volume"), Action::ToggleNormalize).checked(c.normalize),
    );
    audio_out.push(Entry::item(
        i18n::tr("Equalizer..."),
        Action::ShowControlPanel(Some("Audio")),
    ));
    root.push(Entry::sub(i18n::tr("Audio"), audio_out));

    // -- Misc ----------------------------------------------------------------------------------
    let misc = vec![
        Entry::key(
            i18n::tr("Save Current Frame"),
            "Ctrl+E",
            Action::Screenshot("video", false),
        )
        .enabled(has),
        Entry::key(
            i18n::tr("Save Current Frame (With Subtitles)"),
            "Ctrl+Alt+E",
            Action::Screenshot("subtitles", false),
        )
        .enabled(has),
        Entry::key(
            i18n::tr("Copy Current Frame"),
            "Ctrl+C",
            Action::Screenshot("video", true),
        )
        .enabled(has),
        Entry::key(
            i18n::tr("Copy Current Frame (With Subtitles)"),
            "Ctrl+Alt+C",
            Action::Screenshot("subtitles", true),
        )
        .enabled(has),
        Entry::item(
            i18n::tr("Open Screenshot Folder"),
            Action::OpenScreenshotFolder,
        ),
        Entry::Sep,
        Entry::sub(
            i18n::tr("On Top"),
            vec![
                Entry::item(i18n::tr("Never"), Action::SetOnTop("never".into()))
                    .checked(c.on_top == "never"),
                Entry::item(i18n::tr("Always"), Action::SetOnTop("always".into()))
                    .checked(c.on_top == "always"),
                Entry::item(
                    i18n::tr("While Playing"),
                    Action::SetOnTop("playing".into()),
                )
                .checked(c.on_top == "playing"),
            ],
        ),
        Entry::key(
            i18n::tr("Playback Information (OSD)"),
            "Tab",
            Action::ToggleInfo,
        )
        .checked(c.info_visible),
        Entry::sub(i18n::tr("Sleep Timer"), sleep_menu(c.sleep_minutes)),
        Entry::item(i18n::tr("History..."), Action::ShowHistory),
    ];
    root.push(Entry::sub(i18n::tr("Misc"), misc));
    root.push(Entry::Sep);

    let window_size = vec![
        Entry::key(i18n::tr("50%"), "Alt+1", Action::SetWindowScale(50)).enabled(has),
        Entry::key(i18n::tr("100%"), "Alt+2", Action::SetWindowScale(100)).enabled(has),
        Entry::key(i18n::tr("150%"), "Alt+3", Action::SetWindowScale(150)).enabled(has),
        Entry::key(i18n::tr("200%"), "Alt+4", Action::SetWindowScale(200)).enabled(has),
        Entry::Sep,
        Entry::item(
            i18n::tr("Fit Window to Video on Open"),
            Action::ToggleAutofit,
        )
        .checked(c.autofit),
    ];
    root.push(Entry::sub(i18n::tr("Window Size"), window_size));
    root.push(
        Entry::key(i18n::tr("Full Screen"), "Enter", Action::ToggleFullscreen)
            .checked(c.fullscreen),
    );
    root.push(
        Entry::item(i18n::tr("Mini Player"), Action::ToggleMiniPlayer).checked(c.mini_player),
    );
    root.push(Entry::Sep);
    root.push(Entry::key(
        i18n::tr("Preferences..."),
        "Ctrl+,",
        Action::ShowPreferences(None),
    ));
    root.push(Entry::key(
        i18n::tr("Playlist..."),
        "Ctrl+P",
        Action::TogglePlaylist,
    ));
    root.push(
        Entry::item(i18n::tr("Export Playlist..."), Action::ExportPlaylist)
            .enabled(c.playlist_count > 0),
    );
    root.push(Entry::item(
        i18n::tr("Control Panel..."),
        Action::ShowControlPanel(None),
    ));
    root.push(Entry::key(
        i18n::tr("Playback/System Info..."),
        "Ctrl+F1",
        Action::ShowMediaInfo,
    ));
    root.push(Entry::key(i18n::tr("About..."), "F1", Action::ShowAbout));
    root.push(Entry::Sep);
    root.push(Entry::key(i18n::tr("Exit"), quit_shortcut(), Action::Quit));
    root
}

#[cfg(target_os = "macos")]
fn display_shortcut(shortcut: &str) -> String {
    shortcut.replace("Ctrl", "Cmd").replace("Alt", "Option")
}

#[cfg(not(target_os = "macos"))]
fn display_shortcut(shortcut: &str) -> String {
    shortcut.to_string()
}

#[cfg(target_os = "macos")]
fn quit_shortcut() -> &'static str {
    "Cmd+Q"
}

#[cfg(not(target_os = "macos"))]
fn quit_shortcut() -> &'static str {
    "Alt+F4"
}

fn suffix_time(value: Option<f64>) -> String {
    match value {
        Some(seconds) => format!("   {}", crate::util::fmt_time(seconds)),
        None => String::new(),
    }
}

/// yt-dlp quality submenu, shared by the Open menu.
fn quality_menu(current: &str) -> Entry {
    let ids: &[(&str, &str)] = &[
        ("best", "Best Available"),
        ("2160", "4K (2160p)"),
        ("1440", "2K (1440p)"),
        ("1080", "1080p"),
        ("720", "720p"),
        ("480", "480p"),
        ("360", "360p"),
        ("audio", "Audio Only"),
    ];
    Entry::sub(
        i18n::tr("Online Stream Quality"),
        ids.iter()
            .map(|(id, label)| {
                Entry::item(i18n::tr(label), Action::SetYtdlQuality(id)).checked(*id == current)
            })
            .collect(),
    )
}

/// video-sync modes plus the interpolation switch.
fn motion_menu(c: &MenuCtx<'_>) -> Vec<Entry> {
    let modes: &[(&str, &str)] = &[
        ("auto", "Auto (default)"),
        ("audio", "Audio Clock"),
        ("display-resync", "Display Sync (resample audio)"),
        ("display-desync", "Display (drop/duplicate frames)"),
    ];
    let mut entries: Vec<Entry> = modes
        .iter()
        .map(|(value, label)| {
            Entry::item(i18n::tr(label), Action::SetVideoSync((*value).to_string()))
                .checked(c.video_sync == *value)
        })
        .collect();
    entries.push(Entry::Sep);
    entries.push(
        Entry::item(
            i18n::tr("Frame Interpolation"),
            Action::SetInterpolation(!c.interpolation),
        )
        .checked(c.interpolation),
    );
    entries
}

fn scaling_menu(current: &str) -> Vec<Entry> {
    let profiles: &[(&str, &str)] = &[
        ("bilinear", "Bilinear"),
        ("bicubic", "Bicubic"),
        ("spline36", "Spline36"),
        ("lanczos", "Lanczos"),
        ("ewa_lanczossharp", "EWA Lanczos Sharp"),
    ];
    profiles
        .iter()
        .map(|(value, label)| {
            Entry::item(i18n::tr(label), Action::SetScaleProfile(value)).checked(*value == current)
        })
        .collect()
}

fn filter_menu(c: &MenuCtx<'_>) -> Vec<Entry> {
    let (deband, denoise, sharpen) = c.filters;
    vec![
        Entry::item(
            i18n::tr("Deband"),
            Action::SetVideoFilter {
                filter: "deband",
                on: !deband,
            },
        )
        .checked(deband),
        Entry::item(
            i18n::tr("Denoise"),
            Action::SetVideoFilter {
                filter: "denoise",
                on: !denoise,
            },
        )
        .checked(denoise),
        Entry::item(
            i18n::tr("Sharpen"),
            Action::SetVideoFilter {
                filter: "sharpen",
                on: !sharpen,
            },
        )
        .checked(sharpen),
    ]
}

fn visualizer_menu(current: &str) -> Vec<Entry> {
    [("off", "Off"), ("wave", "Wave"), ("spectrum", "Spectrum")]
        .iter()
        .map(|(value, label)| {
            Entry::item(i18n::tr(label), Action::SetVisualizer(value)).checked(*value == current)
        })
        .collect()
}

/// Sleep-timer presets: 0 = off, -1 = pause when the playlist ends.
fn sleep_menu(active: i32) -> Vec<Entry> {
    let mut entries =
        vec![Entry::item(i18n::tr("Off"), Action::SetSleepTimer(0)).checked(active == 0)];
    for minutes in [15, 30, 60, 90, 120] {
        let label = if active == minutes {
            format!("{minutes} min ✓")
        } else {
            format!("{minutes} min")
        };
        entries.push(Entry::item(label, Action::SetSleepTimer(minutes)));
    }
    entries.push(Entry::Sep);
    entries.push(
        Entry::item(i18n::tr("After Playlist"), Action::SetSleepTimer(-1)).checked(active < 0),
    );
    entries
}

pub fn track_menu(kind: TrackKind, tracks: &[Value], has_file: bool) -> Vec<Entry> {
    let mut entries = tracks_menu(kind, tracks, kind == TrackKind::Sub, has_file);
    if kind == TrackKind::Sub {
        entries.push(Entry::Sep);
        entries.push(
            Entry::key(
                i18n::tr("Load Subtitle..."),
                "Alt+O",
                Action::LoadSubtitleDialog,
            )
            .enabled(has_file),
        );
    }
    entries
}

// -- menu rendering ------------------------------------------------------------------------------

const ITEM_H: f32 = 26.0;
const SEP_H: f32 = 9.0;
const PAD_LEFT: f32 = 28.0;
const PAD_RIGHT: f32 = 30.0;

#[derive(Default)]
pub struct MenuState {
    pub pos: Option<Pos2>,
    pub path: Vec<usize>,
    /// Keyboard-focused item at the deepest open menu level.
    pub active: Option<usize>,
    /// The press that opened the menu can land in the same frame `show` first runs;
    /// the outside-close check must skip that frame (fast micro-clicks).
    pub just_opened: bool,
    /// When the menu was opened; drives the ~100 ms fade-in of the root panel.
    pub opened_at: Option<std::time::Instant>,
}

impl MenuState {
    pub fn open_at(&mut self, pos: Pos2) {
        self.pos = Some(pos);
        self.just_opened = true;
        self.path.clear();
        self.active = None;
        self.opened_at = Some(std::time::Instant::now());
    }

    pub fn close(&mut self) {
        self.pos = None;
        self.path.clear();
        self.active = None;
    }

    pub fn is_open(&self) -> bool {
        self.pos.is_some()
    }
}

/// Render the open menu. Returns the triggered action, if any; the menu closes itself then.
pub fn show(
    ctx: &Context,
    state: &mut MenuState,
    id_salt: &str,
    entries: &[Entry],
) -> Option<Action> {
    let root_pos = state.pos?;
    let ctx = ctx.clone();

    let mut action: Option<Action> = None;
    let mut rects: Vec<Rect> = Vec::new();
    let mut path = std::mem::take(&mut state.path);
    if entries_at(entries, &path).is_none() {
        path.clear();
    }
    let mut active = state.active;
    if ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
        active = None;
    }
    if !path.is_empty() && ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
        path.pop();
        active = None;
    }
    if let Some(slice) = entries_at(entries, &path) {
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            active = next_enabled(slice, active, 1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            active = next_enabled(slice, active, -1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight))
            && let Some(index) = active
            && matches!(slice.get(index), Some(Entry::Sub { enabled: true, .. }))
        {
            let depth = path.len();
            open_submenu(&mut path, depth, index);
            active = None;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(index) = active
            && let Some(entry) = slice.get(index)
        {
            match entry {
                Entry::Item {
                    enabled: true,
                    action: item_action,
                    ..
                } => action = Some(item_action.clone()),
                Entry::Sub { enabled: true, .. } => {
                    let depth = path.len();
                    open_submenu(&mut path, depth, index);
                    active = None;
                }
                _ => {}
            }
        }
    }
    // Keep the root panel on screen: the quiet-player chrome opens menus near the
    // bottom edge, where an unclamped panel would overflow the window.
    let mut pos = {
        let content = ctx.content_rect();
        let w = panel_width(&ctx, entries);
        let h = panel_height(entries);
        let margin = 6.0;
        pos2(
            root_pos.x.clamp(
                content.left() + margin,
                (content.right() - w - margin).max(content.left() + margin),
            ),
            root_pos.y.clamp(
                content.top() + margin,
                (content.bottom() - h - margin).max(content.top() + margin),
            ),
        )
    };
    let mut level = 0usize;
    // Root panel ease-in: smoothstep over the first ~100 ms after open_at().
    let fade = state
        .opened_at
        .map(|t| (t.elapsed().as_secs_f32() / 0.10).clamp(0.0, 1.0))
        .unwrap_or(1.0);
    let fade = fade * fade * (3.0 - 2.0 * fade);
    while let Some(slice) = entries_at(entries, &path[..level]) {
        let out = panel(
            &ctx,
            id_salt,
            level,
            pos,
            slice,
            path.get(level).copied(),
            if level == path.len() { active } else { None },
            if level == 0 { fade } else { 1.0 },
            &mut action,
            &mut rects,
        );
        if let Some(clicked) = out.clicked_sub {
            open_submenu(&mut path, level, clicked);
            active = None;
            if let Some(item_rect) = out.clicked_rect {
                pos = child_pos(&ctx, entries, &path, level, item_rect);
            }
            level += 1;
            if level > 6 {
                break;
            }
            continue;
        }
        if let Some(item_rect) = out.active_rect
            && level < path.len()
        {
            pos = child_pos(&ctx, entries, &path, level, item_rect);
            level += 1;
            if level > 6 {
                break;
            }
            continue;
        }
        path.truncate(level);
        break;
    }

    let just_opened = state.just_opened;
    state.just_opened = false;
    if !just_opened && ctx.input(|i| i.pointer.primary_pressed()) {
        let over_menu = ctx
            .pointer_hover_pos()
            .is_some_and(|p| rects.iter().any(|r| r.contains(p)));
        if !over_menu {
            state.close();
        }
    }

    state.path = path;
    state.active = active;
    if action.is_some() {
        state.close();
    }
    action
}

fn entry_enabled(entry: &Entry) -> bool {
    match entry {
        Entry::Item { enabled, .. } | Entry::Sub { enabled, .. } => *enabled,
        Entry::Sep => false,
    }
}

/// Next enabled item in `delta` direction, wrapping around separators and disabled rows.
fn next_enabled(entries: &[Entry], current: Option<usize>, delta: i32) -> Option<usize> {
    let len = entries.len();
    if len == 0 {
        return None;
    }
    let start = match current {
        Some(index) if index < len => index as i32,
        Some(_) => -1,
        None if delta > 0 => -1,
        None => len as i32,
    };
    (1..=len as i32)
        .map(|step| (start + delta * step).rem_euclid(len as i32) as usize)
        .find(|index| entry_enabled(&entries[*index]))
}

fn open_submenu(path: &mut Vec<usize>, level: usize, submenu: usize) {
    path.truncate(level);
    path.push(submenu);
}

fn child_pos(
    ctx: &Context,
    entries: &[Entry],
    path: &[usize],
    level: usize,
    item_rect: Rect,
) -> Pos2 {
    let child_entries = entries_at(entries, &path[..=level]).unwrap_or(&[]);
    submenu_pos(
        item_rect,
        panel_width(ctx, child_entries),
        panel_height(child_entries),
        ctx.content_rect(),
    )
}

fn submenu_pos(item_rect: Rect, child_width: f32, child_height: f32, bounds: Rect) -> Pos2 {
    let right = item_rect.right() + 2.0;
    let left = item_rect.left() - child_width - 2.0;
    let x = if right + child_width <= bounds.right() {
        right
    } else if left >= bounds.left() {
        left
    } else {
        (bounds.right() - child_width).max(bounds.left())
    };
    let y = (item_rect.top() - 3.0).clamp(
        bounds.top(),
        (bounds.bottom() - child_height).max(bounds.top()),
    );
    pos2(x, y)
}

fn entries_at<'a>(entries: &'a [Entry], path: &[usize]) -> Option<&'a [Entry]> {
    let mut slice = entries;
    for index in path {
        match slice.get(*index) {
            Some(Entry::Sub { entries: sub, .. }) => slice = sub,
            _ => return None,
        }
    }
    Some(slice)
}

fn panel_width(ctx: &Context, entries: &[Entry]) -> f32 {
    let colors = skin::active_colors();
    let font_id = skin::font(13.0, false);
    let measure = |text: &str| {
        ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text.to_string(), font_id.clone(), colors.text_e4)
                .size()
                .x
        })
    };
    let mut width = 0.0f32;
    for entry in entries {
        let (label, shortcut) = match entry {
            Entry::Item {
                label, shortcut, ..
            } => (label, Some(shortcut)),
            Entry::Sub { label, .. } => (label, None),
            Entry::Sep => continue,
        };
        let mut w = measure(label) + PAD_LEFT + PAD_RIGHT;
        if let Some(shortcut) = shortcut {
            if !shortcut.is_empty() {
                w += measure(shortcut) + 12.0;
            }
        } else {
            w += 14.0;
        }
        width = width.max(w);
    }
    width.max(150.0)
}

fn panel_height(entries: &[Entry]) -> f32 {
    entries
        .iter()
        .map(|entry| match entry {
            Entry::Sep => SEP_H,
            _ => ITEM_H,
        })
        .sum()
}

struct PanelOut {
    clicked_sub: Option<usize>,
    clicked_rect: Option<Rect>,
    active_rect: Option<Rect>,
}

// A menu panel is one paint call over a fixed rect stack; a parameter struct
// would only move the same nine values behind another name.
#[allow(clippy::too_many_arguments)]
fn panel(
    ctx: &Context,
    id_salt: &str,
    level: usize,
    pos: Pos2,
    entries: &[Entry],
    active_sub: Option<usize>,
    keyboard_active: Option<usize>,
    fade: f32,
    action: &mut Option<Action>,
    rects: &mut Vec<Rect>,
) -> PanelOut {
    let colors = skin::active_colors();
    // Fade multiplier for the opening animation; one tint applied to every color.
    let tint = |color: Color32| color.gamma_multiply(fade);
    let font_id = skin::font(13.0, false);
    let width = panel_width(ctx, entries);
    let height = panel_height(entries);

    let mut out = PanelOut {
        clicked_sub: None,
        clicked_rect: None,
        active_rect: None,
    };
    let area = egui::Area::new(Id::new("macplayer-menu").with(id_salt).with(level))
        .fixed_pos(pos + vec2(0.0, (1.0 - fade) * 4.0))
        .order(Order::Tooltip)
        .show(ctx, |ui| {
            ui.set_min_size(vec2(width, height));
            ui.set_max_size(vec2(width, height));
            // `fixed_pos` is only the requested position: Area constrains it before
            // constructing this Ui. Painter and interaction rectangles use screen space.
            let panel_pos = ui.max_rect().min;
            let painter = ui.painter().clone();
            let panel_rect = Rect::from_min_size(panel_pos, vec2(width, height));
            let corner_radius = ctx.style_of(egui::Theme::Dark).visuals.menu_corner_radius;
            painter.add(
                egui::epaint::Shadow {
                    offset: [0, 3],
                    blur: 12,
                    spread: 2,
                    color: tint(colors.shadow),
                }
                .as_shape(panel_rect, corner_radius),
            );
            painter.rect_filled(panel_rect, corner_radius, tint(colors.menu_bg));
            painter.rect_stroke(
                panel_rect,
                corner_radius,
                Stroke::new(1.0, tint(colors.dialog_line)),
                egui::StrokeKind::Outside,
            );

            let mut y = 3.0f32;
            for (index, entry) in entries.iter().enumerate() {
                let item_rect = match entry {
                    Entry::Sep => {
                        painter.rect_filled(
                            Rect::from_min_size(
                                pos2(panel_pos.x + 4.0, panel_pos.y + y + 4.0),
                                vec2(width - 8.0, 1.0),
                            ),
                            0.0,
                            tint(colors.line),
                        );
                        y += SEP_H;
                        continue;
                    }
                    _ => {
                        Rect::from_min_size(pos2(panel_pos.x, panel_pos.y + y), vec2(width, ITEM_H))
                    }
                };
                y += ITEM_H;

                let (label, shortcut, checked, enabled) = match entry {
                    Entry::Item {
                        label,
                        shortcut,
                        checked,
                        enabled,
                        ..
                    } => (label, shortcut.as_str(), *checked, *enabled),
                    Entry::Sub { label, enabled, .. } => (label, "", None, *enabled),
                    Entry::Sep => continue,
                };

                let id = ui.id().with("item").with(index);
                let response = ui.interact(item_rect, id, Sense::click());
                let hovered = enabled
                    && (response.hovered()
                        || keyboard_active == Some(index)
                        || active_sub == Some(index));
                if hovered {
                    painter.rect_filled(
                        item_rect.shrink2(vec2(2.0, 1.0)),
                        4.0,
                        tint(colors.menu_sel),
                    );
                    // Focus ring only for keyboard navigation; pointer hover stays a
                    // quiet fill (macOS menu convention).
                    if keyboard_active == Some(index) {
                        painter.rect_stroke(
                            item_rect.shrink2(vec2(2.0, 1.0)),
                            4.0,
                            Stroke::new(1.0, tint(colors.focus)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }
                let text_color = if !enabled {
                    tint(colors.icon_off)
                } else if hovered {
                    tint(colors.text_bright)
                } else {
                    tint(colors.text_e4)
                };
                if checked.unwrap_or(false) {
                    painter.circle_filled(
                        pos2(panel_pos.x + 12.0, item_rect.center().y),
                        3.0,
                        tint(colors.accent),
                    );
                }
                painter.text(
                    pos2(panel_pos.x + PAD_LEFT, item_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label.clone(),
                    font_id.clone(),
                    text_color,
                );
                if matches!(entry, Entry::Sub { .. }) {
                    // Right-pointing chevron drawn on the shared pixel grid, matching
                    // the hand-drawn glyph style everywhere else.
                    let c = pos2(item_rect.right() - 16.0, item_rect.center().y);
                    let stroke = Stroke::new(1.4, text_color);
                    painter
                        .line_segment([pos2(c.x - 2.0, c.y - 3.5), pos2(c.x + 1.0, c.y)], stroke);
                    painter
                        .line_segment([pos2(c.x + 1.0, c.y), pos2(c.x - 2.0, c.y + 3.5)], stroke);
                } else if !shortcut.is_empty() {
                    painter.text(
                        pos2(item_rect.right() - PAD_RIGHT + 10.0, item_rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        shortcut,
                        font_id.clone(),
                        if hovered {
                            tint(colors.text_e4)
                        } else {
                            tint(colors.text_dim)
                        },
                    );
                }

                if let Entry::Sub { enabled: true, .. } = entry
                    && active_sub == Some(index)
                {
                    out.active_rect = Some(item_rect);
                }
                if response.clicked() && enabled {
                    match entry {
                        Entry::Item {
                            action: item_action,
                            ..
                        } => *action = Some(item_action.clone()),
                        Entry::Sub { enabled: true, .. } => {
                            out.clicked_sub = Some(index);
                            out.clicked_rect = Some(item_rect);
                        }
                        _ => {}
                    }
                }
            }
        });
    rects.push(area.response.rect);
    out
}

// -- misc helpers --------------------------------------------------------------------------------

/// Flatten a menu tree into `(breadcrumb, shortcut, action)` triples for the command
/// palette, so the palette always offers exactly what the menus offer — one source of
/// truth — with the same shortcut hints the menu rows show.
pub fn flatten_entries(entries: &[Entry], prefix: &str, out: &mut Vec<(String, String, Action)>) {
    for entry in entries {
        match entry {
            Entry::Item {
                label,
                shortcut,
                enabled,
                action,
                ..
            } => {
                if *enabled {
                    let path = if prefix.is_empty() {
                        label.clone()
                    } else {
                        format!("{prefix} › {label}")
                    };
                    out.push((path, shortcut.clone(), action.clone()));
                }
            }
            Entry::Sub {
                label,
                enabled,
                entries,
            } => {
                // A disabled submenu gates its whole subtree (e.g. Jump without a
                // file); the palette must not offer commands the menu refuses.
                if !*enabled {
                    continue;
                }
                let path = if prefix.is_empty() {
                    label.clone()
                } else {
                    format!("{prefix} › {label}")
                };
                flatten_entries(entries, &path, out);
            }
            Entry::Sep => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Rect, pos2, vec2};

    use super::{
        Action, CommandContext, Entry, action_id, command_spec, display_shortcut, next_enabled,
        open_submenu, quit_shortcut, submenu_pos,
    };
    use crate::i18n;

    #[test]
    fn keyboard_navigation_skips_separators_and_disabled_items() {
        let entries = vec![
            Entry::item(i18n::tr("a"), Action::TogglePause),
            Entry::Sep,
            Entry::item(i18n::tr("b"), Action::TogglePause).enabled(false),
            Entry::item(i18n::tr("c"), Action::TogglePause),
        ];

        assert_eq!(next_enabled(&entries, None, 1), Some(0));
        assert_eq!(next_enabled(&entries, Some(0), 1), Some(3));
        assert_eq!(next_enabled(&entries, Some(3), 1), Some(0));
        assert_eq!(next_enabled(&entries, None, -1), Some(3));
        assert_eq!(next_enabled(&entries, Some(0), -1), Some(3));
    }

    #[test]
    fn shortcut_labels_use_the_platform_modifier_names() {
        #[cfg(target_os = "macos")]
        assert_eq!(display_shortcut("Ctrl+Alt+Left"), "Cmd+Option+Left");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(display_shortcut("Ctrl+Alt+Left"), "Ctrl+Alt+Left");
    }

    #[test]
    fn exit_shortcut_matches_the_platform_keymap() {
        #[cfg(target_os = "macos")]
        assert_eq!(quit_shortcut(), "Cmd+Q");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(quit_shortcut(), "Alt+F4");
    }

    #[test]
    fn important_actions_have_stable_command_ids() {
        assert_eq!(action_id(&Action::TogglePause), "media.playPause");
        assert_eq!(action_id(&Action::PlaylistClear), "playlist.clear");
        assert_eq!(action_id(&Action::UndoPlaylist), "playlist.undo");
        assert_eq!(
            command_spec("playlist.redo").map(|command| command.mutation),
            Some(true)
        );
        let context = CommandContext {
            playlist_count: 2,
            ..CommandContext::default()
        };
        assert!(!command_spec("playlist.undo").unwrap().enabled(context));
    }

    #[test]
    fn opening_submenu_replaces_sibling_and_descendants() {
        let mut path = vec![2, 1, 4];

        open_submenu(&mut path, 1, 3);
        assert_eq!(path, vec![2, 3]);

        open_submenu(&mut path, 0, 5);
        assert_eq!(path, vec![5]);
    }

    #[test]
    fn submenu_opens_to_the_right_when_it_fits() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let item = Rect::from_min_size(pos2(400.0, 100.0), vec2(180.0, 26.0));

        assert_eq!(submenu_pos(item, 200.0, 120.0, bounds), pos2(582.0, 97.0));
    }

    #[test]
    fn submenu_flips_to_the_left_when_the_right_edge_is_tight() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let item = Rect::from_min_size(pos2(650.0, 100.0), vec2(140.0, 26.0));

        assert_eq!(submenu_pos(item, 240.0, 120.0, bounds), pos2(408.0, 97.0));
    }

    #[test]
    fn submenu_stays_inside_the_bottom_edge() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let item = Rect::from_min_size(pos2(400.0, 580.0), vec2(180.0, 26.0));

        assert_eq!(submenu_pos(item, 200.0, 180.0, bounds), pos2(582.0, 420.0));
    }
}
