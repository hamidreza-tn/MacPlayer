//! Skinned chrome widgets drawn directly with egui shapes on a stable pixel grid.
//!
//! Custom MacPlayer widgets: each widget paints into a fixed rect and reports pointer
//! interaction without relying on stock media-player controls.

use std::collections::VecDeque;
use std::time::Instant;

use egui::{
    Color32, ColorImage, CursorIcon, Id, Painter, Pos2, Rect, Sense, Stroke, TextureOptions, Ui,
    Vec2, Widget, pos2, vec2,
};

use crate::skin;
use crate::util::fmt_time;

/// A response returned by the chrome widgets.
#[derive(Default, Clone, Copy)]
pub struct Click {
    pub primary: bool,
    pub secondary: bool,
    pub double: bool,
    pub hovered: bool,
}

impl Click {
    pub fn any(&self) -> bool {
        self.primary || self.secondary || self.double
    }
}

pub const MAX_THUMBNAILS: usize = 24;
pub const MAX_THUMBNAIL_EDGE: usize = 640;
pub const MAX_THUMBNAIL_PIXELS: usize = 640 * 360;

#[derive(Clone)]
pub struct Thumbnail {
    pub time: f64,
    pub image: ColorImage,
}

impl Thumbnail {
    pub fn new(time: f64, image: ColorImage) -> Option<Self> {
        let pixels = image.size[0].checked_mul(image.size[1])?;
        (time.is_finite()
            && time >= 0.0
            && image.size[0] > 0
            && image.size[1] > 0
            && image.size[0] <= MAX_THUMBNAIL_EDGE
            && image.size[1] <= MAX_THUMBNAIL_EDGE
            && pixels <= MAX_THUMBNAIL_PIXELS
            && image.pixels.len() == pixels)
            .then_some(Self { time, image })
    }
}

pub struct ThumbnailCache {
    entries: VecDeque<Thumbnail>,
    capacity: usize,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new(MAX_THUMBNAILS)
    }
}

impl ThumbnailCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: capacity.min(MAX_THUMBNAILS),
        }
    }

    pub fn insert(&mut self, thumbnail: Thumbnail) -> bool {
        let Some(pixels) = thumbnail.image.size[0].checked_mul(thumbnail.image.size[1]) else {
            return false;
        };
        if thumbnail.image.size[0] == 0
            || thumbnail.image.size[1] == 0
            || thumbnail.image.size[0] > MAX_THUMBNAIL_EDGE
            || thumbnail.image.size[1] > MAX_THUMBNAIL_EDGE
            || thumbnail.image.pixels.len() > MAX_THUMBNAIL_PIXELS
            || thumbnail.image.pixels.len() != pixels
            || !thumbnail.time.is_finite()
            || thumbnail.time < 0.0
            || self.capacity == 0
        {
            return false;
        }
        self.entries.retain(|item| item.time != thumbnail.time);
        self.entries.push_back(thumbnail);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
        true
    }

    pub fn insert_image(&mut self, time: f64, image: ColorImage) -> bool {
        let Some(thumbnail) = Thumbnail::new(time, image) else {
            return false;
        };
        self.insert(thumbnail)
    }

    pub fn get(&self, time: f64) -> Option<&Thumbnail> {
        self.entries.iter().find(|item| item.time == time)
    }

    pub fn nearest(&self, time: f64) -> Option<&Thumbnail> {
        if !time.is_finite() {
            return None;
        }
        self.entries.iter().min_by(|left, right| {
            (left.time - time)
                .abs()
                .total_cmp(&(right.time - time).abs())
        })
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

pub struct ThumbnailPreview<'a> {
    cache: &'a ThumbnailCache,
    time: f64,
    size: Vec2,
}

impl<'a> ThumbnailPreview<'a> {
    pub fn new(cache: &'a ThumbnailCache, time: f64, size: Vec2) -> Self {
        Self { cache, time, size }
    }
}

impl Widget for ThumbnailPreview<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(self.size, Sense::hover());
        paint_thumbnail_preview(ui, rect, self.cache, self.time);
        response
    }
}

pub fn thumbnail_preview(ui: &mut Ui, rect: Rect, cache: &ThumbnailCache, time: f64) -> bool {
    paint_thumbnail_preview(ui, rect, cache, time)
}

