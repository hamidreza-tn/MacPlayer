//! Preferences / Control Panel / GoTo / Text / URL dialogs.
//!
//! Each dialog is an egui `Window` on a custom frame, styled with the MacPlayer palette.

use crate::engine::{AudioTrackInfo, AudioTrackState, FileTrackState, TrackSelection};
use crate::menus::Action;
use crate::menus::{TrackKind, TrackSel};
use crate::settings::Settings;
use crate::skin;
use crate::skin::*;
use crate::util::parse_time;
use crate::value::Value;
use egui::{Align2, Frame, Id, Rect, RichText, Sense, Stroke, TextEdit, Window, pos2, vec2};

fn active_colors() -> skin::ThemeColors {
    skin::active_colors()
}

// -- shared chrome ---------------------------------------------------------------------------------

fn dlg_frame() -> Frame {
    Frame::new()
        .fill(active_colors().dialog_bg)
        .stroke(Stroke::new(1.0, active_colors().dialog_line))
        .inner_margin(egui::Margin::same(10))
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(13.0)
            .color(active_colors().text_e4)
            .strong(),
    );
    ui.add_space(6.0);
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(active_colors().text_a8),
    );
}

/// A row containing a label and its associated widget.
fn row<T>(ui: &mut egui::Ui, text: &str, add: impl FnOnce(&mut egui::Ui) -> T) -> T {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        let w = 190.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
        ui.painter().text(
            r.left_center(),
            Align2::LEFT_CENTER,
            text,
            font(12.0, false),
            active_colors().text_a8,
        );
        out = Some(add(ui));
    });
    out.expect("row closure ran")
}

fn check(ui: &mut egui::Ui, id: &str, text: &str, value: &mut bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    let p = ui.painter_at(r);
    let hover = resp.hovered();
    p.rect_filled(
        r,
        2.0,
        if *value {
            active_colors().accent
        } else if hover {
            active_colors().button_hover
        } else {
            active_colors().base
        },
    );
    if *value {
        for i in 0..4 {
            p.rect_filled(
                Rect::from_min_size(
                    pos2(r.left() + 3.0 + i as f32, r.top() + 7.0 + i as f32),
                    vec2(2.0, 1.0),
                ),
                0.0,
                active_colors().bg,
            );
            if i > 0 {
                p.rect_filled(
                    Rect::from_min_size(
                        pos2(r.left() + 3.0 + i as f32, r.top() + 11.0 - i as f32),
                        vec2(2.0, 1.0),
                    ),
                    0.0,
                    active_colors().bg,
                );
            }
        }
    }
    let clicked = resp.clicked();
    if clicked {
        *value = !*value;
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(text)
            .size(12.0)
            .color(active_colors().text_e4),
    );
    let _ = id;
    clicked
}

/// A `QSpinBox`-alike: numeric text field with up/down arrows.
fn spin(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut f64,
    step: f64,
    lo: f64,
    hi: f64,
    suffix: &str,
) -> bool {
    let mut changed = false;
    let text_id = Id::new(("spin-text", id));
    let mut text = ui
        .ctx()
        .data(|data| data.get_temp::<String>(text_id))
        .unwrap_or_else(|| format!("{value:.0}"));
    let r = ui.add(
        TextEdit::singleline(&mut text)
            .desired_width(64.0)
            .font(egui::TextStyle::Monospace)
            .margin(vec2(3.0, 2.0)),
    );
    if r.changed() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(text_id, text.clone()));
    }
    if r.lost_focus() {
        if let Ok(v) = text.trim().parse::<f64>() {
            if (v - *value).abs() > f64::EPSILON {
                *value = v.clamp(lo, hi);
                changed = true;
            }
        }
        ui.ctx().data_mut(|data| data.remove::<String>(text_id));
    }
    ui.add_space(2.0);
    let arrow = |ui: &mut egui::Ui, up: bool| {
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 11.0), Sense::click());
        let p = ui.painter_at(r);
        p.rect_filled(
            r,
            1.0,
            if resp.hovered() {
                active_colors().button_hover
            } else {
                active_colors().button_bg
            },
        );
        let c = r.center();
        for i in 0..3 {
            let y = if up {
                c.y + 1.0 - i as f32
            } else {
                c.y - 1.0 + i as f32
            };
            p.rect_filled(
                Rect::from_min_size(
                    pos2(c.x - 1.0 - i as f32, y),
                    vec2(2.0 + 2.0 * i as f32, 1.0),
                ),
                0.0,
                active_colors().text_e4,
            );
        }
        resp.clicked()
    };
    if arrow(ui, true) {
        *value = (*value + step).min(hi);
        changed = true;
    }
    if arrow(ui, false) {
        *value = (*value - step).max(lo);
        changed = true;
    }
    if !suffix.is_empty() {
        ui.add_space(3.0);
        ui.label(
            RichText::new(suffix)
                .size(12.0)
                .color(active_colors().text_a8),
        );
    }
    let _ = id;
    changed
}

fn line_edit(ui: &mut egui::Ui, id: &str, value: &mut String, width: f32, hint: &str) -> bool {
    let mut text = value.clone();
    let r = ui.add(
        TextEdit::singleline(&mut text)
            .desired_width(width)
            .font(egui::TextStyle::Monospace)
            .margin(vec2(3.0, 2.0))
            .hint_text(
                RichText::new(hint)
                    .size(12.0)
                    .color(active_colors().text_9a),
            ),
    );
    let changed = r.changed();
    if changed {
        *value = text;
    }
    let _ = id;
    changed
}

