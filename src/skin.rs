//! Skin: themes, metrics, fonts, shared tables and hand-drawn vector glyphs (no bitmap assets).
//!
//! MacPlayer's custom skin. Glyphs are drawn from primitives on a stable pixel grid;
//! `(cx, cy)` is the centre of the button and the coordinate system is y-down.
//! Colors always flow through `ThemeColors`; the active theme and accent are process
//! globals set once at startup and whenever preferences change.

use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Color32, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::value::Value;

// -- colours -------------------------------------------------------------------------------------

pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x0f);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x24, 0x28, 0x31);
pub const BG_PRESS: Color32 = Color32::from_rgb(0x10, 0x12, 0x16);
pub const VIDEO_BG: Color32 = Color32::from_rgb(0x08, 0x0a, 0x0d);
pub const LINE: Color32 = Color32::from_rgb(0x29, 0x2d, 0x36);
pub const ACCENT: Color32 = Color32::from_rgb(0xe6, 0xbd, 0x72);
pub const KNOB: Color32 = Color32::from_rgb(0xf3, 0xf0, 0xe7);
pub const ICON: Color32 = KNOB;
pub const ICON_HOVER: Color32 = Color32::WHITE;
pub const ICON_DIM: Color32 = Color32::from_rgb(0x86, 0x8c, 0x98);
pub const ICON_OFF: Color32 = Color32::from_rgb(0x4c, 0x52, 0x5d);
pub const TEXT: Color32 = Color32::from_rgb(0xc7, 0xc8, 0xc5);
pub const TEXT_BRIGHT: Color32 = KNOB;
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x7c, 0x82, 0x8d);
pub const LOGO: Color32 = ACCENT;
pub const PANEL: Color32 = Color32::from_rgb(0x12, 0x14, 0x19);
pub const SELECT: Color32 = Color32::from_rgb(0x28, 0x2d, 0x38);
pub const DANGER: Color32 = Color32::from_rgb(0xe0, 0x4a, 0x3f);
pub const BUFFER: Color32 = ICON_OFF;

// Dialog chrome.
pub const DIALOG_BG: Color32 = Color32::from_rgb(0x17, 0x1a, 0x20);
pub const DIALOG_LINE: Color32 = Color32::from_rgb(0x32, 0x37, 0x42);
pub const DIALOG_PANE: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
pub const BASE: Color32 = Color32::from_rgb(0x14, 0x17, 0x1c);
pub const BUTTON_BG: Color32 = Color32::from_rgb(0x20, 0x24, 0x2c);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x31, 0x36, 0x42);
pub const BUTTON_PRESS: Color32 = Color32::from_rgb(0x18, 0x1b, 0x21);
pub const MENU_BG: Color32 = Color32::from_rgb(0x18, 0x1b, 0x21);
pub const MENU_SEL: Color32 = Color32::from_rgb(0x2d, 0x33, 0x3e);
pub const TAB_BG: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x27);
pub const TAB_SEL: Color32 = Color32::from_rgb(0x26, 0x2b, 0x35);
pub const TEXT_E4: Color32 = Color32::from_rgb(0xe4, 0xe4, 0xe4);
pub const TEXT_A8: Color32 = Color32::from_rgb(0xb4, 0xb8, 0xc0);
pub const TEXT_9A: Color32 = Color32::from_rgb(0x8f, 0x95, 0xa0);
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xff, 0x6a, 0x5e);
pub const FOCUS: Color32 = Color32::from_rgb(0xf1, 0xce, 0x8a);
pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0xd0);

// Idle-screen gradient: four corner stops plus the center of the corner fan,
// and the faint decorative dot grid on top of it.
pub const LOGO_GRAD_TL: Color32 = Color32::from_rgb(0x16, 0x2d, 0x3b);
pub const LOGO_GRAD_TR: Color32 = Color32::from_rgb(0x45, 0x2a, 0x2a);
pub const LOGO_GRAD_BR: Color32 = Color32::from_rgb(0x08, 0x0b, 0x0f);
pub const LOGO_GRAD_BL: Color32 = Color32::from_rgb(0x0d, 0x2e, 0x37);
pub const LOGO_GRAD_CENTER: Color32 = Color32::from_rgb(0x2d, 0x24, 0x31);
pub const LOGO_DOT: Color32 = Color32::from_rgba_unmultiplied_const(0xe5, 0xe3, 0xda, 20);

// Floating chrome: gradient scrims over the video and the translucent drawer surface.
pub const SCRIM_TOP: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 115);
pub const SCRIM_BOTTOM: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 185);
pub const DRAWER_FILL: Color32 = Color32::from_rgba_premultiplied(19, 20, 24, 240);

// -- themes and accents --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    pub fn from_str(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            _ => Self::Dark,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Accent {
    Gold,
    Blue,
    Green,
    Rose,
    Graphite,
}

impl Accent {
    pub fn from_str(value: &str) -> Self {
        match value {
            "blue" => Self::Blue,
            "green" => Self::Green,
            "rose" => Self::Rose,
            "graphite" => Self::Graphite,
            _ => Self::Gold,
        }
    }
}