fn paint_thumbnail_preview(ui: &mut Ui, rect: Rect, cache: &ThumbnailCache, time: f64) -> bool {
    let colors = skin::active_colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.video_bg);
    let Some(thumbnail) = cache.nearest(time) else {
        return false;
    };
    let texture = ui.ctx().load_texture(
        "macplayer-thumbnail-preview",
        thumbnail.image.clone(),
        TextureOptions::LINEAR,
    );
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    true
}

/// Icon button: a fixed-size rect with a hand-drawn glyph, hover/press background and tooltip.
pub struct IconBtn<'a> {
    pub icon: &'a str,
    pub w: f32,
    pub h: f32,
    pub tip: Option<&'a str>,
    pub color: Color32,
    pub hover_color: Color32,
    pub flat: bool,
    pub checked: bool,
    pub enabled: bool,
}

impl<'a> IconBtn<'a> {
    pub fn new(icon: &'a str, w: f32, h: f32) -> Self {
        let colors = skin::active_colors();
        Self {
            icon,
            w,
            h,
            tip: None,
            color: colors.icon,
            hover_color: colors.icon_hover,
            flat: false,
            checked: false,
            enabled: true,
        }
    }

    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    pub fn hover_color(mut self, color: Color32) -> Self {
        self.hover_color = color;
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
        let corner_radius = ui.style().visuals.widgets.inactive.corner_radius;
        let mut bg = colors.bg;
        if !self.flat && self.enabled {
            if down {
                bg = colors.bg_press;
            } else if hovered {
                bg = colors.bg_hover;
            }
        }
        painter.rect_filled(rect, corner_radius, bg);
        painter.rect_stroke(
            rect,
            corner_radius,
            Stroke::new(1.0, colors.line),
            egui::StrokeKind::Outside,
        );
        let color = if !self.enabled {
            colors.icon_off
        } else if self.checked {
            colors.accent
        } else if hovered {
            self.hover_color
        } else {
            self.color
        };
        let center = rect.center();
        skin::draw_icon(
            &painter,
            self.icon,
            center.x.round(),
            center.y.round(),
            color,
            bg,
        );

        Click {
            primary: response.clicked(),
            secondary: response.secondary_clicked(),
            double: response.double_clicked(),
            hovered,
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

// -- title bar -----------------------------------------------------------------------------------

pub struct TitleBar {
    pub menu_hover: bool,
    pub menu_click: Click,
    pub buttons: [Click; 5],
    pub drag_started: bool,
    pub double: bool,
}

impl Default for TitleBar {
    fn default() -> Self {
        Self {
            menu_hover: false,
            menu_click: Click::default(),
            buttons: [Click::default(); 5],
            drag_started: false,
            double: false,
        }
    }
}

/// Draw the title bar into `rect`. `press_origin` is the drag anchor from the previous frame.
pub fn title_bar(
    ui: &mut Ui,
    rect: Rect,
    title: &str,
    app_name: &str,
    icons: [&str; 5],
    checked: [bool; 5],
    press_origin: &mut Option<Pos2>,
) -> TitleBar {
    let colors = skin::active_colors();
    let mut out = TitleBar::default();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.bg);

    // Keep the application controls on the left. The native window decorations
    // are disabled, so these are the only window controls rendered by the app.
    let controls_w = (rect.width().max(0.0)).min(skin::TITLE_BTN_W * icons.len() as f32);
    let controls_left = rect.left();
    let controls_right = controls_left + controls_w;
    let button_w = if icons.is_empty() {
        0.0
    } else {
        controls_w / icons.len() as f32
    };
    let mut x = controls_left;
    for (index, icon) in icons.iter().enumerate() {
        let btn_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(button_w, rect.height()));
        let hover_color = if *icon == "close" {
            colors.close_hover
        } else {
            colors.icon_hover
        };
        let click = IconBtn::new(icon, button_w, rect.height())
            .color(colors.icon_dim)
            .hover_color(hover_color)
            .flat(true)
            .checked(checked[index])
            .ui(ui, ui.id().with(format!("title-btn-{index}")), btn_rect);
        out.buttons[index] = click;
        x += button_w;
    }
    if controls_w > 0.0 {
        painter.line_segment(
            [
                pos2(controls_right + 5.0, rect.top()),
                pos2(controls_right + 5.0, rect.bottom()),
            ],
            Stroke::new(1.0, colors.line),
        );
    }

    // Menu button follows the window controls on the left.
    let menu_w = {
        let text_w = skin::text_width(&painter, app_name, &skin::font(12.0, false));
        let preferred = skin::MENU_BTN_W.max(11.0 + text_w + 11.0 + 14.0);
        preferred.min((rect.right() - controls_right - 1.0).max(0.0))
    };
    let menu_rect = Rect::from_min_size(
        pos2(controls_right + 6.0, rect.top()),
        vec2(menu_w, rect.height()),
    );
    let menu = ui.interact(menu_rect, ui.id().with("title-menu"), Sense::click());
    let menu = menu.on_hover_text("Menu");
    out.menu_hover = menu.hovered();
    if menu.hovered() {
        painter.rect_filled(menu_rect, 0.0, colors.bg_hover);
    }
    if menu_w > 0.0 {
        let app_name = skin::elide(
            &painter,
            app_name,
            &skin::font(12.0, false),
            (menu_w - 25.0).max(0.0),
        );
        painter.text(
            pos2(menu_rect.min.x + 11.0, rect.min.y + 15.5),
            egui::Align2::LEFT_CENTER,
            app_name,
            skin::font(12.0, false),
            if menu.hovered() {
                colors.text_bright
            } else {
                colors.text
            },
        );
        if menu_w >= 14.0 {
            skin::draw_icon(
                &painter,
                "chevron",
                menu_rect.right() - 14.0,
                rect.min.y + 13.0,
                colors.text_bright,
                if menu.hovered() {
                    colors.bg_hover
                } else {
                    colors.bg
                },
            );
        }
    }
    out.menu_click = Click {
        primary: menu.clicked(),
        secondary: menu.secondary_clicked(),
        double: false,
        hovered: menu.hovered(),
    };
    painter.line_segment(
        [
            pos2(menu_rect.right(), rect.top()),
            pos2(menu_rect.right(), rect.bottom()),
        ],
        Stroke::new(1.0, colors.line),
    );

    // Title text, elided in the middle.
    if !title.is_empty() {
        let title_left = menu_rect.right() + 11.0;
        let room = rect.right() - title_left - 11.0;
        let font_id = skin::font(12.0, false);
        let text = skin::elide_middle(&painter, title, &font_id, room.max(0.0));
        painter.text(
            pos2(title_left, rect.min.y + 15.5),
            egui::Align2::LEFT_CENTER,
            text,
            font_id,
            colors.text,
        );
    }

    // Drag / double-click on the bar itself.
    let bar = ui.interact(
        Rect::from_min_max(
            pos2((menu_rect.right() + 1.0).min(rect.right()), rect.top()),
            pos2(rect.right(), rect.bottom()),
        ),
        ui.id().with("title-drag"),
        Sense::click_and_drag(),
    );
    if bar.drag_started_by(egui::PointerButton::Primary) {
        *press_origin = Some(bar.interact_pointer_pos().unwrap_or(rect.center()));
    }
    if let Some(origin) = *press_origin {
        if bar.dragged_by(egui::PointerButton::Primary) {
            let delta = bar.interact_pointer_pos().unwrap_or(origin) - origin;
            if delta.length() > 4.0 {
                *press_origin = None;
                out.drag_started = true;
            }
        } else {
            *press_origin = None;
        }
    }
    out.double = bar.double_clicked_by(egui::PointerButton::Primary);
    out
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

pub fn seek_bar(
    ui: &mut Ui,
    rect: Rect,
    id_salt: Id,
    state: &mut SeekUi,
    now: Instant,
) -> SeekBarOut {
    let colors = skin::active_colors();
    let mut out = SeekBarOut {
        seek: None,
        hover: None,
        wheel: 0,
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.bg);

    let response = ui.interact(rect, id_salt, Sense::click_and_drag());
    let was_hovered = state.hover;
    state.hover = response.hovered();
    let y = (rect.height() / 2.0 - 1.0).round();
    let (left, span) = state.span(rect.width());
    painter.rect_filled(
        Rect::from_min_size(pos2(rect.left() + left, rect.top() + y), vec2(span, 2.0)),
        0.0,
        colors.line,
    );

    if response.hovered() && ui.input(|i| i.smooth_scroll_delta.y != 0.0) {
        out.wheel = if ui.input(|i| i.smooth_scroll_delta.y) > 0.0 {
            1
        } else {
            -1
        };
    }
    if response.hovered() {
        if let Some(pos) = response.hover_pos() {
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
                Rect::from_min_size(pos2(rect.left() + x, rect.top() + y), vec2(x_end - x, 2.0)),
                0.0,
                colors.buffer,
            );
        }
        let (a, b) = state.ab;
        if let (Some(a), Some(b)) = (a, b) {
            if b > a {
                let xa = state.x_of(a, rect.width());
                let xb = state.x_of(b, rect.width());
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(rect.left() + xa, rect.top() + y - 2.0),
                        vec2(xb - xa, 1.0),
                    ),
                    0.0,
                    colors.accent,
                );
            }
        }
        painter.rect_filled(
            Rect::from_min_size(
                pos2(rect.left() + left, rect.top() + y),
                vec2((x - left).max(0.0), 2.0),
            ),
            0.0,
            colors.accent,
        );
        for chapter in &state.chapters {
            if *chapter > 0.0 && *chapter < state.duration {
                let cx = state.x_of(*chapter, rect.width()).round();
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(rect.left() + cx, rect.top() + y - 2.0),
                        vec2(1.0, 6.0),
                    ),
                    0.0,
                    colors.text_dim,
                );
            }
        }
        for point in [a, b].into_iter().flatten() {
            let px = state.x_of(point, rect.width()).round();
            painter.rect_filled(
                Rect::from_min_size(pos2(rect.left() + px, rect.top() + y - 4.0), vec2(1.0, 4.0)),
                0.0,
                colors.accent,
            );
        }
        let radius = if state.hover || state.dragging {
            4.5
        } else {
            4.0
        };
        painter.circle_filled(
            pos2(
                (rect.left() + x).max(rect.left() + left),
                rect.top() + y + 1.0,
            ),
            radius,
            colors.knob,
        );
    }

    out
}