/// A `QComboBox` over `(label, value)` pairs.
fn combo<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    id: &str,
    items: &[(String, T)],
    current: &T,
    width: f32,
) -> Option<T> {
    let mut picked = None;
    let mut text = String::from("—");
    for (label, v) in items {
        if v == current {
            text = label.clone();
        }
    }
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(
            RichText::new(text)
                .size(12.0)
                .color(active_colors().text_e4),
        )
        .show_ui(ui, |ui| {
            for (label, v) in items {
                if ui
                    .selectable_label(
                        v == current,
                        RichText::new(label.as_str())
                            .size(12.0)
                            .color(active_colors().text_e4),
                    )
                    .clicked()
                {
                    picked = Some(v.clone());
                }
            }
        });
    picked
}

fn button(ui: &mut egui::Ui, _id: &str, text: &str) -> bool {
    ui.add_sized(
        vec2((text.len() as f32) * 7.0 + 26.0, 24.0),
        egui::Button::new(
            RichText::new(text)
                .size(12.0)
                .color(active_colors().text_e4),
        ),
    )
    .clicked()
}

fn ok_cancel_apply(ui: &mut egui::Ui) -> (bool, bool, bool, bool) {
    let mut ok = false;
    let mut cancel = false;
    let mut apply = false;
    let mut close = false;
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        if ui
            .add_sized(
                vec2(84.0, 26.0),
                egui::Button::new(
                    RichText::new("OK")
                        .size(12.0)
                        .color(active_colors().text_e4),
                ),
            )
            .clicked()
        {
            ok = true;
        }
        ui.add_space(4.0);
        if ui
            .add_sized(
                vec2(84.0, 26.0),
                egui::Button::new(
                    RichText::new("Cancel")
                        .size(12.0)
                        .color(active_colors().text_e4),
                ),
            )
            .clicked()
        {
            cancel = true;
        }
        ui.add_space(4.0);
        if ui
            .add_sized(
                vec2(84.0, 26.0),
                egui::Button::new(
                    RichText::new("Apply")
                        .size(12.0)
                        .color(active_colors().text_e4),
                ),
            )
            .clicked()
        {
            apply = true;
        }
        ui.add_space(4.0);
        if ui
            .add_sized(
                vec2(84.0, 26.0),
                egui::Button::new(
                    RichText::new("Close")
                        .size(12.0)
                        .color(active_colors().text_e4),
                ),
            )
            .clicked()
        {
            close = true;
        }
    });
    (ok, cancel, apply, close)
}

fn tabs(ui: &mut egui::Ui, names: &[&str], current: &mut usize) {
    let start_x = ui.cursor().left();
    let mut x = start_x;
    let y = ui.cursor().top();
    for (i, name) in names.iter().enumerate() {
        let w = (name.chars().count() as f32) * 7.0 + 30.0;
        let rect = Rect::from_min_size(pos2(x, y), vec2(w, 24.0));
        let resp = ui.interact(rect, Id::new(("tab", i)), Sense::click());
        let p = ui.painter_at(rect);
        let sel = i == *current;
        p.rect_filled(
            rect,
            0.0,
            if sel {
                active_colors().tab_sel
            } else if resp.hovered() {
                active_colors().button_hover
            } else {
                active_colors().tab_bg
            },
        );
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            *name,
            font(12.0, sel),
            if sel {
                active_colors().text_e4
            } else {
                active_colors().text_a8
            },
        );
        if resp.clicked() {
            *current = i;
        }
        x += w + 2.0;
    }
    ui.allocate_space(vec2((x - start_x).max(0.0), 24.0));
    ui.add_space(6.0);
}

// -- GoTo ------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum GoToMode {
    Time,
    Percent,
}

pub struct GoToDialog {
    pub open: bool,
    pub mode: GoToMode,
    pub input: String,
    pub error: Option<String>,
}

impl Default for GoToDialog {
    fn default() -> Self {
        Self {
            open: false,
            mode: GoToMode::Time,
            input: String::new(),
            error: None,
        }
    }
}