/// Accent id in `ThemeColors`: the identity color, a softer focus variant and the
/// idle-screen watermark color.
const ACCENT_SETS: &[(Accent, Color32, Color32)] = &[
    (
        Accent::Gold,
        Color32::from_rgb(0xe6, 0xbd, 0x72),
        Color32::from_rgb(0xf1, 0xce, 0x8a),
    ),
    (
        Accent::Blue,
        Color32::from_rgb(0x6a, 0xa7, 0xe8),
        Color32::from_rgb(0x8d, 0xbd, 0xf0),
    ),
    (
        Accent::Green,
        Color32::from_rgb(0x7f, 0xc9, 0x8a),
        Color32::from_rgb(0x9b, 0xd6, 0xa4),
    ),
    (
        Accent::Rose,
        Color32::from_rgb(0xe8, 0x8a, 0x9a),
        Color32::from_rgb(0xf0, 0xa5, 0xb2),
    ),
    (
        Accent::Graphite,
        Color32::from_rgb(0x9a, 0xa3, 0xb2),
        Color32::from_rgb(0xb3, 0xba, 0xc6),
    ),
];

static ACTIVE_THEME: AtomicU8 = AtomicU8::new(0);
static ACTIVE_ACCENT: AtomicU8 = AtomicU8::new(0);

pub fn set_theme(theme: Theme) {
    ACTIVE_THEME.store(u8::from(theme == Theme::Light), Ordering::Release);
}

pub fn set_accent(accent: Accent) {
    let index = ACCENT_SETS
        .iter()
        .position(|(candidate, _, _)| *candidate == accent)
        .unwrap_or(0);
    ACTIVE_ACCENT.store(index as u8, Ordering::Release);
}

pub fn current_theme() -> Theme {
    match ACTIVE_THEME.load(Ordering::Acquire) {
        1 => Theme::Light,
        _ => Theme::Dark,
    }
}

// Light-theme constants. The video letterbox intentionally stays dark: letterboxing a
// bright video with a bright surface looks broken, and mpv fills the rest either way.
pub const LIGHT_BG: Color32 = Color32::from_rgb(0xf2, 0xf1, 0xed);
pub const LIGHT_BG_HOVER: Color32 = Color32::from_rgb(0xe3, 0xe1, 0xda);
pub const LIGHT_BG_PRESS: Color32 = Color32::from_rgb(0xe9, 0xe7, 0xe1);
pub const LIGHT_LINE: Color32 = Color32::from_rgb(0xd5, 0xd2, 0xc9);
pub const LIGHT_KNOB: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x45);
pub const LIGHT_ICON: Color32 = LIGHT_KNOB;
pub const LIGHT_ICON_HOVER: Color32 = Color32::from_rgb(0x10, 0x12, 0x16);
pub const LIGHT_ICON_DIM: Color32 = Color32::from_rgb(0x6d, 0x72, 0x80);
pub const LIGHT_ICON_OFF: Color32 = Color32::from_rgb(0xb9, 0xbc, 0xc4);
pub const LIGHT_TEXT: Color32 = Color32::from_rgb(0x3c, 0x3f, 0x46);
pub const LIGHT_TEXT_BRIGHT: Color32 = Color32::from_rgb(0x17, 0x18, 0x1c);
pub const LIGHT_TEXT_DIM: Color32 = LIGHT_ICON_DIM;
pub const LIGHT_PANEL: Color32 = Color32::from_rgb(0xfa, 0xf9, 0xf6);
pub const LIGHT_SELECT: Color32 = Color32::from_rgb(0xec, 0xe7, 0xd9);
pub const LIGHT_DANGER: Color32 = Color32::from_rgb(0xc9, 0x3a, 0x2f);
pub const LIGHT_BUFFER: Color32 = LIGHT_ICON_OFF;
pub const LIGHT_DIALOG_BG: Color32 = Color32::from_rgb(0xfb, 0xfa, 0xf7);
pub const LIGHT_DIALOG_LINE: Color32 = LIGHT_LINE;
pub const LIGHT_DIALOG_PANE: Color32 = Color32::from_rgb(0xf1, 0xef, 0xe9);
pub const LIGHT_BASE: Color32 = Color32::from_rgb(0xed, 0xec, 0xe7);
pub const LIGHT_BUTTON_BG: Color32 = LIGHT_BG_PRESS;
pub const LIGHT_BUTTON_HOVER: Color32 = Color32::from_rgb(0xdd, 0xd9, 0xd0);
pub const LIGHT_BUTTON_PRESS: Color32 = LIGHT_BG;
pub const LIGHT_MENU_BG: Color32 = LIGHT_DIALOG_BG;
pub const LIGHT_MENU_SEL: Color32 = LIGHT_SELECT;
pub const LIGHT_TAB_BG: Color32 = LIGHT_DIALOG_PANE;
pub const LIGHT_TAB_SEL: Color32 = Color32::from_rgb(0xe6, 0xe2, 0xd6);
pub const LIGHT_TEXT_E4: Color32 = Color32::from_rgb(0x26, 0x28, 0x2e);
pub const LIGHT_TEXT_A8: Color32 = Color32::from_rgb(0x4c, 0x50, 0x5a);
pub const LIGHT_TEXT_9A: Color32 = LIGHT_ICON_DIM;
pub const LIGHT_CLOSE_HOVER: Color32 = Color32::from_rgb(0xd6, 0x3b, 0x2f);
pub const LIGHT_SHADOW: Color32 = Color32::from_rgba_premultiplied(0x40, 0x38, 0x28, 0x5e);
pub const LIGHT_VIDEO_BG: Color32 = Color32::from_rgb(0x10, 0x12, 0x16);
pub const LIGHT_LOGO_GRAD_TL: Color32 = Color32::from_rgb(0xdf, 0xe7, 0xea);
pub const LIGHT_LOGO_GRAD_TR: Color32 = Color32::from_rgb(0xea, 0xdf, 0xd8);
pub const LIGHT_LOGO_GRAD_BR: Color32 = Color32::from_rgb(0xf2, 0xf1, 0xed);
pub const LIGHT_LOGO_GRAD_BL: Color32 = Color32::from_rgb(0xe0, 0xe9, 0xe6);
pub const LIGHT_LOGO_GRAD_CENTER: Color32 = Color32::from_rgb(0xe9, 0xe2, 0xe4);
pub const LIGHT_LOGO_DOT: Color32 = Color32::from_rgba_unmultiplied_const(0x33, 0x30, 0x28, 18);
pub const LIGHT_SCRIM_TOP: Color32 = Color32::from_rgba_premultiplied(118, 117, 115, 115);
pub const LIGHT_SCRIM_BOTTOM: Color32 = Color32::from_rgba_premultiplied(117, 116, 114, 185);
pub const LIGHT_DRAWER_FILL: Color32 = Color32::from_rgba_premultiplied(239, 238, 235, 243);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemeColors {
    pub bg: Color32,
    pub bg_hover: Color32,
    pub bg_press: Color32,
    pub video_bg: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub knob: Color32,
    pub icon: Color32,
    pub icon_hover: Color32,
    pub icon_dim: Color32,
    pub icon_off: Color32,
    pub text: Color32,
    pub text_bright: Color32,
    pub text_dim: Color32,
    pub logo: Color32,
    pub panel: Color32,
    pub select: Color32,
    pub danger: Color32,
    pub buffer: Color32,
    pub dialog_bg: Color32,
    pub dialog_line: Color32,
    pub dialog_pane: Color32,
    pub base: Color32,
    pub button_bg: Color32,
    pub button_hover: Color32,
    pub button_press: Color32,
    pub menu_bg: Color32,
    pub menu_sel: Color32,
    pub tab_bg: Color32,
    pub tab_sel: Color32,
    pub text_e4: Color32,
    pub text_a8: Color32,
    pub text_9a: Color32,
    pub close_hover: Color32,
    pub focus: Color32,
    pub shadow: Color32,
    pub logo_grad_tl: Color32,
    pub logo_grad_tr: Color32,
    pub logo_grad_br: Color32,
    pub logo_grad_bl: Color32,
    pub logo_grad_center: Color32,
    pub logo_dot: Color32,
    /// Floating chrome scrims over the video (top and bottom gradients).
    pub scrim_top: Color32,
    pub scrim_bottom: Color32,
    /// Translucent playlist drawer surface.
    pub drawer_fill: Color32,
}