// -- volume bar ----------------------------------------------------------------------------------

#[derive(Default)]
pub struct VolumeUi {
    pub volume: i64,
    pub muted: bool,
    pub dragging: bool,
    pub hover_icon: bool,
}

pub struct VolumeOut {
    pub volume: Option<i64>,
    pub mute: bool,
    pub wheel: i32,
}

pub fn volume_bar(ui: &mut Ui, rect: Rect, id_salt: Id, state: &mut VolumeUi) -> VolumeOut {
    let colors = skin::active_colors();
    let mut out = VolumeOut {
        volume: None,
        mute: false,
        wheel: 0,
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.bg);
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
    let response = response.on_hover_text(format!("Volume: {}%", state.volume));

    let y = (rect.height() / 2.0 - 1.0).round();
    let icon = if state.muted || state.volume == 0 {
        "volume_mute"
    } else if state.volume < 50 {
        "volume_low"
    } else {
        "volume"
    };
    skin::draw_icon(
        &painter,
        icon,
        rect.left() + (width * 0.11).min(11.0),
        rect.center().y,
        if state.hover_icon {
            colors.icon_hover
        } else {
            colors.icon_dim
        },
        colors.bg,
    );
    let knob_x = knob_x0 + (knob_x1 - knob_x0) * state.volume as f32 / 100.0;
    painter.rect_filled(
        Rect::from_min_size(
            pos2(rect.left() + line_x0, rect.top() + y),
            vec2((line_x1 - line_x0).max(0.0), 2.0),
        ),
        0.0,
        colors.line,
    );
    let fill = if state.muted {
        colors.text_dim
    } else {
        colors.accent
    };
    painter.rect_filled(
        Rect::from_min_size(
            pos2(rect.left() + line_x0, rect.top() + y),
            vec2((knob_x - line_x0).max(0.0), 2.0),
        ),
        0.0,
        fill,
    );
    painter.circle_filled(
        pos2(rect.left() + knob_x, rect.top() + y + 1.0),
        4.0,
        colors.knob,
    );

    if response.hovered() && ui.input(|i| i.smooth_scroll_delta.y != 0.0) {
        out.wheel = if ui.input(|i| i.smooth_scroll_delta.y) > 0.0 {
            1
        } else {
            -1
        };
    }
    if !state.dragging && ui.input(|i| i.pointer.primary_pressed()) && response.hovered() {
        let pos = response.interact_pointer_pos().unwrap_or(rect.center());
        if pos.x - rect.left() < line_x0 {
            out.mute = true;
        } else {
            state.dragging = true;
            out.volume = Some(volume_at(pos.x - rect.left(), knob_x0, knob_x1));
        }
    }
    if state.dragging {
        if let Some(pos) = response.interact_pointer_pos() {
            if ui.input(|i| i.pointer.primary_down()) {
                out.volume = Some(volume_at(pos.x - rect.left(), knob_x0, knob_x1));
            } else {
                state.dragging = false;
            }
        } else {
            state.dragging = false;
        }
    }
    out
}

