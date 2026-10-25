//! Skinned chrome widgets drawn directly with egui shapes on a stable pixel grid.
//!
//! Custom MacPlayer widgets: each widget paints into a fixed rect and reports pointer
//! interaction without relying on stock media-player controls.

use std::time::Instant;

use egui::{Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::i18n;
use crate::skin;
use crate::util::fmt_time;

/// A response returned by the chrome widgets.
#[derive(Default, Clone, Copy)]
pub struct Click {
    pub primary: bool,
}

/// Icon button: a fixed rect with a hand-drawn glyph, hover/press background and tooltip.
/// The rect is supplied per call so the same button can be laid out at any size.
pub struct IconBtn<'a> {
    pub icon: &'a str,
    pub tip: Option<&'a str>,
    pub color: Color32,
    pub flat: bool,
    pub checked: bool,
    pub enabled: bool,
    /// Extra alpha multiplier for fading floating chrome in and out.
    pub alpha: f32,
}

impl<'a> IconBtn<'a> {
    pub fn new(icon: &'a str) -> Self {
        let colors = skin::active_colors();
        Self {
            icon,
            tip: None,
            color: colors.icon,
            flat: false,
            checked: false,
            enabled: true,
            alpha: 1.0,
        }
    }

    /// Fade the whole button by `alpha` (0..1), for animated chrome.
    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    pub fn flat(mut self, flat: bool) -> Self {
        self.flat = flat;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn ui(self, ui: &mut Ui, id_salt: Id, rect: Rect) -> Click {
        let colors = skin::active_colors();
        // Faded chrome: one multiplier applied to every painted color.
        let fade = |color: Color32| color.gamma_multiply(self.alpha);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, id_salt, sense);
        let response = if let Some(tip) = self.tip {
            response.on_hover_text(tip)
        } else {
            response
        };
        let hovered = response.hovered();
        let down = response.is_pointer_button_down_on();

        let painter = ui.painter_at(rect);
        let corner_radius = if self.flat { 7.0 } else { 6.0 };
        let mut bg = if self.flat {
            Color32::TRANSPARENT
        } else if self.enabled {
            colors.bg
        } else {
            // A different fill is the only disabled cue beyond the dimmed icon.
            colors.base
        };
        if self.flat && self.checked {
            bg = Color32::from_rgba_unmultiplied(
                colors.accent.r(),
                colors.accent.g(),
                colors.accent.b(),
                24,
            );
        } else if !self.flat && self.enabled {
            if down {
                bg = colors.bg_press;
            } else if hovered {
                bg = colors.bg_hover;
            }
        }
        painter.rect_filled(rect, corner_radius, fade(bg));
        painter.rect_stroke(
            rect,
            corner_radius,
            Stroke::new(
                if response.has_focus() {
                    2.0
                } else if self.flat {
                    0.0
                } else {
                    1.0
                },
                fade(if response.has_focus() {
                    colors.accent.gamma_multiply(0.85)
                } else if self.checked {
                    colors.accent
                } else {
                    colors.line
                }),
            ),
            egui::StrokeKind::Outside,
        );
        let color = if !self.enabled {
            colors.icon_off
        } else if self.checked {
            colors.accent
        } else if hovered {
            colors.icon_hover
        } else {
            self.color
        };
        let center = rect.center();
        skin::draw_icon(
            &painter,
            self.icon,
            center.x.round(),
            center.y.round(),
            fade(color),
            fade(bg),
        );

        Click {
            primary: response.clicked(),
        }
    }
}

/// Plain text button used by the playlist footer (Add / Del / Sort).
pub fn text_button(ui: &mut Ui, id_salt: Id, rect: Rect, text: &str) -> bool {
    let colors = skin::active_colors();
    let response = ui.interact(rect, id_salt, Sense::click());
    let painter = ui.painter_at(rect);
    let corner_radius = ui.style().visuals.widgets.inactive.corner_radius;
    painter.rect_filled(
        rect,
        corner_radius,
        if response.hovered() {
            colors.bg_hover
        } else {
            colors.bg
        },
    );
    painter.rect_stroke(
        rect,
        corner_radius,
        Stroke::new(1.0, colors.line),
        egui::StrokeKind::Outside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        skin::font(12.0, false),
        if response.hovered() {
            colors.text_bright
        } else {
            colors.text
        },
    );
    response.clicked()
}