impl ThemeColors {
    pub const fn classic() -> Self {
        Self {
            bg: BG,
            bg_hover: BG_HOVER,
            bg_press: BG_PRESS,
            video_bg: VIDEO_BG,
            line: LINE,
            accent: ACCENT,
            knob: KNOB,
            icon: ICON,
            icon_hover: ICON_HOVER,
            icon_dim: ICON_DIM,
            icon_off: ICON_OFF,
            text: TEXT,
            text_bright: TEXT_BRIGHT,
            text_dim: TEXT_DIM,
            logo: LOGO,
            panel: PANEL,
            select: SELECT,
            danger: DANGER,
            buffer: BUFFER,
            dialog_bg: DIALOG_BG,
            dialog_line: DIALOG_LINE,
            dialog_pane: DIALOG_PANE,
            base: BASE,
            button_bg: BUTTON_BG,
            button_hover: BUTTON_HOVER,
            button_press: BUTTON_PRESS,
            menu_bg: MENU_BG,
            menu_sel: MENU_SEL,
            tab_bg: TAB_BG,
            tab_sel: TAB_SEL,
            text_e4: TEXT_E4,
            text_a8: TEXT_A8,
            text_9a: TEXT_9A,
            close_hover: CLOSE_HOVER,
            focus: FOCUS,
            shadow: SHADOW,
            logo_grad_tl: LOGO_GRAD_TL,
            logo_grad_tr: LOGO_GRAD_TR,
            logo_grad_br: LOGO_GRAD_BR,
            logo_grad_bl: LOGO_GRAD_BL,
            logo_grad_center: LOGO_GRAD_CENTER,
            logo_dot: LOGO_DOT,
            scrim_top: SCRIM_TOP,
            scrim_bottom: SCRIM_BOTTOM,
            drawer_fill: DRAWER_FILL,
        }
    }