fn volume_at(x: f32, knob_x0: f32, knob_x1: f32) -> i64 {
    if knob_x1 <= knob_x0 {
        return if x >= knob_x1 { 100 } else { 0 };
    }
    let frac = (x - knob_x0) / (knob_x1 - knob_x0);
    ((frac.clamp(0.0, 1.0) * 100.0).round()) as i64
}

// -- time label ----------------------------------------------------------------------------------

/// Width the time label wants for the given text parts.
pub fn time_label_width(painter: &Painter, left: &str, right: &str) -> f32 {
    skin::text_width(painter, left, &skin::font(11.0, false))
        + skin::text_width(painter, right, &skin::font(11.0, false))
        + 3.0
}

pub fn time_label(ui: &mut Ui, rect: Rect, id_salt: Id, left: &str, right: &str) -> bool {
    let colors = skin::active_colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.bg);
    let font_id = skin::font(11.0, false);
    let center_y = rect.center().y + 1.0;
    painter.text(
        pos2(rect.left(), center_y),
        egui::Align2::LEFT_CENTER,
        left,
        font_id.clone(),
        colors.text_bright,
    );
    let offset = skin::text_width(&painter, left, &font_id);
    painter.text(
        pos2(rect.left() + offset, center_y),
        egui::Align2::LEFT_CENTER,
        right,
        font_id,
        colors.text_dim,
    );
    let response = ui.interact(rect, id_salt, Sense::click());
    let response = response.on_hover_text("Click to switch between elapsed and remaining time");
    response.clicked()
}