impl GoToDialog {
    pub fn open(&mut self, mode: GoToMode) {
        self.open = true;
        self.mode = mode;
        self.input.clear();
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, duration: f64, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut submit = false;
        Window::new("Go to")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_min_width(250.0);
                heading(ui, "Go to");
                ui.horizontal(|ui| {
                    let mut m = self.mode;
                    if ui
                        .selectable_label(
                            m == GoToMode::Time,
                            RichText::new("Time")
                                .size(12.0)
                                .color(active_colors().text_e4),
                        )
                        .clicked()
                    {
                        m = GoToMode::Time;
                    }
                    if ui
                        .selectable_label(
                            m == GoToMode::Percent,
                            RichText::new("Percent")
                                .size(12.0)
                                .color(active_colors().text_e4),
                        )
                        .clicked()
                    {
                        m = GoToMode::Percent;
                    }
                    self.mode = m;
                });
                ui.add_space(6.0);
                let mut input = self.input.clone();
                let r = ui.add(
                    TextEdit::singleline(&mut input)
                        .desired_width(230.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(
                            RichText::new("HH:MM:SS or 01:30")
                                .size(12.0)
                                .color(active_colors().text_9a),
                        ),
                );
                self.input = input;
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                if let Some(e) = &self.error {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(e.as_str())
                            .size(12.0)
                            .color(active_colors().danger),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            vec2(80.0, 24.0),
                            egui::Button::new(
                                RichText::new("OK")
                                    .size(12.0)
                                    .color(active_colors().text_e4),
                            ),
                        )
                        .clicked()
                    {
                        submit = true;
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            vec2(80.0, 24.0),
                            egui::Button::new(
                                RichText::new("Cancel")
                                    .size(12.0)
                                    .color(active_colors().text_e4),
                            ),
                        )
                        .clicked()
                    {
                        self.open = false;
                    }
                });
            });
        if submit {
            let s = self.input.trim().to_string();
            match self.mode {
                GoToMode::Percent => match s.trim_end_matches('%').trim().parse::<f64>() {
                    Ok(p) if p.is_finite() => {
                        let pct = p.clamp(0.0, 100.0) / 100.0;
                        actions.push(Action::SeekTo {
                            seconds: None,
                            percent: Some(pct),
                        });
                        self.open = false;
                    }
                    Ok(_) | Err(_) => self.error = Some("Please enter a number".into()),
                },
                GoToMode::Time => match parse_time(&s, Some(duration)) {
                    Some(secs) if secs.is_finite() => {
                        actions.push(Action::SeekTo {
                            seconds: Some(secs.max(0.0)),
                            percent: None,
                        });
                        self.open = false;
                    }
                    _ => self.error = Some("Please enter a valid time".into()),
                },
            }
        }
        self.open = self.open && open;
    }
}

// -- Text dialog -----------------------------------------------------------------------------------

pub struct TextDialog {
    pub open: bool,
    pub title: String,
    pub body: String,
    pub mono: bool,
    pub error: bool,
}

impl Default for TextDialog {
    fn default() -> Self {
        Self {
            open: false,
            title: String::new(),
            body: String::new(),
            mono: true,
            error: false,
        }
    }
}

impl TextDialog {
    pub fn set(&mut self, title: &str, body: &str, mono: bool, error: bool) {
        self.title = title.into();
        self.body = body.into();
        self.mono = mono;
        self.error = error;
        self.open = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        Window::new(self.title.as_str())
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(380.0)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    let c = r.center();
                    skin::draw_icon(
                        ui.painter(),
                        if self.error { "close" } else { "info" },
                        c.x.round(),
                        c.y.round(),
                        if self.error {
                            active_colors().danger
                        } else {
                            active_colors().accent
                        },
                        active_colors().dialog_bg,
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.title.as_str())
                            .size(13.0)
                            .color(active_colors().text_e4)
                            .strong(),
                    );
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("text-dialog-scroll")
                    .auto_shrink([false, false])
                    .max_height(300.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.body)
                                .desired_width(f32::INFINITY)
                                .font(if self.mono {
                                    egui::TextStyle::Monospace
                                } else {
                                    egui::TextStyle::Body
                                })
                                .interactive(false),
                        );
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            vec2(90.0, 24.0),
                            egui::Button::new(
                                RichText::new("Copy")
                                    .size(12.0)
                                    .color(active_colors().text_e4),
                            ),
                        )
                        .clicked()
                    {
                        ui.copy_text(self.body.clone());
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            vec2(90.0, 24.0),
                            egui::Button::new(
                                RichText::new("Close")
                                    .size(12.0)
                                    .color(active_colors().text_e4),
                            ),
                        )
                        .clicked()
                    {
                        self.open = false;
                    }
                });
            });
        self.open = self.open && open;
    }
}

// -- URL dialog ------------------------------------------------------------------------------------

pub struct UrlDialog {
    pub open: bool,
    pub url: String,
    pub loop_input: String,
    pub error: Option<String>,
}

impl Default for UrlDialog {
    fn default() -> Self {
        Self {
            open: false,
            url: String::new(),
            loop_input: "0".into(),
            error: None,
        }
    }
}

impl UrlDialog {
    pub fn show(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut submit = false;
        Window::new("Play URL")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -30.0))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                heading(ui, "Enter URL:");
                let mut url = self.url.clone();
                let r = ui.add(
                    TextEdit::singleline(&mut url)
                        .desired_width(320.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(
                            RichText::new("https://example.com/video.mp4")
                                .size(12.0)
                                .color(active_colors().text_9a),
                        ),
                );
                self.url = url;
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    label(ui, "Loop interval (seconds, 0 = no loop):");
                    let mut li = self.loop_input.clone();
                    ui.add(
                        TextEdit::singleline(&mut li)
                            .desired_width(52.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    self.loop_input = li;
                });
                if let Some(e) = &self.error {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(e.as_str())
                            .size(12.0)
                            .color(active_colors().danger),
                    );
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            vec2(84.0, 26.0),
                            egui::Button::new(
                                RichText::new("OK")
                                    .size(12.0)
                                    .color(active_colors().text_e4),
                            ),
                        )
                        .clicked()
                    {
                        submit = true;
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            vec2(84.0, 26.0),
                            egui::Button::new(
                                RichText::new("Cancel")
                                    .size(12.0)
                                    .color(active_colors().text_e4),
                            ),
                        )
                        .clicked()
                    {
                        self.open = false;
                    }
                });
            });
        if submit {
            let u = self.url.trim().to_string();
            if !crate::util::is_url(&u) {
                self.error = Some("Please enter a valid URL".into());
            } else {
                let loop_s: i64 = self.loop_input.trim().parse().unwrap_or(0);
                actions.push(Action::PlayUrl { url: u, loop_s });
                self.open = false;
            }
        }
        self.open = self.open && open;
    }
}