// -- seek bar ------------------------------------------------------------------------------------

pub struct SeekUi {
    pub duration: f64,
    pub position: f64,
    pub cache_end: Option<f64>,
    pub chapters: Vec<f64>,
    pub ab: (Option<f64>, Option<f64>),
    pub dragging: bool,
    pub drag_time: f64,
    pub last_emit: Instant,
    pub hover: bool,
    /// Leftover raw wheel distance below one step.
    pub scroll_acc: f32,
}

impl Default for SeekUi {
    fn default() -> Self {
        Self {
            duration: 0.0,
            position: 0.0,
            cache_end: None,
            chapters: Vec::new(),
            ab: (None, None),
            dragging: false,
            drag_time: 0.0,
            last_emit: Instant::now(),
            hover: false,
            scroll_acc: 0.0,
        }
    }
}

pub struct SeekBarOut {
    pub seek: Option<(f64, bool)>, // seconds, exact
    pub hover: Option<f64>,        // seconds, or -1.0 when the pointer left the bar
    pub wheel: i32,
}

impl SeekUi {
    pub fn reset(&mut self) {
        self.duration = 0.0;
        self.position = 0.0;
        self.cache_end = None;
        self.chapters.clear();
        self.ab = (None, None);
        self.dragging = false;
        self.scroll_acc = 0.0;
    }

    fn span(&self, width: f32) -> (f32, f32) {
        let inset = (width.max(0.0) / 2.0).min(5.0);
        (inset, (width - inset * 2.0).max(0.0))
    }

    fn time_at(&self, x: f32, width: f32) -> f64 {
        let (left, span) = self.span(width);
        if span <= 0.0 || !self.duration.is_finite() || self.duration <= 0.0 {
            return 0.0;
        }
        (((x - left) / span).clamp(0.0, 1.0) as f64) * self.duration
    }

    fn x_of(&self, seconds: f64, width: f32) -> f32 {
        let (left, span) = self.span(width);
        if span <= 0.0 || !self.duration.is_finite() || self.duration <= 0.0 {
            return left;
        }
        left + ((seconds / self.duration).clamp(0.0, 1.0) as f32) * span
    }
}

/// Accumulate raw `MouseWheel` events into discrete steps for stepping
/// controls (volume, seek).
///
/// `smooth_scroll_delta` must not be used here: it is a low-pass filter that
/// stays nonzero for dozens of frames after a single wheel notch, and on macOS
/// trackpads scroll events keep arriving for the whole momentum phase after the
/// fingers lift — so reading its sign per frame would fire a full step every
/// frame for seconds. Raw events quantized to notches give one step per wheel
/// notch and distance-proportional steps on a trackpad.
fn wheel_steps(ui: &Ui, acc: &mut f32) -> i32 {
    /// Trackpad points that add up to one step (one wheel notch).
    const POINTS_PER_STEP: f32 = 10.0;
    ui.input(|i| {
        for event in &i.events {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
            {
                // Zoom/alt gestures belong to egui, not to stepping.
                if modifiers.ctrl || modifiers.command || modifiers.alt {
                    continue;
                }
                match unit {
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                        *acc += delta.y;
                    }
                    egui::MouseWheelUnit::Point => *acc += delta.y / POINTS_PER_STEP,
                }
            }
        }
    });
    let mut steps = 0;
    while *acc >= 1.0 {
        steps += 1;
        *acc -= 1.0;
    }
    while *acc <= -1.0 {
        steps -= 1;
        *acc += 1.0;
    }
    steps
}