// -- info badges ---------------------------------------------------------------------------------

pub struct Badge {
    pub text: String,
    pub on: bool,
}

pub fn info_badges(ui: &mut Ui, rect: Rect, badges: &[Badge]) {
    if badges.is_empty() {
        return;
    }
    let colors = skin::active_colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.bg);
    let font_id = skin::font(10.0, false);
    let mut x = rect.left();
    for badge in badges {
        let w = skin::text_width(&painter, &badge.text, &font_id);
        if x + w > rect.right() {
            break;
        }
        painter.text(
            pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            badge.text.clone(),
            font_id.clone(),
            if badge.on {
                colors.accent
            } else {
                colors.text_dim
            },
        );
        x += w + 12.0;
    }
}

// -- seek bubble ---------------------------------------------------------------------------------

pub fn seek_bubble(ui: &mut Ui, area: Rect, text: &str) {
    let colors = skin::active_colors();
    let font_id = skin::font(11.0, false);
    let painter = ui.painter();
    let w = skin::text_width(painter, text, &font_id) + 14.0;
    let h = 20.0;
    let pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(area.center());
    let x = (pos.x - w / 2.0).clamp(area.left() + 2.0, area.right() - w - 2.0);
    let rect = Rect::from_min_size(pos2(x, area.top() - h), vec2(w, h));
    let painter = ui.painter_at(rect);
    let corner_radius = ui.style().visuals.menu_corner_radius;
    painter.rect_filled(rect, corner_radius, colors.dialog_line);
    painter.rect_filled(rect.shrink(1.0), corner_radius, colors.dialog_bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font_id,
        colors.text_bright,
    );
}

// -- on-screen display ----------------------------------------------------------------------------

#[derive(Default)]
pub struct Osd {
    pub message: String,
    pub message_color: Color32,
    pub message_until: Option<Instant>,
    pub info: String,
    pub center: String,
}

impl Osd {
    pub fn show_message(&mut self, text: impl Into<String>, ms: u64, color: Color32) {
        self.message = text.into();
        self.message_color = color;
        self.message_until = Some(Instant::now() + std::time::Duration::from_millis(ms));
    }

    pub fn clear_message(&mut self) {
        self.message.clear();
        self.message_until = None;
    }

    pub fn tick(&mut self) {
        if let Some(until) = self.message_until {
            if Instant::now() >= until {
                self.clear_message();
            }
        }
    }