// -- Control panel ---------------------------------------------------------------------------------

/// Live copy of the mpv properties the Control Panel shows, refreshed on a timer.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPanelState {
    pub audio_tracks: Vec<AudioTrackInfo>,
    pub audio: AudioTrackState,
    pub tracks: FileTrackState,
    pub eq_enabled: bool,
    pub eq: Vec<f64>,
    pub normalize: bool,
    pub audio_delay: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub hue: f64,
    pub gamma: f64,
    pub sub_delay: f64,
    pub sub_scale: f64,
    pub sub_pos: f64,
    pub sub_visible: bool,
    pub speed: f64,
    pub ab_a: Option<f64>,
    pub ab_b: Option<f64>,
}

impl Default for ControlPanelState {
    fn default() -> Self {
        Self {
            audio_tracks: Vec::new(),
            audio: AudioTrackState::default(),
            tracks: FileTrackState::default(),
            eq_enabled: false,
            eq: vec![0.0; 10],
            normalize: false,
            audio_delay: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            hue: 0.0,
            gamma: 0.0,
            sub_delay: 0.0,
            sub_scale: 1.0,
            sub_pos: 100.0,
            sub_visible: true,
            speed: 1.0,
            ab_a: None,
            ab_b: None,
        }
    }
}

pub struct ControlPanel {
    pub open: bool,
    pub tab: usize,
    pub state: ControlPanelState,
    pub next_refresh: f64,
    /// True while the user is dragging a slider, so `refresh()` must not fight them.
    loading: bool,
}

impl Default for ControlPanel {
    fn default() -> Self {
        Self {
            open: false,
            tab: 0,
            state: ControlPanelState::default(),
            next_refresh: 0.0,
            loading: false,
        }
    }
}

impl ControlPanel {
    pub fn select_tab(&mut self, name: &str) {
        for (i, t) in ["Audio", "Video", "Subtitles", "Playback"]
            .iter()
            .enumerate()
        {
            if *t == name {
                self.tab = i;
            }
        }
    }