pub fn seek_bar(
    ui: &mut Ui,
    rect: Rect,
    id_salt: Id,
    state: &mut SeekUi,
    now: Instant,
    alpha: f32,
) -> SeekBarOut {
    let colors = skin::active_colors();
    // Faded chrome: one multiplier applied to every painted color.
    let fade = |color: Color32| color.gamma_multiply(alpha);
    let mut out = SeekBarOut {
        seek: None,
        hover: None,
        wheel: 0,
    };
    let painter = ui.painter_at(rect);
    // No opaque base fill: the bar floats over the video scrim (quiet-player chrome).

    let response = ui.interact(rect, id_salt, Sense::click_and_drag());
    let was_hovered = state.hover;
    state.hover = response.hovered();
    let y = (rect.height() / 2.0 - 2.0).round();
    let (left, span) = state.span(rect.width());
    let track_rect = Rect::from_min_size(pos2(rect.left() + left, rect.top() + y), vec2(span, 5.0));
    if response.hovered() || response.has_focus() {
        painter.rect_filled(
            track_rect.expand(3.0),
            3.0,
            fade(Color32::from_rgba_unmultiplied(
                colors.focus.r(),
                colors.focus.g(),
                colors.focus.b(),
                34,
            )),
        );
    }
    painter.rect_filled(track_rect, 3.0, fade(colors.line));

    if response.hovered() {
        out.wheel = wheel_steps(ui, &mut state.scroll_acc);
    }
    if response.hovered() {
        // No time bubble when nothing is playing: the mapping is meaningless
        // without a duration and would hover "0:00:00" over the empty bar.
        if state.duration > 0.0
            && let Some(pos) = response.hover_pos()
        {
            out.hover = Some(state.time_at(pos.x - rect.left(), rect.width()));
        }
    } else if was_hovered {
        out.hover = Some(-1.0);
    }

    if state.duration > 0.0 {
        let pressed = response.is_pointer_button_down_on();
        let hover_x = response.hover_pos().map(|p| p.x - rect.left());
        if !state.dragging && pressed && ui.input(|i| i.pointer.primary_pressed()) {
            state.dragging = true;
            state.drag_time = hover_x
                .map(|x| state.time_at(x, rect.width()))
                .unwrap_or(0.0);
            state.last_emit = now;
            out.seek = Some((state.drag_time, true));
        }
        if state.dragging {
            if let Some(x) = response.interact_pointer_pos().map(|p| p.x - rect.left()) {
                state.drag_time = state.time_at(x, rect.width());
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                state.dragging = false;
                state.position = state.drag_time;
                out.seek = Some((state.drag_time, true));
            } else {
                let elapsed = now.duration_since(state.last_emit).as_secs_f64();
                if elapsed > 0.05 {
                    state.last_emit = now;
                    out.seek = Some((state.drag_time, false));
                }
            }
        }

        let current = if state.dragging {
            state.drag_time
        } else {
            state.position
        };
        let x = state.x_of(current, rect.width());
        if let Some(cache_end) = state.cache_end.filter(|end| *end > current) {
            let x_end = state.x_of(cache_end, rect.width());
            painter.rect_filled(
                Rect::from_min_size(pos2(rect.left() + x, rect.top() + y), vec2(x_end - x, 5.0)),
                3.0,
                fade(colors.buffer),
            );
        }
        let (a, b) = state.ab;
        if let (Some(a), Some(b)) = (a, b)
            && b > a
        {
            let xa = state.x_of(a, rect.width());
            let xb = state.x_of(b, rect.width());
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(rect.left() + xa, rect.top() + y - 2.0),
                    vec2(xb - xa, 3.0),
                ),
                0.0,
                fade(colors.accent),
            );
        }
        painter.rect_filled(
            Rect::from_min_size(
                pos2(rect.left() + left, rect.top() + y),
                vec2((x - left).max(0.0), 5.0),
            ),
            3.0,
            fade(colors.accent),
        );
        for chapter in &state.chapters {
            if *chapter > 0.0 && *chapter < state.duration {
                let cx = state.x_of(*chapter, rect.width()).round();
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(rect.left() + cx, rect.top() + y - 2.0),
                        vec2(2.0, 9.0),
                    ),
                    0.0,
                    fade(colors.text_dim),
                );
            }
        }
        for point in [a, b].into_iter().flatten() {
            let px = state.x_of(point, rect.width()).round();
            painter.rect_filled(
                Rect::from_min_size(pos2(rect.left() + px, rect.top() + y - 5.0), vec2(2.0, 6.0)),
                0.0,
                fade(colors.accent),
            );
        }
        let radius = if state.hover || state.dragging {
            5.5
        } else {
            4.5
        };
        painter.circle_filled(
            pos2(
                (rect.left() + x).max(rect.left() + left),
                rect.top() + y + 1.0,
            ),
            radius,
            fade(colors.knob),
        );
    }

    out
}

