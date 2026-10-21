//! The right-click / title menu, rebuilt from live player state every time it opens.
//!
//! The MacPlayer menu model and renderer, drawn as egui areas on the tooltip layer so the
//! menu floats above every other widget.

use egui::{Context, Id, Order, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

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

    pub fn plural(self) -> &'static str {
        match self {
            TrackKind::Sub => "subtitle",
            TrackKind::Audio => "audio",
            TrackKind::Video => "video",
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

    fn parse(value: &Value) -> TrackSel {
        match value.as_i64() {
            Some(id) => TrackSel::Id(id),
            None => TrackSel::No,
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
    ToggleShuffle,
    ToggleResume,
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
    SetKeepAspect(bool),
    SetSubPos(f64),
    PlayUrl {
        url: String,
        loop_s: i64,
    },
    // -- playlist -------------------------------------------------------------------------------
    PlaylistRemove,
    PlaylistRemoveMissing,
    PlaylistRemoveDuplicates,
    PlaylistClear,
    PlaylistSort(&'static str),
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
    ToggleMaximize,
    Minimize,
    Close,
    TogglePlaylist,
    // -- dialogs -------------------------------------------------------------------------------
    ShowPreferences(Option<&'static str>),
    ShowControlPanel(Option<&'static str>),
    ShowMediaInfo,
    ShowAbout,
    ShowTrackMenu(TrackKind),
    ShowMainMenu,
    ApplySettings {
        hwdec_changed: bool,
    },
    Quit,
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

    fn sub_disabled(label: impl Into<String>, entries: Vec<Entry>) -> Self {
        Entry::Sub {
            label: label.into(),
            enabled: false,
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
            Entry::item("Off", Action::SetTrack(kind, TrackSel::No))
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
        entries.push(Entry::item("(none)", Action::TogglePause).enabled(false));
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
    pub playlist_current: Option<usize>,
    pub recent: &'a [String],
    pub bookmarks: &'a [Bookmark],
    pub repeat: &'a str,
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
}

pub fn main_menu(c: &MenuCtx<'_>) -> Vec<Entry> {
    let has = c.has_file;
    let mut root = Vec::new();

    root.push(Entry::key(
        "Open File(s)...",
        "F3",
        Action::OpenFiles { append: false },
    ));
    let open = vec![
        Entry::key(
            "Open File(s)...",
            "Ctrl+O",
            Action::OpenFiles { append: false },
        ),
        Entry::key("Open Folder...", "F2", Action::OpenFolder { append: false }),
        Entry::key("Open URL...", "Ctrl+U", Action::OpenUrl),
        Entry::key("Open from Clipboard", "Ctrl+Shift+V", Action::OpenClipboard),
        Entry::Sep,
        Entry::key("Load Subtitle...", "Alt+O", Action::LoadSubtitleDialog).enabled(has),
        Entry::item("Load External Audio...", Action::LoadAudioDialog).enabled(has),
    ];
    root.push(Entry::sub("Open", open));

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
    recent
        .push(Entry::item("Clear Recent Files", Action::ClearRecent).enabled(!c.recent.is_empty()));
    root.push(Entry::sub("Recent Files", recent));

    let mut bookmarks = Vec::new();
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
        bookmarks.push(Entry::item(
            format!("Remove: {label}"),
            Action::RemoveBookmark {
                path: bookmark.path.clone(),
                time: bookmark.time,
            },
        ));
    }
    bookmarks.push(Entry::Sep);
    bookmarks.push(Entry::item("Add Bookmark", Action::AddBookmark).enabled(has));
    root.push(Entry::sub("Bookmarks", bookmarks));

    root.push(Entry::key("Close Playback", "F4", Action::Stop).enabled(has));
    root.push(Entry::Sep);

    // -- Playback ------------------------------------------------------------------------------
    let mut playback = vec![
        Entry::key("Play / Pause", "Space", Action::TogglePause)
            .enabled(has || c.playlist_count > 0),
        Entry::key("Stop", "F4", Action::Stop).enabled(has),
        Entry::key("Previous File", "PgUp", Action::PrevFile).enabled(c.playlist_count > 1),
        Entry::key("Next File", "PgDn", Action::NextFile).enabled(c.playlist_count > 1),
        Entry::Sep,
    ];
    let jump = vec![
        Entry::key(
            format!("Back {} sec", c.seek_small),
            "Left",
            Action::SeekRel(-c.seek_small),
        ),
        Entry::key(
            format!("Forward {} sec", c.seek_small),
            "Right",
            Action::SeekRel(c.seek_small),
        ),
        Entry::key(
            format!("Back {} sec", c.seek_medium),
            "Ctrl+Left",
            Action::SeekRel(-c.seek_medium),
        ),
        Entry::key(
            format!("Forward {} sec", c.seek_medium),
            "Ctrl+Right",
            Action::SeekRel(c.seek_medium),
        ),
        Entry::key(
            format!("Back {} sec", c.seek_large),
            "Shift+Left",
            Action::SeekRel(-c.seek_large),
        ),
        Entry::key(
            format!("Forward {} sec", c.seek_large),
            "Shift+Right",
            Action::SeekRel(c.seek_large),
        ),
        Entry::key(
            format!("Back {} sec", c.seek_huge),
            "Ctrl+Alt+Left",
            Action::SeekRel(-c.seek_huge),
        ),
        Entry::key(
            format!("Forward {} sec", c.seek_huge),
            "Ctrl+Alt+Right",
            Action::SeekRel(c.seek_huge),
        ),
        Entry::Sep,
        Entry::key("Jump to Beginning", "Backspace", Action::SeekAbs(0.0)),
        Entry::key("Go to Time...", "G", Action::OpenGoTo),
        Entry::Sep,
        Entry::item("Previous Subtitle Line", Action::SubSeek(-1)),
        Entry::item("Next Subtitle Line", Action::SubSeek(1)),
    ];
    playback.push(Entry::sub("Jump", jump).enabled(has));
    let speed = vec![
        Entry::key("Faster", "C", Action::ChangeSpeed(0.1)),
        Entry::key("Slower", "X", Action::ChangeSpeed(-0.1)),
        Entry::key("Normal", "Z", Action::SetSpeed(1.0)),
        Entry::Sep,
    ]
    .into_iter()
    .chain(crate::skin::SPEEDS.iter().map(|value| {
        Entry::item(format!("{value}x"), Action::SetSpeed(*value))
            .checked((c.speed - value).abs() < 0.005)
    }))
    .collect();
    playback.push(Entry::sub("Speed", speed));
    let frame_step = vec![
        Entry::key("Next Frame", "F", Action::FrameStep(1)),
        Entry::key("Previous Frame", "D", Action::FrameStep(-1)),
    ];
    playback.push(Entry::sub("Frame Step", frame_step).enabled(has));
    let (a, b) = c.ab;
    let ab = vec![
        Entry::key(
            format!("Set Start Point (A){}", suffix_time(a)),
            "[",
            Action::AbSet('a'),
        ),
        Entry::key(
            format!("Set End Point (B){}", suffix_time(b)),
            "]",
            Action::AbSet('b'),
        ),
        Entry::Sep,
        Entry::key("Clear A-B Repeat", "\\", Action::AbClear).enabled(a.is_some() || b.is_some()),
    ];
    playback.push(Entry::sub("A-B Repeat", ab).enabled(has));
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
                    .unwrap_or_else(|| format!("Chapter {}", index + 1));
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
            "Previous Chapter",
            "Shift+PgUp",
            Action::ChapterStep(-1),
        ));
        chapter_entries.push(Entry::key(
            "Next Chapter",
            "Shift+PgDn",
            Action::ChapterStep(1),
        ));
        playback.push(Entry::sub("Chapters", chapter_entries));
    }
    playback.push(Entry::Sep);
    let repeat = vec![
        Entry::item("Off", Action::SetRepeat("off".into())).checked(c.repeat == "off"),
        Entry::item("Current File", Action::SetRepeat("one".into())).checked(c.repeat == "one"),
        Entry::item("Playlist", Action::SetRepeat("all".into())).checked(c.repeat == "all"),
    ];
    playback.push(Entry::sub("Repeat", repeat));
    playback.push(Entry::item("Shuffle", Action::ToggleShuffle).checked(c.shuffle));
    playback
        .push(Entry::item("Remember Playback Position", Action::ToggleResume).checked(c.resume));
    root.push(Entry::sub("Playback", playback));

    // -- Subtitles -----------------------------------------------------------------------------
    let mut subtitles = vec![
        Entry::key("Show Subtitles", "Alt+H", Action::ToggleSubVisibility).checked(c.sub_visible),
        Entry::key("Load Subtitle...", "Alt+O", Action::LoadSubtitleDialog).enabled(has),
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
        Entry::item("Off", Action::SetSecondarySub(TrackSel::No))
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
    subtitles.push(Entry::sub("Secondary Subtitle", secondary).enabled(has));
    let sync = vec![
        Entry::key("0.5 sec Faster", ",", Action::SubDelay(Some(-0.5))),
        Entry::key("0.5 sec Slower", ".", Action::SubDelay(Some(0.5))),
        Entry::key("Reset Sync", "/", Action::SubDelay(None)),
    ];
    subtitles.push(Entry::sub("Subtitle Sync", sync).enabled(has));
    let size = vec![
        Entry::key("Larger", "Alt+PgUp", Action::SubScale(0.1)),
        Entry::key("Smaller", "Alt+PgDn", Action::SubScale(-0.1)),
        Entry::key("Move Up", "Alt+Up", Action::SubMove(-1)),
        Entry::key("Move Down", "Alt+Down", Action::SubMove(1)),
        Entry::Sep,
        Entry::item("Reset", Action::SubReset),
    ];
    subtitles.push(Entry::sub("Size and Position", size));
    let encoding: Vec<Entry> = crate::skin::ENCODINGS
        .iter()
        .map(|(label, codepage)| {
            Entry::item(*label, Action::SetSubCodepage((*codepage).to_string()))
                .checked(c.sub_codepage == *codepage)
        })
        .collect();
    subtitles.push(Entry::sub("Encoding", encoding));
    subtitles.push(Entry::Sep);
    subtitles.push(Entry::item(
        "Subtitle Settings...",
        Action::ShowPreferences(Some("Subtitles")),
    ));
    root.push(Entry::sub("Subtitles", subtitles));

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
        ("Keep Aspect Ratio", "fit"),
        ("Fill Window (Crop)", "fill"),
        ("Stretch to Window", "stretch"),
        ("Original Size", "original"),
    ]
    .iter()
    .map(|(label, mode)| {
        Entry::item(*label, Action::SetDisplayMode((*mode).to_string()))
            .checked(c.display_mode == *mode)
    })
    .collect();
    let pan_scan = vec![
        Entry::key("Zoom In", "Num 9", Action::Zoom(0.1)),
        Entry::key("Zoom Out", "Num 1", Action::Zoom(-0.1)),
        Entry::Sep,
        Entry::key("Move Left", "Num 4", Action::Pan(-0.02, 0.0)),
        Entry::key("Move Right", "Num 6", Action::Pan(0.02, 0.0)),
        Entry::key("Move Up", "Num 8", Action::Pan(0.0, -0.02)),
        Entry::key("Move Down", "Num 2", Action::Pan(0.0, 0.02)),
        Entry::Sep,
        Entry::key("Reset", "Num 5", Action::ResetGeometry),
    ];
    let rotation = vec![
        Entry::item("0°", Action::Rotate(Some(0))).checked(c.rotation == 0),
        Entry::item("90°", Action::Rotate(Some(90))).checked(c.rotation == 90),
        Entry::item("180°", Action::Rotate(Some(180))).checked(c.rotation == 180),
        Entry::item("270°", Action::Rotate(Some(270))).checked(c.rotation == 270),
        Entry::Sep,
        Entry::key("Rotate 90° Clockwise", "Alt+K", Action::Rotate(None)),
        Entry::key("Flip Horizontally", "Ctrl+Z", Action::ToggleFlip("hflip")).checked(c.flips.0),
        Entry::key("Flip Vertically", "Ctrl+V", Action::ToggleFlip("vflip")).checked(c.flips.1),
    ];
    let mut image: Vec<Entry> = Vec::new();
    for (label, prop, minus, plus) in [
        ("Brightness", "brightness", "W", "E"),
        ("Contrast", "contrast", "R", "T"),
        ("Saturation", "saturation", "Y", "U"),
        ("Hue", "hue", "I", "O"),
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
    image.push(Entry::key("Reset", "Q", Action::ImageReset));
    let hw = vec![
        Entry::item("Use Hardware Decoding", Action::SetHwdec(true)).checked(c.hwdec),
        Entry::item("Software Decoding", Action::SetHwdec(false)).checked(!c.hwdec),
        Entry::Sep,
        Entry::item(
            format!(
                "Active: {}",
                if c.hwdec_current == "no" {
                    "none (software)".to_string()
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
        Entry::sub("Aspect Ratio", aspect),
        Entry::sub("Frame Size", display),
        Entry::sub("Pan & Scan", pan_scan),
        Entry::sub("Rotate and Flip", rotation),
        Entry::sub("Image Adjustments", image),
        Entry::Sep,
        Entry::item("Deinterlace", Action::ToggleDeinterlace).checked(c.deinterlace),
        Entry::sub("Hardware Acceleration", hw),
    ];
    if videos.len() > 1 {
        video.push(Entry::sub(
            "Video Track",
            tracks_menu(TrackKind::Video, c.tracks, false, has),
        ));
    }
    root.push(Entry::sub("Video", video));

    // -- Audio ---------------------------------------------------------------------------------
    let mut audio_out = vec![
        Entry::sub(
            "Select Audio Stream",
            tracks_menu(TrackKind::Audio, c.tracks, false, has),
        ),
        Entry::item("Load External Audio...", Action::LoadAudioDialog).enabled(has),
        Entry::Sep,
        Entry::key("Volume Up", "Up", Action::VolumeStep(1)),
        Entry::key("Volume Down", "Down", Action::VolumeStep(-1)),
        Entry::key("Mute", "M", Action::ToggleMute).checked(c.mute),
        Entry::Sep,
    ];
    let audio_sync = vec![
        Entry::key(
            "0.05 sec Faster",
            "Shift+,",
            Action::AudioDelay(Some(-0.05)),
        ),
        Entry::key("0.05 sec Slower", "Shift+.", Action::AudioDelay(Some(0.05))),
        Entry::key("Reset Sync", "Shift+/", Action::AudioDelay(None)),
    ];
    audio_out.push(Entry::sub("Audio Sync", audio_sync).enabled(has));
    let devices: Vec<Entry> = c
        .audio_devices
        .iter()
        .filter_map(|device| {
            let name = device.get("name").as_str()?.to_string();
            let label = if name == "auto" {
                "Auto (system default)".to_string()
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
    audio_out.push(Entry::sub("Output Device", devices));
    audio_out.push(Entry::item("Normalize Volume", Action::ToggleNormalize).checked(c.normalize));
    audio_out.push(Entry::item(
        "Equalizer...",
        Action::ShowControlPanel(Some("Audio")),
    ));
    root.push(Entry::sub("Audio", audio_out));

    // -- Misc ----------------------------------------------------------------------------------
    let misc = vec![
        Entry::key(
            "Save Current Frame",
            "Ctrl+E",
            Action::Screenshot("video", false),
        )
        .enabled(has),
        Entry::key(
            "Save Current Frame (With Subtitles)",
            "Ctrl+Alt+E",
            Action::Screenshot("subtitles", false),
        )
        .enabled(has),
        Entry::key(
            "Copy Current Frame",
            "Ctrl+C",
            Action::Screenshot("video", true),
        )
        .enabled(has),
        Entry::key(
            "Copy Current Frame (With Subtitles)",
            "Ctrl+Alt+C",
            Action::Screenshot("subtitles", true),
        )
        .enabled(has),
        Entry::item("Open Screenshot Folder", Action::OpenScreenshotFolder),
        Entry::Sep,
        Entry::sub(
            "On Top",
            vec![
                Entry::item("Never", Action::SetOnTop("never".into())).checked(c.on_top == "never"),
                Entry::item("Always", Action::SetOnTop("always".into()))
                    .checked(c.on_top == "always"),
                Entry::item("While Playing", Action::SetOnTop("playing".into()))
                    .checked(c.on_top == "playing"),
            ],
        ),
        Entry::key("Playback Information (OSD)", "Tab", Action::ToggleInfo).checked(c.info_visible),
    ];
    root.push(Entry::sub("Misc", misc));
    root.push(Entry::Sep);

    let window_size = vec![
        Entry::key("50%", "1", Action::SetWindowScale(50)).enabled(has),
        Entry::key("100%", "2", Action::SetWindowScale(100)).enabled(has),
        Entry::key("150%", "3", Action::SetWindowScale(150)).enabled(has),
        Entry::key("200%", "4", Action::SetWindowScale(200)).enabled(has),
        Entry::Sep,
        Entry::item("Fit Window to Video on Open", Action::ToggleAutofit).checked(c.autofit),
    ];
    root.push(Entry::sub("Window Size", window_size));
    root.push(Entry::key("Full Screen", "Enter", Action::ToggleFullscreen).checked(c.fullscreen));
    root.push(Entry::Sep);
    root.push(Entry::key(
        "Preferences...",
        "F5",
        Action::ShowPreferences(None),
    ));
    root.push(Entry::key("Playlist...", "F6", Action::TogglePlaylist));
    root.push(Entry::item(
        "Control Panel...",
        Action::ShowControlPanel(None),
    ));
    root.push(Entry::key(
        "Playback/System Info...",
        "Ctrl+F1",
        Action::ShowMediaInfo,
    ));
    root.push(Entry::key("About...", "F1", Action::ShowAbout));
    root.push(Entry::Sep);
    root.push(Entry::key("Exit", quit_shortcut(), Action::Quit));
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

pub fn track_menu(kind: TrackKind, tracks: &[Value], has_file: bool) -> Vec<Entry> {
    let mut entries = tracks_menu(kind, tracks, kind == TrackKind::Sub, has_file);
    if kind == TrackKind::Sub {
        entries.push(Entry::Sep);
        entries.push(
            Entry::key("Load Subtitle...", "Alt+O", Action::LoadSubtitleDialog).enabled(has_file),
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
}

impl MenuState {
    pub fn open_at(&mut self, pos: Pos2) {
        self.pos = Some(pos);
        self.path.clear();
    }

    pub fn close(&mut self) {
        self.pos = None;
        self.path.clear();
    }

    pub fn is_open(&self) -> bool {
        self.pos.is_some()
    }
}

/// Render the open menu. Returns the triggered action, if any; the menu closes itself then.
pub fn show(
    ui: &mut Ui,
    state: &mut MenuState,
    id_salt: &str,
    entries: &[Entry],
) -> Option<Action> {
    let Some(root_pos) = state.pos else {
        return None;
    };
    let ctx = ui.ctx().clone();

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.close();
        return None;
    }

    let mut action: Option<Action> = None;
    let mut rects: Vec<Rect> = Vec::new();
    let mut path = std::mem::take(&mut state.path);
    let mut pos = root_pos;
    let mut level = 0usize;
    loop {
        let Some(slice) = entries_at(entries, &path[..level]) else {
            break;
        };
        let out = panel(
            &ctx,
            id_salt,
            level,
            pos,
            slice,
            path.get(level).copied(),
            &mut action,
            &mut rects,
        );
        if let Some(clicked) = out.clicked_sub {
            open_submenu(&mut path, level, clicked);
            if let Some(item_rect) = out.clicked_rect {
                pos = child_pos(&ctx, entries, &path, level, item_rect);
            }
            level += 1;
            if level > 6 {
                break;
            }
            continue;
        }
        if let Some(item_rect) = out.active_rect {
            if level < path.len() {
                pos = child_pos(&ctx, entries, &path, level, item_rect);
                level += 1;
                if level > 6 {
                    break;
                }
                continue;
            }
        }
        path.truncate(level);
        break;
    }

    if ctx.input(|i| i.pointer.primary_pressed()) {
        let over_menu = ctx
            .pointer_hover_pos()
            .is_some_and(|p| rects.iter().any(|r| r.contains(p)));
        if !over_menu {
            state.close();
        }
    }

    state.path = path;
    if action.is_some() {
        state.close();
    }
    let _ = ui;
    action
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

fn panel(
    ctx: &Context,
    id_salt: &str,
    level: usize,
    pos: Pos2,
    entries: &[Entry],
    active_sub: Option<usize>,
    action: &mut Option<Action>,
    rects: &mut Vec<Rect>,
) -> PanelOut {
    let colors = skin::active_colors();
    let font_id = skin::font(13.0, false);
    let width = panel_width(ctx, entries);
    let height = panel_height(entries);

    let mut out = PanelOut {
        clicked_sub: None,
        clicked_rect: None,
        active_rect: None,
    };
    let area = egui::Area::new(Id::new("macplayer-menu").with(id_salt).with(level))
        .fixed_pos(pos)
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
                    color: colors.shadow,
                }
                .as_shape(panel_rect, corner_radius),
            );
            painter.rect_filled(panel_rect, corner_radius, colors.menu_bg);
            painter.rect_stroke(
                panel_rect,
                corner_radius,
                Stroke::new(1.0, colors.dialog_line),
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
                            colors.line,
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
                let hovered = response.hovered() && enabled;
                if hovered {
                    painter.rect_filled(item_rect.shrink2(vec2(2.0, 1.0)), 4.0, colors.menu_sel);
                    painter.rect_stroke(
                        item_rect.shrink2(vec2(2.0, 1.0)),
                        4.0,
                        Stroke::new(1.0, colors.focus),
                        egui::StrokeKind::Outside,
                    );
                }
                let text_color = if !enabled {
                    colors.icon_off
                } else if hovered {
                    colors.text_bright
                } else {
                    colors.text_e4
                };
                if checked.unwrap_or(false) {
                    painter.circle_filled(
                        pos2(panel_pos.x + 12.0, item_rect.center().y),
                        3.0,
                        colors.accent,
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
                    painter.text(
                        pos2(item_rect.right() - 16.0, item_rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        "⏵",
                        font_id.clone(),
                        text_color,
                    );
                } else if !shortcut.is_empty() {
                    painter.text(
                        pos2(item_rect.right() - PAD_RIGHT + 10.0, item_rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        shortcut,
                        font_id.clone(),
                        if hovered {
                            colors.text_e4
                        } else {
                            colors.text_dim
                        },
                    );
                }

                if let Entry::Sub { enabled: true, .. } = entry {
                    if active_sub == Some(index) {
                        out.active_rect = Some(item_rect);
                    }
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

pub fn track_ids(tracks: &[Value], kind: TrackKind) -> Vec<i64> {
    tracks
        .iter()
        .filter(|t| t.get("type").as_str() == Some(kind_name(kind)))
        .filter_map(|t| t.get("id").as_i64())
        .collect()
}

#[cfg(test)]
mod tests {
    use egui::{Rect, pos2, vec2};

    use super::{display_shortcut, open_submenu, quit_shortcut, submenu_pos};

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