    /// Light counterpart of [`Self::classic`], using the default gold accent; the
    /// accent fields are patched by [`active_colors`].
    pub const fn light() -> Self {
        Self {
            bg: LIGHT_BG,
            bg_hover: LIGHT_BG_HOVER,
            bg_press: LIGHT_BG_PRESS,
            video_bg: LIGHT_VIDEO_BG,
            line: LIGHT_LINE,
            accent: ACCENT,
            knob: LIGHT_KNOB,
            icon: LIGHT_ICON,
            icon_hover: LIGHT_ICON_HOVER,
            icon_dim: LIGHT_ICON_DIM,
            icon_off: LIGHT_ICON_OFF,
            text: LIGHT_TEXT,
            text_bright: LIGHT_TEXT_BRIGHT,
            text_dim: LIGHT_TEXT_DIM,
            logo: LOGO,
            panel: LIGHT_PANEL,
            select: LIGHT_SELECT,
            danger: LIGHT_DANGER,
            buffer: LIGHT_BUFFER,
            dialog_bg: LIGHT_DIALOG_BG,
            dialog_line: LIGHT_DIALOG_LINE,
            dialog_pane: LIGHT_DIALOG_PANE,
            base: LIGHT_BASE,
            button_bg: LIGHT_BUTTON_BG,
            button_hover: LIGHT_BUTTON_HOVER,
            button_press: LIGHT_BUTTON_PRESS,
            menu_bg: LIGHT_MENU_BG,
            menu_sel: LIGHT_MENU_SEL,
            tab_bg: LIGHT_TAB_BG,
            tab_sel: LIGHT_TAB_SEL,
            text_e4: LIGHT_TEXT_E4,
            text_a8: LIGHT_TEXT_A8,
            text_9a: LIGHT_TEXT_9A,
            close_hover: LIGHT_CLOSE_HOVER,
            focus: FOCUS,
            shadow: LIGHT_SHADOW,
            logo_grad_tl: LIGHT_LOGO_GRAD_TL,
            logo_grad_tr: LIGHT_LOGO_GRAD_TR,
            logo_grad_br: LIGHT_LOGO_GRAD_BR,
            logo_grad_bl: LIGHT_LOGO_GRAD_BL,
            logo_grad_center: LIGHT_LOGO_GRAD_CENTER,
            logo_dot: LIGHT_LOGO_DOT,
            scrim_top: LIGHT_SCRIM_TOP,
            scrim_bottom: LIGHT_SCRIM_BOTTOM,
            drawer_fill: LIGHT_DRAWER_FILL,
        }
    }
}

/// The active palette: theme from `set_theme`, accent fields from `set_accent`.
pub fn active_colors() -> ThemeColors {
    let (theme, accent, focus) = {
        let theme = current_theme();
        let index = {
            let len = ACCENT_SETS.len() as u8;
            (ACTIVE_ACCENT.load(Ordering::Acquire) % len.max(1)) as usize
        };
        let (_, accent, focus) = ACCENT_SETS[index];
        (theme, accent, focus)
    };
    match theme {
        Theme::Dark => ThemeColors {
            accent,
            logo: accent,
            focus,
            ..ThemeColors::classic()
        },
        Theme::Light => ThemeColors {
            accent,
            logo: accent,
            focus,
            ..ThemeColors::light()
        },
    }
}

// -- metrics (logical px) ------------------------------------------------------------------------

/// Floating control bar height (seek row + transport row).
pub const CTRL_H: f32 = 64.0;
/// Narrowest playlist drawer.
pub const MIN_PLAYLIST_W: f32 = 292.0;

// -- fonts ---------------------------------------------------------------------------------------

/// Vazirmatn Regular — bundled for Persian UI text and Persian media titles.
/// Source: https://github.com/rastikerdar/vazirmatn — license: SIL OFL 1.1 (assets/fonts/OFL.txt).
const FONT_FA: &[u8] = include_bytes!("../assets/fonts/Vazirmatn-Regular.ttf");

pub fn font(px: f32, bold: bool) -> FontId {
    let _ = bold; // egui "strong" text is a color change; mpv draws the bold subtitles itself.
    FontId::new(px, FontFamily::Proportional)
}

pub fn mono(px: f32) -> FontId {
    FontId::new(px, FontFamily::Monospace)
}

/// Register fonts for the chosen UI language. Latin stays on egui's default fonts;
/// Vazirmatn either leads the family (Persian UI) or sits at the end as a fallback so
/// Persian media titles render correctly even in the English UI.
pub fn apply_fonts(ctx: &egui::Context, persian_ui: bool) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "vazirmatn".into(),
        Arc::new(egui::FontData::from_static(FONT_FA)),
    );
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    if persian_ui {
        proportional.insert(0, "vazirmatn".into());
    } else {
        proportional.push("vazirmatn".into());
    }
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("vazirmatn".into());
    ctx.set_fonts(fonts);
}

// -- shared tables -------------------------------------------------------------------------------

pub const LANGS: &[(&str, &str)] = &[
    ("en", "English"),
    ("eng", "English"),
    ("fa", "Persian"),
    ("per", "Persian"),
    ("fas", "Persian"),
    ("ar", "Arabic"),
    ("ara", "Arabic"),
    ("fr", "French"),
    ("fre", "French"),
    ("fra", "French"),
    ("de", "German"),
    ("ger", "German"),
    ("deu", "German"),
    ("es", "Spanish"),
    ("spa", "Spanish"),
    ("it", "Italian"),
    ("ita", "Italian"),
    ("ru", "Russian"),
    ("rus", "Russian"),
    ("ja", "Japanese"),
    ("jpn", "Japanese"),
    ("ko", "Korean"),
    ("kor", "Korean"),
    ("zh", "Chinese"),
    ("chi", "Chinese"),
    ("zho", "Chinese"),
    ("tr", "Turkish"),
    ("tur", "Turkish"),
    ("pt", "Portuguese"),
    ("por", "Portuguese"),
    ("hi", "Hindi"),
    ("hin", "Hindi"),
    ("nl", "Dutch"),
    ("dut", "Dutch"),
    ("nld", "Dutch"),
    ("sv", "Swedish"),
    ("swe", "Swedish"),
    ("pl", "Polish"),
    ("pol", "Polish"),
    ("he", "Hebrew"),
    ("heb", "Hebrew"),
    ("el", "Greek"),
    ("gre", "Greek"),
    ("ell", "Greek"),
    ("uk", "Ukrainian"),
    ("ukr", "Ukrainian"),
    ("id", "Indonesian"),
    ("ind", "Indonesian"),
    ("th", "Thai"),
    ("tha", "Thai"),
    ("vi", "Vietnamese"),
    ("vie", "Vietnamese"),
    ("und", "Unknown"),
];