    /// Re-read every property from mpv, unless the user is mid-drag.
    pub fn refresh(&mut self, engine: &crate::engine::Engine, settings: &Settings) {
        if self.loading {
            return;
        }
        let g = |name: &str| engine.get_f64(name);
        let s = &mut self.state;
        s.audio_tracks = engine.audio_tracks();
        s.audio = engine.audio_track_state();
        s.tracks = engine.file_track_state();
        s.eq = settings.eq.clone();
        s.eq_enabled = settings.eq_enabled;
        s.audio_delay = g("audio-delay");
        s.brightness = g("brightness");
        s.contrast = g("contrast");
        s.saturation = g("saturation");
        s.hue = g("hue");
        s.gamma = g("gamma");
        s.sub_delay = g("sub-delay");
        s.sub_scale = g("sub-scale");
        s.sub_pos = g("sub-pos");
        s.sub_visible = engine
            .get("sub-visibility")
            .map(|v| v.as_bool())
            .unwrap_or(true);
        s.speed = g("speed");
        s.ab_a = engine.get("ab-loop-a").and_then(|v| v.as_f64());
        s.ab_b = engine.get("ab-loop-b").and_then(|v| v.as_f64());
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        actions: &mut Vec<Action>,
    ) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        Window::new("Control Panel")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .default_width(470.0)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(450.0);
                heading(ui, "Control Panel");
                tabs(
                    ui,
                    &["Audio", "Video", "Subtitles", "Playback"],
                    &mut self.tab,
                );
                ui.separator();
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("cp-scroll")
                    .auto_shrink([false, true])
                    .max_height(280.0)
                    .show(ui, |ui| match self.tab {
                        0 => self.audio_tab(ui, settings, actions),
                        1 => self.video_tab(ui, actions),
                        2 => self.subtitle_tab(ui, actions),
                        _ => self.playback_tab(ui, actions),
                    });
                ui.add_space(10.0);
                if button(ui, "control-panel-close", "Close") {
                    self.open = false;
                }
            });
        self.open = self.open && open;
    }

    fn audio_tab(&mut self, ui: &mut egui::Ui, settings: &mut Settings, actions: &mut Vec<Action>) {
        let mut track_items = vec![
            ("Auto".to_string(), TrackSelection::Auto),
            ("Off".to_string(), TrackSelection::None),
        ];
        track_items.extend(self.state.audio_tracks.iter().map(|track| {
            let mut label = track.id.to_string();
            if !track.title.is_empty() {
                label.push_str(". ");
                label.push_str(&track.title);
            }
            if !track.language.is_empty() {
                label.push_str(" [");
                label.push_str(&track.language);
                label.push(']');
            }
            (label, TrackSelection::Id(track.id))
        }));
        row(ui, "Audio track", |ui| {
            if let Some(selection) = combo(
                ui,
                "audio-track",
                &track_items,
                &self.state.audio.primary,
                190.0,
            ) {
                let action = match selection {
                    TrackSelection::Auto => Action::SetTrack(TrackKind::Audio, TrackSel::Auto),
                    TrackSelection::None => Action::SetTrack(TrackKind::Audio, TrackSel::No),
                    TrackSelection::Id(id) => Action::SetTrack(TrackKind::Audio, TrackSel::Id(id)),
                };
                self.state.audio.primary = selection;
                actions.push(action);
            }
        });
        row(ui, "Audio volume (%)", |ui| {
            let mut volume = self.state.audio.primary_volume;
            if spin(ui, "audio-volume", &mut volume, 1.0, 0.0, 100.0, "%") {
                self.state.audio.primary_volume = volume;
                actions.push(Action::SetVolume(volume.round() as i64));
            }
        });
        let mut enabled = self.state.eq_enabled;
        let mut eq = self.state.eq.clone();
        let mut eq_changed = false;

        ui.horizontal(|ui| {
            if check(ui, "eq-enable", "Enable equalizer", &mut enabled) {
                eq_changed = true;
            }
            ui.add_space(8.0);
            label(ui, "Preset...");
            let items: Vec<(String, String)> = EQ_PRESETS
                .iter()
                .map(|(name, _)| (name.to_string(), name.to_string()))
                .collect();
            let none = String::new();
            if let Some(name) = combo(ui, "eq-preset", &items, &none, 110.0) {
                if let Some((_, values)) = EQ_PRESETS.iter().find(|(n, _)| *n == name.as_str()) {
                    eq = values.to_vec();
                    enabled = true;
                    eq_changed = true;
                }
            }
        });
        ui.add_space(6.0);

        // Ten vertical band sliders, one per EQ_FREQS entry.
        let band_w = 36.0;
        let total_w = band_w * EQ_FREQS.len() as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(total_w, 150.0), Sense::hover());
        let p = ui.painter_at(rect);
        for (i, freq) in EQ_FREQS.iter().enumerate() {
            let x0 = rect.left() + band_w * i as f32;
            let track = Rect::from_min_size(
                pos2(x0 + band_w * 0.5 - 1.0, rect.top() + 8.0),
                vec2(2.0, 110.0),
            );
            p.rect_filled(track, 1.0, active_colors().base);
            let norm = (((eq[i] + 12.0) / 24.0).clamp(0.0, 1.0)) as f32;
            let fill_h = 110.0 * norm;
            let fill = Rect::from_min_size(
                pos2(track.left(), track.bottom() - fill_h),
                vec2(2.0, fill_h),
            );
            p.rect_filled(fill, 1.0, active_colors().accent);
            let ky = track.center().y - (norm - 0.5) * 110.0;
            let knob = Rect::from_center_size(pos2(track.center().x, ky), vec2(18.0, 8.0));
            p.rect_filled(knob, 2.0, active_colors().knob);
            let name = if *freq >= 1000.0 {
                format!("{}k", freq / 1000.0)
            } else {
                format!("{}", *freq as i64)
            };
            p.text(
                pos2(x0 + band_w * 0.5, rect.bottom() - 6.0),
                Align2::CENTER_CENTER,
                name,
                font(11.0, false),
                active_colors().text_9a,
            );
            // Drag the knob to set the band.
            let hit = Rect::from_min_size(pos2(x0, rect.top()), vec2(band_w, 130.0));
            let resp = ui.interact(hit, Id::new(("eq-band", i)), Sense::click_and_drag());
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let t = (1.0 - (pos.y - track.top()) / 110.0).clamp(0.0, 1.0);
                    eq[i] = (t as f64 * 24.0 - 12.0).round();
                    eq_changed = true;
                    self.loading = true;
                }
            } else if resp.drag_stopped() {
                self.loading = false;
            }
        }
        ui.add_space(12.0);

        let mut normalize = self.state.normalize;
        ui.horizontal(|ui| {
            if check(ui, "normalize", "Normalize volume", &mut normalize) {
                actions.push(Action::SetNormalize(normalize));
                self.state.normalize = normalize;
            }
            ui.add_space(8.0);
            label(ui, "Audio sync (sec):");
            let mut d = self.state.audio_delay;
            if spin(ui, "audio-delay", &mut d, 0.05, -60.0, 60.0, "") {
                actions.push(Action::AudioDelay(Some(d)));
                self.state.audio_delay = d;
            }
        });

        if eq_changed {
            self.state.eq = eq.clone();
            self.state.eq_enabled = enabled;
            settings.eq = eq.clone();
            settings.eq_enabled = enabled;
            actions.push(Action::SetEqualizer(eq, enabled));
        }
    }

    fn video_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        for (label_text, prop, field) in [
            ("Brightness", "brightness", 0usize),
            ("Contrast", "contrast", 1),
            ("Saturation", "saturation", 2),
            ("Hue", "hue", 3),
            ("Gamma", "gamma", 4),
        ] {
            let mut v = [
                self.state.brightness,
                self.state.contrast,
                self.state.saturation,
                self.state.hue,
                self.state.gamma,
            ][field];
            row(ui, label_text, |ui| {
                let r = ui.add(
                    egui::Slider::new(&mut v, -100.0..=100.0)
                        .fixed_decimals(0)
                        .show_value(false),
                );
                if r.dragged() {
                    self.loading = true;
                }
                if r.drag_stopped() {
                    self.loading = false;
                }
                if r.changed() {
                    actions.push(Action::ImageAdjust(prop, v as i32));
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{v:.0}"))
                        .size(12.0)
                        .color(active_colors().text_e4)
                        .monospace(),
                );
            });
            match field {
                0 => self.state.brightness = v,
                1 => self.state.contrast = v,
                2 => self.state.saturation = v,
                3 => self.state.hue = v,
                _ => self.state.gamma = v,
            }
        }
        ui.add_space(8.0);
        if button(ui, "img-reset", "Reset") {
            actions.push(Action::ImageReset);
            self.state.brightness = 0.0;
            self.state.contrast = 0.0;
            self.state.saturation = 0.0;
            self.state.hue = 0.0;
            self.state.gamma = 0.0;
        }
    }

    fn subtitle_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        row(ui, "Subtitle sync (sec)", |ui| {
            let mut v = self.state.sub_delay;
            if spin(ui, "sub-delay", &mut v, 0.5, -600.0, 600.0, "") {
                actions.push(Action::SubDelay(Some(v)));
                self.state.sub_delay = v;
            }
        });
        row(ui, "Size", |ui| {
            let mut v = self.state.sub_scale * 100.0;
            let r = ui.add(
                egui::Slider::new(&mut v, 30.0..=300.0)
                    .fixed_decimals(0)
                    .show_value(false),
            );
            if r.changed() {
                actions.push(Action::SubScale(v / 100.0));
                self.state.sub_scale = v / 100.0;
            }
        });
        row(ui, "Vertical position", |ui| {
            let mut v = self.state.sub_pos;
            let r = ui.add(
                egui::Slider::new(&mut v, 0.0..=150.0)
                    .fixed_decimals(0)
                    .show_value(false),
            );
            if r.changed() {
                actions.push(Action::SetSubPos(v));
                self.state.sub_pos = v;
            }
        });
        ui.add_space(4.0);
        let mut visible = self.state.sub_visible;
        if check(ui, "sub-vis", "Show subtitles", &mut visible) {
            actions.push(Action::ToggleSubVisibility);
            self.state.sub_visible = visible;
        }
        ui.add_space(8.0);
        if button(ui, "sub-reset", "Reset") {
            actions.push(Action::SubReset);
        }
    }

    fn playback_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        row(ui, "Speed", |ui| {
            let mut v = self.state.speed * 100.0;
            let r = ui.add(
                egui::Slider::new(&mut v, 20.0..=400.0)
                    .fixed_decimals(0)
                    .show_value(false),
            );
            if r.changed() {
                actions.push(Action::SetSpeed(v / 100.0));
                self.state.speed = v / 100.0;
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("{:.2}x", self.state.speed))
                    .size(12.0)
                    .color(active_colors().text_e4),
            );
        });
        ui.add_space(4.0);
        if button(ui, "normal-speed", "Normal Speed") {
            actions.push(Action::SetSpeed(1.0));
            self.state.speed = 1.0;
        }
        ui.add_space(8.0);
        label(ui, "A-B repeat");
        ui.horizontal(|ui| {
            if button(ui, "ab-a", "Set A") {
                actions.push(Action::AbSet('a'));
            }
            ui.add_space(4.0);
            if button(ui, "ab-b", "Set B") {
                actions.push(Action::AbSet('b'));
            }
            ui.add_space(4.0);
            if button(ui, "ab-clear", "Clear") {
                actions.push(Action::AbClear);
            }
        });
        ui.add_space(4.0);
        let a = self
            .state
            .ab_a
            .map(crate::util::fmt_time)
            .unwrap_or_else(|| "--:--:--".into());
        let b = self
            .state
            .ab_b
            .map(crate::util::fmt_time)
            .unwrap_or_else(|| "--:--:--".into());
        ui.label(
            RichText::new(format!("A: {a}    B: {b}"))
                .size(12.0)
                .color(active_colors().text_a8)
                .monospace(),
        );
    }
}