// -- volume bar ----------------------------------------------------------------------------------

pub struct VolumeUi {
    pub volume: i64,
    /// Upper end of the slider (the volume-boost ceiling), percent.
    pub max: i64,
    pub muted: bool,
    pub dragging: bool,
    pub hover_icon: bool,
    /// Leftover raw wheel distance below one step.
    pub scroll_acc: f32,
}

impl Default for VolumeUi {
    fn default() -> Self {
        Self {
            volume: 100,
            max: 100,
            muted: false,
            dragging: false,
            hover_icon: false,
            scroll_acc: 0.0,
        }
    }
}

pub struct VolumeOut {
    pub volume: Option<i64>,
    pub mute: bool,
    pub wheel: i32,
}

pub fn volume_bar(
    ui: &mut Ui,
    rect: Rect,
    id_salt: Id,
    state: &mut VolumeUi,
    alpha: f32,
) -> VolumeOut {
    let colors = skin::active_colors();
    // Faded chrome: one multiplier applied to every painted color.
    let fade = |color: Color32| color.gamma_multiply(alpha);
    let mut out = VolumeOut {
        volume: None,
        mute: false,
        wheel: 0,
    };
    let painter = ui.painter_at(rect);
    // No opaque base fill: the bar floats over the video scrim (quiet-player chrome).
    let response = ui.interact(rect, id_salt, Sense::click_and_drag());

    let width = rect.width().max(0.0);
    let line_x0 = (width * 0.22).min(22.0);
    let line_x1 = (width - 5.0).max(line_x0).min(91.0);
    let knob_x0 = (line_x0 + 4.0).min(line_x1);
    let knob_x1 = (line_x1 - 5.0).max(knob_x0).min(line_x1);

    let local_x = response.hover_pos().map(|p| p.x - rect.left());
    let hover_icon = local_x.is_some_and(|x| x < line_x0);
    if hover_icon != state.hover_icon {
        state.hover_icon = hover_icon;
    }
    let response = response.on_hover_text(format!("{}: {}%", i18n::tr("Volume"), state.volume));

    let y = (rect.height() / 2.0 - 2.0).round();
    let icon = if state.muted || state.volume == 0 {
        "volume_mute"
    } else if state.volume < 50 {
        "volume_low"
    } else {
        "volume"
    };
    let max = state.max.max(1) as f32;
    skin::draw_icon(
        &painter,
        icon,
        rect.left() + (width * 0.11).min(11.0),
        rect.center().y,
        fade(if state.hover_icon {
            colors.icon_hover
        } else {
            colors.icon_dim
        }),
        fade(colors.panel),
    );
    let knob_x = knob_x0 + (knob_x1 - knob_x0) * state.volume as f32 / max;
    let track_rect = Rect::from_min_size(
        pos2(rect.left() + line_x0, rect.top() + y),
        vec2((line_x1 - line_x0).max(0.0), 5.0),
    );
    if response.hovered() || response.has_focus() {
        painter.rect_filled(
            track_rect.expand(3.0),
            3.0,
            fade(Color32::from_rgba_unmultiplied(
                colors.focus.r(),
                colors.focus.g(),
                colors.focus.b(),
                30,
            )),
        );
    }
    painter.rect_filled(track_rect, 3.0, fade(colors.line));
    let fill = if state.muted {
        colors.text_dim
    } else {
        colors.accent
    };
    painter.rect_filled(
        Rect::from_min_size(
            pos2(rect.left() + line_x0, rect.top() + y),
            vec2((knob_x - line_x0).max(0.0), 5.0),
        ),
        3.0,
        fade(fill),
    );
    painter.circle_filled(
        pos2(rect.left() + knob_x, rect.top() + y + 1.0),
        if state.dragging || response.hovered() {
            5.0
        } else {
            4.5
        },
        fade(colors.knob),
    );

    if response.hovered() {
        out.wheel = wheel_steps(ui, &mut state.scroll_acc);
    }
    if !state.dragging && ui.input(|i| i.pointer.primary_pressed()) && response.hovered() {
        let pos = response.interact_pointer_pos().unwrap_or(rect.center());
        if pos.x - rect.left() < line_x0 {
            out.mute = true;
        } else {
            state.dragging = true;
            out.volume = Some(volume_at(pos.x - rect.left(), knob_x0, knob_x1, state.max));
        }
    }
    if state.dragging {
        if let Some(pos) = response.interact_pointer_pos() {
            if ui.input(|i| i.pointer.primary_down()) {
                out.volume = Some(volume_at(pos.x - rect.left(), knob_x0, knob_x1, state.max));
            } else {
                state.dragging = false;
            }
        } else {
            state.dragging = false;
        }
    }
    out
}