pub fn lang_name(code: &str) -> Option<&'static str> {
    LANGS
        .iter()
        .find(|(key, _)| *key == code)
        .map(|(_, name)| *name)
}

pub const ENCODINGS: &[(&str, &str)] = &[
    ("Auto-detect", "auto"),
    ("UTF-8", "utf-8"),
    ("Arabic / Persian (Windows-1256)", "cp1256"),
    ("Cyrillic (Windows-1251)", "cp1251"),
    ("Central European (Windows-1250)", "cp1250"),
    ("Western (Windows-1252)", "cp1252"),
    ("Greek (Windows-1253)", "cp1253"),
    ("Turkish (Windows-1254)", "cp1254"),
    ("Hebrew (Windows-1255)", "cp1255"),
    ("Chinese Simplified (GB18030)", "gb18030"),
    ("Chinese Traditional (Big5)", "big5"),
    ("Japanese (Shift-JIS)", "shift_jis"),
    ("Korean (CP949)", "cp949"),
];

pub const ASPECTS: &[(&str, &str)] = &[
    ("Default", "-1"),
    ("4:3", "4:3"),
    ("16:9", "16:9"),
    ("16:10", "16:10"),
    ("1.85:1", "1.85"),
    ("2.35:1", "2.35"),
    ("2.39:1", "2.39"),
    ("1:1", "1"),
];

pub const SPEEDS: &[f64] = &[0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];