    pub fn paint(&self, ui: &mut Ui, rect: Rect) {
        if self.message.is_empty() && self.info.is_empty() && self.center.is_empty() {
            return;
        }
        let colors = skin::active_colors();
        let painter = ui.painter_at(rect);
        let size = (15.0f32).max((30.0f32).min(rect.height() / 22.0));
        let mut y = rect.top() + 8.0;
        if !self.message.is_empty() {
            let font_id = skin::font(size, true);
            let classic = skin::ThemeColors::classic();
            let message_color = if self.message_color == classic.danger {
                colors.danger
            } else if self.message_color == classic.text_e4 {
                colors.text_e4
            } else {
                self.message_color
            };
            outlined_text(
                &painter,
                pos2(rect.left() + 12.0, y),
                &self.message,
                &font_id,
                message_color,
            );
            let line = size * 1.35;
            y += line * (self.message.matches('\n').count() as f32 + 1.0) + 6.0;
        }
        if !self.info.is_empty() {
            let font_id = skin::mono((size * 0.62).max(11.0));
            let line = font_id.size * 1.35;
            let lines: Vec<&str> = self.info.split('\n').collect();
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
                    *text,
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

// -- idle logo -----------------------------------------------------------------------------------

pub fn paint_logo(ui: &mut Ui, rect: Rect, audio_title: &str) {
    let colors = skin::active_colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.video_bg);
    let scale = 0.35f32.max(1.0f32.min((rect.width() / 400.0).min(rect.height() / 150.0)));
    skin::draw_logo(
        &painter,
        rect.center().x,
        rect.center().y,
        scale,
        "MacPlayer",
    );
    if !audio_title.is_empty() {
        let font_id = skin::font(13.0, false);
        let text = skin::elide(
            &painter,
            audio_title,
            &font_id,
            (rect.width() - 20.0).max(0.0),
        );
        painter.text(
            pos2(
                rect.center().x,
                rect.top() + rect.height() / 2.0 + 44.0 * scale,
            ),
            egui::Align2::CENTER_TOP,
            text,
            font_id,
            colors.text_dim,
        );
    }
}

/// Hide the cursor while the pointer sits still over a playing video.
pub fn apply_cursor(ctx: &egui::Context, hidden: bool) {
    ctx.set_cursor_icon(if hidden {
        CursorIcon::None
    } else {
        CursorIcon::Default
    });
}

pub fn vec_to_pos(v: Vec2) -> Pos2 {
    pos2(v.x, v.y)
}

pub fn fmt_pair(position: f64, duration: f64) -> (String, String) {
    (fmt_time(position), fmt_time(duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize) -> ColorImage {
        ColorImage::filled([width, height], Color32::WHITE)
    }

    #[test]
    fn thumbnail_cache_is_bounded_and_replaces_same_time() {
        let mut cache = ThumbnailCache::new(2);
        assert!(cache.insert_image(1.0, image(2, 2)));
        assert!(cache.insert_image(1.0, image(3, 2)));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(1.0).unwrap().image.size, [3, 2]);
        assert!(cache.insert_image(2.0, image(2, 2)));
        assert!(cache.insert_image(3.0, image(2, 2)));
        assert_eq!(cache.len(), 2);
        assert!(cache.get(1.0).is_none());
        assert_eq!(cache.nearest(2.2).unwrap().time, 2.0);
    }

    #[test]
    fn thumbnail_cache_rejects_oversized_images_and_invalid_times() {
        let mut cache = ThumbnailCache::default();
        assert!(!cache.insert_image(f64::NAN, image(2, 2)));
        assert!(!cache.insert_image(
            1.0,
            ColorImage::filled([MAX_THUMBNAIL_EDGE, MAX_THUMBNAIL_EDGE], Color32::WHITE),
        ));
        assert!(cache.is_empty());
    }

    #[test]
    fn seek_geometry_handles_empty_or_invalid_ranges() {
        let mut seek = SeekUi::default();
        seek.duration = 120.0;
        assert_eq!(seek.time_at(10.0, 0.0), 0.0);
        assert_eq!(seek.x_of(30.0, 0.0), 0.0);

        seek.duration = f64::NAN;
        assert_eq!(seek.time_at(10.0, 100.0), 0.0);
        assert_eq!(seek.x_of(30.0, 100.0), 5.0);
    }
}