fn volume_at(x: f32, knob_x0: f32, knob_x1: f32, max: i64) -> i64 {
    if knob_x1 <= knob_x0 {
        return if x >= knob_x1 { max } else { 0 };
    }
    let frac = (x - knob_x0) / (knob_x1 - knob_x0);
    ((frac.clamp(0.0, 1.0) * max as f32).round()) as i64
}

// -- on-screen display ----------------------------------------------------------------------------

/// Semantic color of an OSD message; resolved against the active theme at paint time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OsdTone {
    #[default]
    Normal,
    Danger,
}

#[derive(Default)]
pub struct Osd {
    pub message: String,
    pub message_tone: OsdTone,
    pub message_until: Option<Instant>,
    pub info: String,
    pub center: String,
}

impl Osd {
    pub fn show_message(&mut self, text: impl Into<String>, ms: u64, tone: OsdTone) {
        self.message = text.into();
        self.message_tone = tone;
        self.message_until = Some(Instant::now() + std::time::Duration::from_millis(ms));
    }

    pub fn clear_message(&mut self) {
        self.message.clear();
        self.message_until = None;
    }

    pub fn tick(&mut self) {
        if let Some(until) = self.message_until
            && Instant::now() >= until
        {
            self.clear_message();
        }
    }

    pub fn paint(&self, ui: &mut Ui, rect: Rect) {
        if self.message.is_empty() && self.info.is_empty() && self.center.is_empty() {
            return;
        }
        let colors = skin::active_colors();
        let painter = ui.painter_at(rect);
        let size = (15.0f32).max((30.0f32).min(rect.height() / 22.0));
        // The message box fades out over its last 300 ms instead of vanishing.
        let message_alpha = self
            .message_until
            .map(|until| {
                let remaining = until
                    .saturating_duration_since(Instant::now())
                    .as_secs_f32();
                (remaining / 0.3).clamp(0.0, 1.0)
            })
            .unwrap_or(1.0);
        let mut y = rect.top() + 8.0;
        if !self.message.is_empty() {
            let fade = |color: Color32| color.gamma_multiply(message_alpha);
            let font_id = skin::font(size, true);
            let message_color = match self.message_tone {
                OsdTone::Normal => colors.text_e4,
                OsdTone::Danger => colors.danger,
            };
            let line = size * 1.35;
            // Long error messages must not overflow the window; elide to the
            // available width instead of growing the box past the video.
            let max_line_width = (rect.width() - 40.0).max(60.0);
            let lines: Vec<String> = self
                .message
                .split('\n')
                .map(|l| skin::elide(&painter, l, &font_id, max_line_width))
                .collect();
            let w = lines
                .iter()
                .map(|l| skin::text_width(&painter, l, &font_id))
                .fold(0.0f32, f32::max)
                + 20.0;
            let h = line * lines.len() as f32 + 12.0;
            let box_rect = Rect::from_min_size(pos2(rect.left() + 10.0, y), vec2(w, h));
            painter.rect_filled(box_rect, 8.0, fade(colors.panel));
            painter.rect_stroke(
                box_rect,
                8.0,
                Stroke::new(1.0, fade(colors.dialog_line)),
                egui::StrokeKind::Outside,
            );
            for (index, text) in lines.iter().enumerate() {
                painter.text(
                    pos2(
                        box_rect.left() + 10.0,
                        box_rect.top() + 6.0 + line * index as f32,
                    ),
                    egui::Align2::LEFT_TOP,
                    text.as_str(),
                    font_id.clone(),
                    fade(message_color),
                );
            }
            y += h + 6.0;
        }
        if !self.info.is_empty() {
            let font_id = skin::mono((size * 0.62).max(11.0));
            let line = font_id.size * 1.35;
            let max_line_width = (rect.width() - 40.0).max(60.0);
            let lines: Vec<String> = self
                .info
                .split('\n')
                .map(|l| skin::elide(&painter, l, &font_id, max_line_width))
                .collect();
            let w = lines
                .iter()
                .map(|l| skin::text_width(&painter, l, &font_id))
                .fold(0.0f32, f32::max)
                + 20.0;
            let h = line * lines.len() as f32 + 14.0;
            let box_rect = Rect::from_min_size(pos2(rect.left() + 10.0, y), vec2(w, h));
            painter.rect_filled(
                box_rect,
                ui.style().visuals.menu_corner_radius,
                colors.dialog_pane,
            );
            painter.rect_stroke(
                box_rect,
                ui.style().visuals.menu_corner_radius,
                Stroke::new(1.0, colors.dialog_line),
                egui::StrokeKind::Outside,
            );
            for (index, text) in lines.iter().enumerate() {
                painter.text(
                    pos2(
                        box_rect.left() + 10.0,
                        box_rect.top() + 7.0 + line * index as f32,
                    ),
                    egui::Align2::LEFT_TOP,
                    text.as_str(),
                    font_id.clone(),
                    colors.text_e4,
                );
            }
        }
        if !self.center.is_empty() {
            let font_id = skin::font((size - 4.0).max(14.0), true);
            let w = skin::text_width(&painter, &self.center, &font_id);
            let pos = pos2(
                rect.center().x - w / 2.0,
                rect.center().y - font_id.size * 0.7,
            );
            outlined_text(&painter, pos, &self.center, &font_id, colors.text_bright);
        }
    }
}