// -- Preferences -----------------------------------------------------------------------------------

pub struct PreferencesDialog {
    pub open: bool,
    pub tab: usize,
    pub temp: Settings,
    orig_hwdec: bool,
    /// Set when the user presses OK or Apply; the player swaps it in and re-applies.
    pub result: Option<(Settings, bool)>,
}

impl Default for PreferencesDialog {
    fn default() -> Self {
        Self {
            open: false,
            tab: 0,
            temp: Settings::default(),
            orig_hwdec: false,
            result: None,
        }
    }
}

impl PreferencesDialog {
    pub fn open(&mut self, current: &Settings) {
        self.open = true;
        self.temp = current.clone();
        self.orig_hwdec = current.hwdec;
        self.result = None;
    }

    pub fn select_tab(&mut self, name: &str) {
        for (i, t) in ["General", "Playback", "Subtitles", "Audio"]
            .iter()
            .enumerate()
        {
            if *t == name {
                self.tab = i;
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut close = false;
        Window::new("Preferences")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .default_width(560.0)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(540.0);
                heading(ui, "Preferences");
                tabs(
                    ui,
                    &["General", "Playback", "Subtitles", "Audio"],
                    &mut self.tab,
                );
                ui.separator();
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("pref-scroll")
                    .auto_shrink([false, true])
                    .max_height(330.0)
                    .show(ui, |ui| match self.tab {
                        0 => self.tab_general(ui),
                        1 => self.tab_playback(ui),
                        2 => self.tab_subtitles(ui),
                        _ => self.tab_audio(ui),
                    });
                ui.add_space(10.0);
                let (ok, cancel, apply, close_button) = ok_cancel_apply(ui);
                if cancel {
                    self.open = false;
                }
                if ok {
                    close = true;
                }
                if ok || apply {
                    let hwdec_changed = self.temp.hwdec != self.orig_hwdec;
                    self.orig_hwdec = self.temp.hwdec;
                    self.result = Some((self.temp.clone(), hwdec_changed));
                    actions.push(Action::ApplySettings { hwdec_changed });
                }
                if close_button {
                    self.open = false;
                    close = true;
                }
            });
        if close {
            self.open = false;
        } else {
            self.open = self.open && open;
        }
    }