pub const EQ_FREQS: &[f64] = &[
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

pub const EQ_PRESETS: &[(&str, [f64; 10])] = &[
    ("Flat", [0.0; 10]),
    (
        "Bass Boost",
        [7.0, 6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    (
        "Treble Boost",
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 7.0],
    ),
    (
        "Vocal",
        [-3.0, -2.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0],
    ),
    ("Rock", [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 1.0, 3.0, 4.0, 5.0]),
    (
        "Pop",
        [-1.0, 0.0, 2.0, 4.0, 4.0, 2.0, 0.0, -1.0, -1.0, -1.0],
    ),
    (
        "Classical",
        [4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0],
    ),
    ("Movie", [4.0, 3.0, 1.0, 0.0, 1.0, 2.0, 3.0, 2.0, 1.0, 1.0]),
];

/// "3. Title - Name  [H264]  (default)" for a `track-list` entry.
pub fn track_label(track: &Value) -> String {
    let lang = track.get("lang").as_str().unwrap_or("").to_lowercase();
    let name = lang_name(&lang).map(str::to_string).unwrap_or_else(|| {
        if lang.is_empty() {
            String::new()
        } else {
            lang.to_uppercase()
        }
    });
    let mut title = track.get("title").as_str().unwrap_or("").to_string();
    if track.get("external").as_bool()
        && let Some(file) = track.get("external-filename").as_str()
    {
        title = crate::util::title_of(file);
    }
    let parts: Vec<&str> = [title.as_str(), name.as_str()]
        .iter()
        .copied()
        .filter(|p| !p.is_empty())
        .collect();
    let mut label = format!(
        "{}. {}",
        track.get("id").as_i64().unwrap_or(0),
        if parts.is_empty() {
            crate::i18n::tr("Track").to_string()
        } else {
            parts.join(" - ")
        }
    );
    if let Some(codec) = track.get("codec").as_str()
        && !codec.is_empty()
    {
        label.push_str(&format!("  [{}]", codec.to_uppercase()));
    }
    let flags: Vec<&str> = ["default", "forced", "external"]
        .iter()
        .filter(|flag| track.get(flag).as_bool())
        .copied()
        .collect();
    if !flags.is_empty() {
        label.push_str(&format!("  ({})", flags.join(", ")));
    }
    label
}

// -- glyphs --------------------------------------------------------------------------------------

/// Draw one skinned icon centred on `(cx, cy)`. `bg` is the button background, used by glyphs
/// that punch a hole (the gear's axle).
pub fn draw_icon(p: &Painter, name: &str, cx: f32, cy: f32, color: Color32, bg: Color32) {
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_filled(
            Rect::from_min_size(pos2(cx + x, cy + y), vec2(w, h)),
            0.0,
            color,
        );
    };
    let faint = |x: f32, y: f32, w: f32, h: f32, alpha: f32| {
        let tint =
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha * 255.0) as u8);
        p.rect_filled(
            Rect::from_min_size(pos2(cx + x, cy + y), vec2(w, h)),
            0.0,
            tint,
        );
    };
    let poly = |pts: &[(f32, f32)]| {
        let points: Vec<Pos2> = pts.iter().map(|&(x, y)| pos2(cx + x, cy + y)).collect();
        p.add(Shape::convex_polygon(points, color, Stroke::NONE));
    };
    let corners = |pixels: &[(f32, f32)]| {
        for sx in [1.0f32, -1.0] {
            for sy in [1.0f32, -1.0] {
                for &(x, y) in pixels {
                    rect(x * sx, y * sy, 1.0, 1.0);
                }
            }
        }
    };

    match name {
        "play" => poly(&[
            (-3.0, -5.0),
            (-1.28, -5.0),
            (5.6, 0.0),
            (-1.28, 5.0),
            (-3.0, 5.0),
        ]),
        "pause" => {
            rect(-4.0, -5.0, 3.0, 10.0);
            rect(1.0, -5.0, 3.0, 10.0);
        }
        "stop" => {
            p.rect_filled(
                Rect::from_min_size(pos2(cx - 4.0, cy - 4.0), vec2(8.0, 8.0)),
                0.8,
                color,
            );
        }
        "prev" => {
            rect(-4.0, -4.0, 2.0, 8.0);
            poly(&[
                (4.25, -4.0),
                (3.05, -4.0),
                (-2.55, 0.0),
                (3.05, 4.0),
                (4.25, 4.0),
            ]);
        }
        "next" => {
            poly(&[
                (-4.25, -4.0),
                (-3.05, -4.0),
                (2.55, 0.0),
                (-3.05, 4.0),
                (-4.25, 4.0),
            ]);
            rect(2.0, -4.0, 2.0, 8.0);
        }
        "eject" => {
            poly(&[
                (0.0, -4.2),
                (4.0, -0.2),
                (4.0, 1.36),
                (-4.0, 1.36),
                (-4.0, -0.2),
            ]);
            rect(-4.0, 3.0, 8.0, 1.0);
        }
        "menu" => {
            for dy in [-4.0f32, 0.0, 4.0] {
                rect(-6.0, dy, 12.0, 1.0);
            }
        }
        "playlist" => {
            for dy in [-5.0f32, 0.0, 5.0] {
                p.circle_filled(pos2(cx - 6.0, cy + dy), 1.2, color);
                rect(-3.0, dy - 1.0, 9.0, 2.0);
            }
        }
        "gear" => gear(p, cx, cy, color, bg),
        "volume" | "volume_low" | "volume_mute" => {
            rect(-5.0, -2.0, 3.0, 4.0);
            rect(-6.0, -1.0, 1.0, 2.0);
            faint(-6.0, -2.0, 1.0, 1.0, 0.3);
            faint(-6.0, 1.0, 1.0, 1.0, 0.3);
            rect(-2.0, -3.0, 1.0, 6.0);
            rect(-1.0, -4.0, 2.0, 8.0);
            faint(0.0, -5.0, 1.0, 1.0, 0.45);
            faint(0.0, 4.0, 1.0, 1.0, 0.45);
            if name == "volume_mute" {
                for i in 0..5 {
                    rect(2.0 + i as f32, -2.0 + i as f32, 1.0, 1.0);
                    rect(6.0 - i as f32, -2.0 + i as f32, 1.0, 1.0);
                }
            } else {
                rect(2.0, -2.0, 1.0, 4.0);
                if name == "volume" {
                    rect(4.0, -3.0, 1.0, 6.0);
                }
            }
        }
        // Beamed eighth-note pair: the audio *track* selector, kept visually
        // distinct from the speaker glyphs that mean volume.
        "note" => {
            rect(-3.0, -5.0, 1.6, 6.4);
            rect(2.8, -5.0, 1.6, 7.4);
            rect(-3.0, -5.0, 7.4, 2.0);
            p.rect_filled(
                Rect::from_min_size(pos2(cx - 5.6, cy + 0.6), vec2(5.2, 3.4)),
                1.6,
                color,
            );
            p.rect_filled(
                Rect::from_min_size(pos2(cx + 0.4, cy + 1.4), vec2(5.2, 3.4)),
                1.6,
                color,
            );
        }
        "pin" => {
            rect(-3.0, -5.0, 7.0, 1.0);
            rect(-2.0, -4.0, 5.0, 4.0);
            rect(-3.0, 0.0, 7.0, 1.0);
            rect(-4.0, 1.0, 9.0, 1.0);
            rect(0.0, 2.0, 1.0, 3.0);
            rect(-3.0, 5.0, 7.0, 1.0);
        }
        "minimize" => rect(-4.0, 3.0, 9.0, 2.0),
        "maximize" => {
            rect(-4.0, -4.0, 9.0, 3.0);
            rect(-4.0, -1.0, 1.0, 5.0);
            rect(4.0, -1.0, 1.0, 5.0);
            rect(-4.0, 4.0, 9.0, 1.0);
        }
        "restore" => {
            rect(-4.0, -1.0, 7.0, 2.0);
            rect(-4.0, 1.0, 1.0, 3.0);
            rect(2.0, 1.0, 1.0, 3.0);
            rect(-4.0, 4.0, 7.0, 1.0);
            rect(-2.0, -4.0, 7.0, 1.0);
            rect(-2.0, -3.0, 1.0, 1.0);
            rect(4.0, -3.0, 1.0, 5.0);
            rect(3.0, 1.0, 1.0, 1.0);
        }
        "fullscreen" => corners(&[
            (-4.0, -4.0),
            (-3.0, -4.0),
            (-2.0, -4.0),
            (-4.0, -3.0),
            (-3.0, -3.0),
            (-4.0, -2.0),
            (-2.0, -2.0),
            (-1.0, -1.0),
        ]),
        "fullscreen_exit" => corners(&[
            (-1.0, -1.0),
            (-2.0, -1.0),
            (-3.0, -1.0),
            (-1.0, -2.0),
            (-2.0, -2.0),
            (-1.0, -3.0),
            (-3.0, -3.0),
            (-4.0, -4.0),
        ]),
        "close" => {
            for i in 0..4 {
                let i = i as f32;
                for y in [-4.0 + i, 3.0 - i] {
                    rect(-4.0 + i, y, 2.0, 1.0);
                    rect(3.0 - i, y, 2.0, 1.0);
                }
            }
        }
        "info" => {
            p.circle_stroke(pos2(cx, cy), 6.0, Stroke::new(1.2, color));
            p.circle_filled(pos2(cx, cy - 2.5), 0.9, color);
            rect(-0.8, 0.0, 1.6, 4.0);
        }
        "chevron" => {
            for i in 0..4 {
                rect(-3.0 + i as f32, i as f32, 1.0, 1.0);
                rect(3.0 - i as f32, i as f32, 1.0, 1.0);
            }
        }
        "repeat" => {
            let pts: Vec<Pos2> = (0..=32)
                .map(|i| {
                    let angle = (40.0 + 280.0 * i as f32 / 32.0).to_radians();
                    pos2(cx + 5.0 * angle.cos(), cy + 4.0 * angle.sin())
                })
                .collect();
            p.add(Shape::line(pts, Stroke::new(1.4, color)));
            poly(&[(2.0, -6.5), (6.5, -4.0), (2.0, -1.5)]);
        }
        "shuffle" => {
            for pts in [
                [(-6.0f32, -3.0), (-2.0, -3.0), (2.0, 3.0), (6.0, 3.0)],
                [(-6.0, 3.0), (-2.0, 3.0), (2.0, -3.0), (6.0, -3.0)],
            ] {
                let points: Vec<Pos2> = pts.iter().map(|&(x, y)| pos2(cx + x, cy + y)).collect();
                p.add(Shape::line(points, Stroke::new(1.3, color)));
            }
        }
        "seek_back" | "seek_fwd" => {
            let mirror = name == "seek_fwd";
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let angle = (140.0 + 260.0 * i as f32 / 24.0).to_radians();
                    let x = 6.4 * angle.cos();
                    let x = if mirror { -x } else { x };
                    pos2(cx + x, cy - 6.4 * angle.sin())
                })
                .collect();
            p.add(Shape::line(pts, Stroke::new(1.4, color)));
            // Arrowhead at the arc start, pointing along the tangent.
            let start = 140.0f32.to_radians();
            let hx = if mirror {
                -6.4 * start.cos()
            } else {
                6.4 * start.cos()
            };
            let hy = -6.4 * start.sin();
            let a = 0.55f32;
            if mirror {
                poly(&[
                    (hx - 3.6, hy - a),
                    (hx + 1.2, hy - 2.8),
                    (hx + 1.2, hy + 2.8),
                ]);
            } else {
                poly(&[
                    (hx + 3.6, hy - a),
                    (hx - 1.2, hy - 2.8),
                    (hx - 1.2, hy + 2.8),
                ]);
            }
        }
        "subs" => {
            p.rect_stroke(
                Rect::from_min_size(pos2(cx - 7.0, cy - 5.0), vec2(14.0, 10.0)),
                2.0,
                Stroke::new(1.2, color),
                egui::StrokeKind::Inside,
            );
            rect(-4.5, 0.6, 5.0, 1.4);
            rect(1.5, 0.6, 3.0, 1.4);
            rect(-4.5, -2.6, 3.0, 1.4);
            rect(0.5, -2.6, 4.0, 1.4);
        }
        "search" => {
            p.circle_stroke(pos2(cx - 1.5, cy - 1.5), 4.0, Stroke::new(1.5, color));
            p.line_segment(
                [pos2(cx + 1.5, cy + 1.5), pos2(cx + 5.5, cy + 5.5)],
                Stroke::new(1.5, color),
            );
        }
        _ => {}
    }
}