/// Text with a black outline and a round-joined stroke.
pub fn outlined_text(
    painter: &Painter,
    pos: Pos2,
    text: &str,
    font_id: &egui::FontId,
    color: Color32,
) {
    for (dx, dy) in [
        (-1.0f32, -1.0f32),
        (0.0, -1.0),
        (1.0, -1.0),
        (-1.0, 0.0),
        (1.0, 0.0),
        (-1.0, 1.0),
        (0.0, 1.0),
        (1.0, 1.0),
    ] {
        painter.text(
            pos2(pos.x + dx, pos.y + dy),
            egui::Align2::LEFT_TOP,
            text,
            font_id.clone(),
            skin::active_colors().video_bg,
        );
    }
    painter.text(pos, egui::Align2::LEFT_TOP, text, font_id.clone(), color);
}

/// Hide the cursor while the pointer sits still over a playing video.
pub fn apply_cursor(ctx: &egui::Context, hidden: bool) {
    ctx.set_cursor_icon(if hidden {
        CursorIcon::None
    } else {
        CursorIcon::Default
    });
}

pub fn fmt_pair(position: f64, duration: f64) -> (String, String) {
    (fmt_time(position), fmt_time(duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_geometry_handles_empty_or_invalid_ranges() {
        let mut seek = SeekUi {
            duration: 120.0,
            ..Default::default()
        };
        assert_eq!(seek.time_at(10.0, 0.0), 0.0);
        assert_eq!(seek.x_of(30.0, 0.0), 0.0);

        seek.duration = f64::NAN;
        assert_eq!(seek.time_at(10.0, 100.0), 0.0);
        assert_eq!(seek.x_of(30.0, 100.0), 5.0);
    }
}