    fn tab_general(&mut self, ui: &mut egui::Ui) {
        let t = &mut self.temp;
        let mut autofit = t.autofit;
        if check(
            ui,
            "autofit",
            "Resize the window to the video when a file opens",
            &mut autofit,
        ) {
            t.autofit = autofit;
        }
        row(ui, "Largest window size (of the screen)", |ui| {
            let mut pct = t.autofit_max_pct as f64;
            if spin(ui, "autofit-pct", &mut pct, 1.0, 30.0, 100.0, "%") {
                t.autofit_max_pct = pct as i64;
            }
        });
        let mut resume = t.resume;
        if check(
            ui,
            "resume",
            "Remember the playback position of each file",
            &mut resume,
        ) {
            t.resume = resume;
        }
        let mut ont = t.on_top.clone();
        row(ui, "Always on top", |ui| {
            let items: Vec<(String, String)> = [
                ("Never", "never"),
                ("Always", "always"),
                ("While playing", "playing"),
            ]
            .iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect();
            if let Some(v) = combo(ui, "on-top", &items, &ont, 130.0) {
                ont = v.clone();
                t.on_top = v;
            }
        });
        let mut show_remaining = t.show_remaining;
        if check(
            ui,
            "show-remaining",
            "Show remaining time instead of total",
            &mut show_remaining,
        ) {
            t.show_remaining = show_remaining;
        }
        let mut hdr = t.hdr_tone_mapping;
        if check(ui, "hdr-tone-mapping", "Convert HDR video to SDR", &mut hdr) {
            t.hdr_tone_mapping = hdr;
        }
        row(ui, "HDR tone mapping", |ui| {
            let items: Vec<(String, String)> = ["bt.2390", "reinhard", "mobius", "hable"]
                .into_iter()
                .map(|mode| (mode.to_string(), mode.to_string()))
                .collect();
            if let Some(mode) = combo(
                ui,
                "hdr-tone-mapping-mode",
                &items,
                &t.hdr_tone_mapping_mode,
                130.0,
            ) {
                t.hdr_tone_mapping_mode = mode;
            }
        });
        row(ui, "Fullscreen display index", |ui| {
            let mut index = t.fullscreen_monitor as f64;
            if spin(ui, "fullscreen-monitor", &mut index, 1.0, 0.0, 8.0, "") {
                t.fullscreen_monitor = index as usize;
            }
        });
        row(ui, "Screenshot folder", |ui| {
            let mut dir = t.screenshot_dir.clone();
            if line_edit(ui, "shot-dir", &mut dir, 200.0, "~/Pictures/MacPlayer") {
                t.screenshot_dir = dir;
            }
            ui.add_space(4.0);
            if button(ui, "shot-browse", "Browse...") {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    t.screenshot_dir = p.to_string_lossy().to_string();
                }
            }
        });
    }

    fn tab_playback(&mut self, ui: &mut egui::Ui) {
        for (key, text) in [
            ("seek_small", "Seek step: Left / Right"),
            ("seek_medium", "Seek step: Ctrl + Left / Right"),
            ("seek_large", "Seek step: Shift + Left / Right"),
            ("seek_huge", "Seek step: Ctrl + Alt + Left / Right"),
        ] {
            row(ui, text, |ui| {
                let t = &mut self.temp;
                let mut v = match key {
                    "seek_small" => t.seek_small,
                    "seek_medium" => t.seek_medium,
                    "seek_large" => t.seek_large,
                    _ => t.seek_huge,
                };
                if spin(ui, key, &mut v, 1.0, 1.0, 3600.0, "sec") {
                    match key {
                        "seek_small" => t.seek_small = v,
                        "seek_medium" => t.seek_medium = v,
                        "seek_large" => t.seek_large = v,
                        _ => t.seek_huge = v,
                    }
                }
            });
        }
        row(ui, "Volume step", |ui| {
            let t = &mut self.temp;
            let mut v = t.volume_step;
            if spin(ui, "volume-step", &mut v, 1.0, 1.0, 20.0, "%") {
                t.volume_step = v;
            }
        });
        let t = &mut self.temp;
        let mut hwdec = t.hwdec;
        if check(
            ui,
            "hwdec",
            "Use hardware video decoding when available",
            &mut hwdec,
        ) {
            t.hwdec = hwdec;
        }
    }

    fn tab_subtitles(&mut self, ui: &mut egui::Ui) {
        let t = &mut self.temp;
        let mut use_default = t.sub_font.is_empty();
        row(ui, "Font", |ui| {
            let mut family = t.sub_font.clone();
            if line_edit(ui, "sub-font", &mut family, 200.0, "sans-serif") {
                t.sub_font = family;
            }
        });
        ui.add_space(2.0);
        if check(
            ui,
            "sub-font-default",
            "Use the default font",
            &mut use_default,
        ) {
            if use_default {
                t.sub_font.clear();
            }
        }
        row(ui, "Size", |ui| {
            let t = &mut self.temp;
            let mut v = t.sub_size as f64;
            if spin(ui, "sub-size", &mut v, 1.0, 10.0, 150.0, "") {
                t.sub_size = v as i64;
            }
        });
        let t = &mut self.temp;
        let mut bold = t.sub_bold;
        if check(ui, "sub-bold", "Bold", &mut bold) {
            t.sub_bold = bold;
        }
        row(ui, "Text encoding for non-Unicode files", |ui| {
            let t = &mut self.temp;
            let items: Vec<(String, String)> = ENCODINGS
                .iter()
                .map(|(l, c)| (l.to_string(), c.to_string()))
                .collect();
            let cur = t.sub_codepage.clone();
            if let Some(v) = combo(ui, "sub-codepage", &items, &cur, 170.0) {
                t.sub_codepage = v;
            }
        });
        row(ui, "Load subtitles automatically", |ui| {
            let t = &mut self.temp;
            let items: Vec<(String, String)> = [
                ("Same name as the video", "exact"),
                ("Names containing the video name", "fuzzy"),
                ("Every subtitle in the folder", "all"),
                ("Do not load automatically", "no"),
            ]
            .iter()
            .map(|(l, m)| (l.to_string(), m.to_string()))
            .collect();
            let cur = t.sub_auto.clone();
            if let Some(v) = combo(ui, "sub-auto", &items, &cur, 190.0) {
                t.sub_auto = v;
            }
        });
        row(ui, "Preferred subtitle languages", |ui| {
            let t = &mut self.temp;
            let mut v = t.preferred_sub_lang.clone();
            if line_edit(ui, "sub-lang", &mut v, 170.0, "e.g. fa,per,en") {
                t.preferred_sub_lang = v;
            }
        });
    }

    fn tab_audio(&mut self, ui: &mut egui::Ui) {
        row(ui, "Preferred audio languages", |ui| {
            let t = &mut self.temp;
            let mut v = t.preferred_audio_lang.clone();
            if line_edit(ui, "audio-lang", &mut v, 170.0, "e.g. en,fa") {
                t.preferred_audio_lang = v;
            }
        });
    }
}