/// Eight-tooth gear with an axle hole punched out in the button's background colour.
fn gear(p: &Painter, cx: f32, cy: f32, color: Color32, bg: Color32) {
    let teeth = 8;
    let half = PI / teeth as f32;
    let mut points: Vec<Pos2> = Vec::with_capacity(teeth * 4);
    for i in 0..teeth {
        let base = 2.0 * PI * i as f32 / teeth as f32;
        for (radius, delta) in [
            (5.0f32, -half * 0.62),
            (7.0, -half * 0.36),
            (7.0, half * 0.36),
            (5.0, half * 0.62),
        ] {
            let angle = base + delta;
            points.push(pos2(cx + radius * angle.cos(), cy + radius * angle.sin()));
        }
    }
    // Radially monotone: a fan from the centre is always inside the outline.
    fan(p, pos2(cx, cy), &points, color);
    p.circle_filled(pos2(cx, cy), 2.1, bg);
}

fn fan(p: &Painter, center: Pos2, pts: &[Pos2], color: Color32) {
    if pts.len() < 3 {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for pt in pts {
        mesh.colored_vertex(*pt, color);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + ((i + 1) % n));
    }
    p.add(Shape::Mesh(Arc::new(mesh)));
}

/// Two-stop vertical gradient fill used by the floating chrome scrims.
pub fn v_gradient(p: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::Mesh(Arc::new(mesh)));
}

