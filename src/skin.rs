//! Skin: colours, metrics, fonts, shared tables and hand-drawn vector glyphs (no bitmap assets).
//!
//! MacPlayer's custom dark skin. Glyphs are drawn from primitives on a stable pixel grid;
//! `(cx, cy)` is the centre of the button and the coordinate system is y-down.

use std::f32::consts::PI;
use std::sync::Arc;

use egui::{Color32, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::value::Value;

// -- colours -------------------------------------------------------------------------------------

pub const BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
pub const BG_PRESS: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
pub const VIDEO_BG: Color32 = Color32::from_rgb(0x10, 0x10, 0x10);
pub const LINE: Color32 = Color32::BLACK;
pub const ACCENT: Color32 = Color32::from_rgb(0xfa, 0xe1, 0x00);
pub const KNOB: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);
pub const ICON: Color32 = KNOB;
pub const ICON_HOVER: Color32 = Color32::WHITE;
pub const ICON_DIM: Color32 = Color32::from_rgb(0x77, 0x77, 0x77);
pub const ICON_OFF: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
pub const TEXT: Color32 = Color32::from_rgb(0xc5, 0xc5, 0xc5);
pub const TEXT_BRIGHT: Color32 = KNOB;
pub const TEXT_DIM: Color32 = ICON_DIM;
pub const LOGO: Color32 = ACCENT;
pub const PANEL: Color32 = Color32::from_rgb(0x19, 0x19, 0x19);
pub const SELECT: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
pub const DANGER: Color32 = Color32::from_rgb(0xe0, 0x4a, 0x3f);
pub const BUFFER: Color32 = ICON_OFF;

// Dialog chrome.
pub const DIALOG_BG: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);
pub const DIALOG_LINE: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
pub const DIALOG_PANE: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
pub const BASE: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const BUTTON_BG: Color32 = SELECT;
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x47, 0x47, 0x47);
pub const BUTTON_PRESS: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const MENU_BG: Color32 = DIALOG_BG;
pub const MENU_SEL: Color32 = Color32::from_rgb(0x45, 0x45, 0x45);
pub const TAB_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x23);
pub const TAB_SEL: Color32 = DIALOG_BG;
pub const TEXT_E4: Color32 = Color32::from_rgb(0xe4, 0xe4, 0xe4);
pub const TEXT_A8: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xa8);
pub const TEXT_9A: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xff, 0x6a, 0x5e);
pub const FOCUS: Color32 = Color32::from_rgb(0xff, 0xea, 0x55);
pub const SHADOW: Color32 = Color32::from_rgba_premultiplied(0x00, 0x00, 0x00, 0xb8);

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
        }
    }
}

pub fn active_colors() -> ThemeColors {
    ThemeColors::classic()
}

// -- metrics (logical px) ------------------------------------------------------------------------

pub const FRAME: f32 = 3.0;
pub const TITLE_H: f32 = 30.0;
pub const SEEK_H: f32 = 20.0;
pub const CTRL_H: f32 = 40.0;
pub const BTN_W: f32 = 40.0;
pub const VOLUME_W: f32 = 102.0;
pub const MENU_BTN_W: f32 = 88.0;
pub const TITLE_BTN_W: f32 = 24.0;
pub const MIN_PLAYLIST_W: f32 = 160.0;
pub const MIN_MAIN_W: f32 = 168.0;

// -- fonts ---------------------------------------------------------------------------------------

pub fn font(px: f32, bold: bool) -> FontId {
    let _ = bold; // egui has no synthetic bold for the default fonts; mpv draws the subs itself.
    FontId::new(px, FontFamily::Proportional)
}

pub fn mono(px: f32) -> FontId {
    FontId::new(px, FontFamily::Monospace)
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

pub const VIDEO_CODECS: &[(&str, &str)] = &[
    ("h264", "AVC1"),
    ("hevc", "HEVC"),
    ("mpeg4", "MP4V"),
    ("mpeg2video", "MPEG2"),
    ("vp9", "VP9"),
    ("vp8", "VP8"),
    ("av1", "AV1"),
    ("wmv3", "WMV3"),
    ("vc1", "VC1"),
    ("msmpeg4v3", "DIV3"),
];

pub const AUDIO_CODECS: &[(&str, &str)] = &[
    ("eac3", "E-AC3"),
    ("truehd", "TrueHD"),
    ("vorbis", "Vorbis"),
    ("opus", "Opus"),
];

pub fn codec_label(codecs: &[(&str, &str)], codec: &str) -> String {
    codecs
        .iter()
        .find(|(key, _)| *key == codec)
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| codec.to_uppercase())
}

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
    if track.get("external").as_bool() {
        if let Some(file) = track.get("external-filename").as_str() {
            title = crate::util::title_of(file);
        }
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
            "Track".to_string()
        } else {
            parts.join(" - ")
        }
    );
    if let Some(codec) = track.get("codec").as_str() {
        if !codec.is_empty() {
            label.push_str(&format!("  [{}]", codec.to_uppercase()));
        }
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

/// Idle-screen watermark: a dark disc with a play triangle, and the application name beside it.
pub fn draw_logo(p: &Painter, cx: f32, cy: f32, scale: f32, text: &str) {
    let font_id = font(46.0 * scale * 0.935, false);
    let width = p
        .fonts_mut(|fonts| fonts.layout_no_wrap(text.to_string(), font_id.clone(), TEXT_DIM))
        .size()
        .x;
    let radius = 32.0 * scale;
    let gap = 15.0 * scale;
    let left = (cx - (2.0 * radius + gap + width) / 2.0).round();
    let dcx = left + radius;
    let dcy = cy + scale;
    p.circle_filled(pos2(dcx, dcy), radius, LOGO);
    p.add(Shape::convex_polygon(
        vec![
            pos2(dcx - 9.2 * scale, dcy - 14.7 * scale),
            pos2(dcx - 9.2 * scale, dcy + 14.7 * scale),
            pos2(dcx + 15.3 * scale, dcy),
        ],
        VIDEO_BG,
        Stroke::NONE,
    ));
    p.text(
        pos2(left + 2.0 * radius + gap, dcy + 14.7 * scale),
        egui::Align2::LEFT_BOTTOM,
        text,
        font_id,
        LOGO,
    );
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
        let mid = (lo + hi + 1) / 2;
        let prefix: String = text.chars().take(mid).collect();
        if text_width(p, &prefix, font_id) + ellipsis_width <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
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

/// Restyle the whole egui context to the MacPlayer dark palette.
pub fn apply(ctx: &egui::Context) {
    ctx.set_style_of(egui::Theme::Dark, style());
}

pub fn style() -> egui::Style {
    let colors = ThemeColors::classic();
    let mut style = egui::Style::default();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(colors.text_e4);
    v.panel_fill = colors.dialog_bg;
    v.window_fill = colors.dialog_bg;
    v.window_stroke = Stroke::new(1.0, colors.dialog_line);
    v.window_corner_radius = 8.into();
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 18,
        spread: 2,
        color: colors.shadow,
    };
    v.extreme_bg_color = colors.base;
    v.faint_bg_color = colors.tab_bg;
    v.selection.bg_fill = colors.select;
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
    style.spacing.item_spacing = vec2(6.0, 6.0);
    style.spacing.button_padding = vec2(10.0, 4.0);
    style.spacing.interact_size = vec2(28.0, 22.0);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 1,
        color: colors.shadow,
    };
    style
}