// -- media info ------------------------------------------------------------------------------------

fn value_text(value: &Value) -> String {
    match value {
        Value::None => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Str(value) => value.clone(),
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(value_text).collect::<Vec<_>>().join(", ")
        ),
        Value::Map(values) => values
            .iter()
            .map(|(key, value)| format!("{key}: {}", value_text(value)))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// Build the "Media Information" text block from mpv properties.
pub fn media_info_text(engine: &crate::engine::Engine) -> String {
    let mut out = String::new();
    let s = |name: &str| engine.get(name).map(|v| value_text(&v)).unwrap_or_default();
    let path = engine
        .get("path")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let _ = path;
    out.push_str(&format!(
        "File: {}\n",
        engine
            .get("media-title")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    ));
    out.push_str(&format!(
        "Duration: {}\n",
        crate::util::fmt_time(engine.get_f64("duration"))
    ));
    out.push('\n');
    let video = engine.get("video-params");
    if !video.is_none() {
        out.push_str("Video:\n");
        for (key, label) in [
            ("codec", "Codec"),
            ("width", "Width"),
            ("height", "Height"),
            ("pixelformat", "Pixel format"),
            ("hw-pixelformat", "Hardware pixel format"),
            ("container-fps", "Container FPS"),
            ("estimated-vf-fps", "Estimated FPS"),
            ("color-space", "Color space"),
            ("color-range", "Color range"),
            ("hardware", "Hardware decoder"),
        ] {
            let v = video
                .as_ref()
                .map(|value| value.get(key))
                .unwrap_or(Value::None);
            if !v.is_none() {
                out.push_str(&format!("  {label}: {}\n", value_text(&v)));
            }
        }
        out.push('\n');
    }
    let audio = engine.get("audio-params");
    if !audio.is_none() {
        out.push_str("Audio:\n");
        for (key, label) in [
            ("codec", "Codec"),
            ("samplerate", "Sample rate"),
            ("channels", "Channels"),
        ] {
            let v = audio
                .as_ref()
                .map(|value| value.get(key))
                .unwrap_or(Value::None);
            if !v.is_none() {
                out.push_str(&format!("  {label}: {}\n", value_text(&v)));
            }
        }
        out.push('\n');
    }
    let sub = engine.get("current-tracks/sub");
    if !sub.is_none() {
        out.push_str("Subtitle:\n");
        for (key, label) in [("codec", "Codec"), ("lang", "Language"), ("title", "Title")] {
            let v = sub
                .as_ref()
                .map(|value| value.get(key))
                .unwrap_or(Value::None);
            if !v.is_none() {
                out.push_str(&format!("  {label}: {}\n", value_text(&v)));
            }
        }
        out.push('\n');
    }
    out.push_str("Pixel format:\n");
    out.push_str(&format!("  {}\n", s("video-format")));
    out
}