// -- text helpers --------------------------------------------------------------------------------

/// Width of `text` when drawn with `font_id`.
pub fn text_width(p: &Painter, text: &str, font_id: &FontId) -> f32 {
    p.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_string(), font_id.clone(), TEXT))
        .size()
        .x
}

/// Elide text from the right when it exceeds the available width.
pub fn elide(p: &Painter, text: &str, font_id: &FontId, max_width: f32) -> String {
    if text_width(p, text, font_id) <= max_width {
        return text.to_string();
    }
    let ellipsis = "...";
    let ellipsis_width = text_width(p, ellipsis, font_id);
    let mut lo = 0usize;
    let mut hi = text.chars().count();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let prefix: String = text.chars().take(mid).collect();
        if text_width(p, &prefix, font_id) + ellipsis_width <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    // Scene-release names are dot-separated tokens; a cut like "1080p.6..."
    // reads broken. Snap back to a token boundary when only a short dangling
    // fragment would remain.
    let chars: Vec<char> = text.chars().collect();
    if let Some(sep) = (0..lo)
        .rev()
        .find(|&i| matches!(chars[i], '.' | ' ' | '_' | '-'))
        && lo - sep <= (lo / 4).max(4)
        && sep > 0
    {
        lo = sep;
    }
    format!("{}{}", text.chars().take(lo).collect::<String>(), ellipsis)
}

/// Middle ellipsis (used for window titles).
pub fn elide_middle(p: &Painter, text: &str, font_id: &FontId, max_width: f32) -> String {
    if text_width(p, text, font_id) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let head = chars.len() / 2;
    let tail_start = head + 1;
    for gap in 1..chars.len() {
        let prefix: String = chars[..head.saturating_sub(gap / 2)].iter().collect();
        let suffix: String = chars[(tail_start + gap / 2).min(chars.len())..]
            .iter()
            .collect();
        let candidate = format!("{prefix}...{suffix}");
        if text_width(p, &candidate, font_id) <= max_width {
            return candidate;
        }
    }
    elide(p, text, font_id, max_width)
}

// -- egui style ----------------------------------------------------------------------------------

/// Restyle the whole egui context to the active MacPlayer palette. Both egui themes are
/// populated and the preference is set, so `ctx.set_theme` stays consistent.
pub fn apply(ctx: &egui::Context) {
    let theme = current_theme();
    ctx.set_style_of(egui::Theme::Dark, style(Theme::Dark));
    ctx.set_style_of(egui::Theme::Light, style(Theme::Light));
    ctx.set_theme(match theme {
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::Light => egui::ThemePreference::Light,
    });
}

pub fn style(theme: Theme) -> egui::Style {
    let index = {
        let len = ACCENT_SETS.len() as u8;
        (ACTIVE_ACCENT.load(Ordering::Acquire) % len.max(1)) as usize
    };
    let (_, accent, focus) = ACCENT_SETS[index];
    let colors = match theme {
        Theme::Dark => ThemeColors {
            accent,
            logo: accent,
            focus,
            ..ThemeColors::classic()
        },
        Theme::Light => ThemeColors {
            accent,
            logo: accent,
            focus,
            ..ThemeColors::light()
        },
    };
    let mut style = egui::Style::default();
    let v = &mut style.visuals;
    v.dark_mode = theme == Theme::Dark;
    v.override_text_color = Some(colors.text_e4);
    v.panel_fill = colors.dialog_bg;
    v.window_fill = colors.dialog_bg;
    v.window_stroke = Stroke::new(1.0, colors.dialog_line);
    v.window_corner_radius = 10.into();
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 2,
        color: colors.shadow,
    };
    v.extreme_bg_color = colors.base;
    v.faint_bg_color = colors.tab_bg;
    // Accent-tinted selection: egui paints slider/progress fills from this too, so
    // every slider agrees with the accent instead of the neutral row-select gray.
    v.selection.bg_fill = accent.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, colors.text_e4);
    v.widgets.noninteractive.bg_fill = colors.dialog_bg;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, colors.text_e4);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.dialog_line);
    v.widgets.inactive.bg_fill = colors.button_bg;
    v.widgets.inactive.weak_bg_fill = colors.button_bg;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, colors.text_e4);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, colors.dialog_line);
    v.widgets.hovered.bg_fill = colors.button_hover;
    v.widgets.hovered.weak_bg_fill = colors.button_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, colors.dialog_line);
    v.widgets.active.bg_fill = colors.button_press;
    v.widgets.active.weak_bg_fill = colors.button_press;
    style.spacing.item_spacing = vec2(7.0, 7.0);
    style.spacing.button_padding = vec2(11.0, 5.0);
    style.spacing.interact_size = vec2(30.0, 24.0);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 1,
        color: colors.shadow,
    };
    style
}
