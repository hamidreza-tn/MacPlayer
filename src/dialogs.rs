//! Preferences / Control Panel / GoTo / Text / URL dialogs.
//!
//! Each dialog is an egui `Window` on a custom frame, styled with the MacPlayer palette.

use crate::engine::{AudioTrackInfo, AudioTrackState, FileTrackState, TrackSelection};
use crate::i18n;
use crate::menus::Action;
use crate::menus::{TrackKind, TrackSel};
use crate::settings::Settings;
use crate::skin;
use crate::skin::*;
use crate::util::{fmt_time, parse_time};
use egui::{
    Align, Align2, Color32, Frame, Id, Layout, Rect, RichText, Sense, Stroke, TextEdit, Window,
    pos2, vec2,
};

fn active_colors() -> skin::ThemeColors {
    skin::active_colors()
}

// -- shared chrome ---------------------------------------------------------------------------------

fn dlg_frame() -> Frame {
    Frame::new()
        .fill(active_colors().dialog_bg)
        .stroke(Stroke::new(1.0, active_colors().dialog_line))
        .corner_radius(10.0)
        .outer_margin(egui::Margin::same(6))
        .inner_margin(egui::Margin::same(12))
}

fn dialog_header(ui: &mut egui::Ui, title: &str) -> bool {
    let colors = active_colors();
    let mut close = false;
    ui.horizontal(|ui| {
        // Accent dot + title — the same signature as the playlist drawer header.
        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 3.0, colors.accent);
        ui.add_space(3.0);
        ui.label(
            RichText::new(title)
                .size(14.5)
                .color(colors.text_bright)
                .strong(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
            let resp = resp.on_hover_text(i18n::tr("Close"));
            let p = ui.painter_at(r);
            let hover = resp.hovered();
            if hover {
                p.rect_filled(r, 6.0, colors.close_hover);
            }
            skin::draw_icon(
                &p,
                "close",
                r.center().x.round(),
                r.center().y.round(),
                if hover {
                    Color32::WHITE
                } else {
                    colors.text_9a
                },
                colors.dialog_bg,
            );
            if resp.clicked() {
                close = true;
            }
        });
    });
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(8.0);
    close
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
        add(ui)
    })
    .inner
}

/// Rounded checkbox + label; the whole box+text area toggles, not just the box.
fn check(ui: &mut egui::Ui, id: &str, text: &str, value: &mut bool) -> bool {
    let colors = active_colors();
    let box_size = 16.0;
    let gap = 7.0;
    let text_w = skin::text_width(ui.painter(), text, &skin::font(12.0, false));
    let total_w = box_size + gap + text_w;
    let (total, _) = ui.allocate_exact_size(vec2(total_w, 20.0), Sense::click());
    let resp = ui.interact(total, Id::new(("dialog-check", id)), Sense::click());
    let p = ui.painter_at(total);
    let box_rect = Rect::from_min_size(
        pos2(total.left(), total.center().y - box_size / 2.0),
        vec2(box_size, box_size),
    );
    let hover = resp.hovered();
    if *value {
        p.rect_filled(box_rect, 4.0, colors.accent);
    } else {
        p.rect_filled(
            box_rect,
            4.0,
            if hover {
                colors.button_hover
            } else {
                colors.base
            },
        );
        p.rect_stroke(
            box_rect,
            4.0,
            Stroke::new(1.0, if hover { colors.text_dim } else { colors.line }),
            egui::StrokeKind::Inside,
        );
    }
    if *value {
        for i in 0..4 {
            p.rect_filled(
                Rect::from_min_size(
                    pos2(
                        box_rect.left() + 3.0 + i as f32,
                        box_rect.top() + 7.0 + i as f32,
                    ),
                    vec2(2.0, 1.0),
                ),
                0.0,
                colors.bg,
            );
            if i > 0 {
                p.rect_filled(
                    Rect::from_min_size(
                        pos2(
                            box_rect.left() + 3.0 + i as f32,
                            box_rect.top() + 11.0 - i as f32,
                        ),
                        vec2(2.0, 1.0),
                    ),
                    0.0,
                    colors.bg,
                );
            }
        }
    }
    p.text(
        pos2(box_rect.right() + gap, total.center().y),
        Align2::LEFT_CENTER,
        text,
        font(12.0, false),
        colors.text_e4,
    );
    let clicked = resp.clicked();
    if clicked {
        *value = !*value;
    }
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
    // Fractional steps must stay visible: 0.05 steps rendering as "0" makes
    // the arrows look dead.
    let decimals = if step < 0.1 {
        2
    } else if step < 1.0 {
        1
    } else {
        0
    };
    let mut text = ui
        .ctx()
        .data(|data| data.get_temp::<String>(text_id))
        .unwrap_or_else(|| format!("{value:.decimals$}"));
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
        if let Ok(v) = text.trim().parse::<f64>()
            && (v - *value).abs() > f64::EPSILON
        {
            *value = v.clamp(lo, hi);
            changed = true;
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
        // Drop any in-progress typed text so the field shows the arrowed value.
        ui.ctx().data_mut(|data| data.remove::<String>(text_id));
    }
    if arrow(ui, false) {
        *value = (*value - step).max(lo);
        changed = true;
        ui.ctx().data_mut(|data| data.remove::<String>(text_id));
    }
    if !suffix.is_empty() {
        ui.add_space(3.0);
        ui.label(
            RichText::new(suffix)
                .size(12.0)
                .color(active_colors().text_a8),
        );
    }
    changed
}

fn line_edit(ui: &mut egui::Ui, id: &str, value: &mut String, width: f32, hint: &str) -> bool {
    let mut text = value.clone();
    let r = ui.add(
        TextEdit::singleline(&mut text)
            .desired_width(width)
            .font(egui::TextStyle::Monospace)
            .margin(vec2(3.0, 2.0))
            .id_salt(Id::new(("dialog-line-edit", id)))
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
    // Measure the real text so byte-heavy scripts (Persian) neither bloat nor
    // clip the button.
    let text_w = skin::text_width(ui.painter(), text, &skin::font(12.0, false));
    ui.add_sized(
        vec2(text_w.max(34.0) + 26.0, 24.0),
        egui::Button::new(
            RichText::new(text)
                .size(12.0)
                .color(active_colors().text_e4),
        ),
    )
    .clicked()
}

/// Accent-filled primary action (OK / Apply / Resume): dark text on the accent color.
fn primary_button(ui: &mut egui::Ui, text: &str, width: f32, height: f32) -> bool {
    let colors = active_colors();
    let text_w = skin::text_width(ui.painter(), text, &skin::font(12.0, false));
    ui.add_sized(
        vec2(width.max(text_w + 26.0), height),
        egui::Button::new(RichText::new(text).size(12.0).color(colors.bg).strong())
            .fill(colors.accent)
            .corner_radius(6.0),
    )
    .clicked()
}

/// OK / Cancel / Apply footer shared by the modal dialogs; each flag draws its button
/// and reports whether it was clicked. OK and Apply draw as accent primaries.
fn dialog_footer(ui: &mut egui::Ui, ok: bool, cancel: bool, apply: bool) -> (bool, bool, bool) {
    let mut ok_clicked = false;
    let mut cancel_clicked = false;
    let mut apply_clicked = false;
    let footer_button = |ui: &mut egui::Ui, text: &str, clicked: &mut bool| {
        if ui
            .add_sized(
                vec2(84.0, 26.0),
                egui::Button::new(
                    RichText::new(text)
                        .size(12.0)
                        .color(active_colors().text_e4),
                ),
            )
            .clicked()
        {
            *clicked = true;
        }
    };
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        if ok && primary_button(ui, i18n::tr("OK"), 84.0, 26.0) {
            ok_clicked = true;
        }
        if ok {
            ui.add_space(4.0);
        }
        if cancel {
            footer_button(ui, i18n::tr("Cancel"), &mut cancel_clicked);
            ui.add_space(4.0);
        }
        if apply && primary_button(ui, i18n::tr("Apply"), 84.0, 26.0) {
            apply_clicked = true;
        }
    });
    (ok_clicked, cancel_clicked, apply_clicked)
}

/// Segmented control over `names`; the selected segment gets a raised fill and an
/// accent outline. Used for dialog tab strips and small mode switches.
fn tabs(ui: &mut egui::Ui, scope: &str, names: &[&str], current: &mut usize) {
    let colors = active_colors();
    let item_h = 26.0;
    let pad_x = 12.0;
    let gap = 4.0;
    let widths: Vec<f32> = names
        .iter()
        .map(|name| {
            skin::text_width(ui.painter(), name, &skin::font(12.0, false)).max(24.0) + pad_x * 2.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + gap * names.len().saturating_sub(1) as f32 + pad_x;
    let (container, _) = ui.allocate_exact_size(vec2(total, item_h), Sense::hover());
    let p = ui.painter_at(container);
    p.rect_filled(container, 7.0, colors.tab_bg);
    let mut x = container.left() + pad_x / 2.0;
    for (i, name) in names.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(x, container.top() + 3.0),
            vec2(widths[i], item_h - 6.0),
        );
        let resp = ui.interact(rect, Id::new(("tab", scope, i)), Sense::click());
        let sel = i == *current;
        if sel {
            p.rect_filled(rect, 5.0, colors.tab_sel);
            p.rect_stroke(
                rect,
                5.0,
                Stroke::new(1.0, colors.accent.gamma_multiply(0.7)),
                egui::StrokeKind::Inside,
            );
        } else if resp.hovered() {
            p.rect_filled(rect, 5.0, colors.button_hover);
        }
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            *name,
            font(12.0, sel),
            if sel {
                colors.text_bright
            } else {
                colors.text_a8
            },
        );
        if resp.clicked() {
            *current = i;
        }
        x += widths[i] + gap;
    }
    ui.add_space(8.0);
}

// -- GoTo ------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum GoToMode {
    Time,
    Percent,
}

// -- Resume playback ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeChoice {
    Resume,
    StartOver,
}

#[derive(Default)]
pub struct ResumeDialog {
    pub open: bool,
    position: f64,
}

impl ResumeDialog {
    pub fn open(&mut self, position: f64) {
        self.position = position.max(0.0);
        self.open = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<ResumeChoice> {
        if !self.open {
            return None;
        }

        let colors = active_colors();
        let mut choice = None;
        Window::new(i18n::tr("Resume playback"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -24.0))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(i18n::tr("Continue watching?"))
                            .size(15.0)
                            .color(colors.text_bright)
                            .strong(),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(i18n::tr("You previously watched this file up to"))
                            .size(12.0)
                            .color(colors.text_a8),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(fmt_time(self.position))
                            .size(24.0)
                            .color(colors.accent)
                            .monospace(),
                    );
                    ui.add_space(12.0);
                });
                ui.horizontal(|ui| {
                    if primary_button(
                        ui,
                        &format!("{} {}", i18n::tr("Resume from"), fmt_time(self.position)),
                        156.0,
                        28.0,
                    ) {
                        choice = Some(ResumeChoice::Resume);
                    }
                    ui.add_space(6.0);
                    if ui
                        .add_sized(
                            vec2(156.0, 28.0),
                            egui::Button::new(
                                RichText::new(i18n::tr("Start from beginning"))
                                    .size(12.0)
                                    .color(colors.text_e4),
                            ),
                        )
                        .clicked()
                    {
                        choice = Some(ResumeChoice::StartOver);
                    }
                });
            });

        if choice.is_some() {
            self.open = false;
        }
        choice
    }
}

pub struct GoToDialog {
    pub open: bool,
    pub mode: GoToMode,
    pub input: String,
    pub error: Option<String>,
    focus_pending: bool,
}

impl Default for GoToDialog {
    fn default() -> Self {
        Self {
            open: false,
            mode: GoToMode::Time,
            input: String::new(),
            error: None,
            focus_pending: false,
        }
    }
}

impl GoToDialog {
    pub fn open(&mut self, mode: GoToMode) {
        self.open = true;
        self.mode = mode;
        self.input.clear();
        self.error = None;
        self.focus_pending = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, duration: f64, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut submit = false;
        Window::new(i18n::tr("Go to"))
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_min_width(250.0);
                heading(ui, i18n::tr("Go to"));
                let mut mode_tab = match self.mode {
                    GoToMode::Time => 0,
                    GoToMode::Percent => 1,
                };
                tabs(
                    ui,
                    "goto",
                    &[i18n::tr("Time"), i18n::tr("Percent")],
                    &mut mode_tab,
                );
                self.mode = match mode_tab {
                    1 => GoToMode::Percent,
                    _ => GoToMode::Time,
                };
                ui.add_space(2.0);
                let mut input = self.input.clone();
                let r = ui.add(
                    TextEdit::singleline(&mut input)
                        .desired_width(230.0)
                        .font(egui::TextStyle::Monospace)
                        .hint_text(
                            RichText::new(i18n::tr("HH:MM:SS or 01:30"))
                                .size(12.0)
                                .color(active_colors().text_9a),
                        ),
                );
                self.input = input;
                if std::mem::take(&mut self.focus_pending) {
                    r.request_focus();
                }
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
                    if primary_button(ui, i18n::tr("OK"), 84.0, 26.0) {
                        submit = true;
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            vec2(80.0, 26.0),
                            egui::Button::new(
                                RichText::new(i18n::tr("Cancel"))
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
                    Ok(_) | Err(_) => self.error = Some(i18n::tr("Please enter a number").into()),
                },
                GoToMode::Time => match parse_time(&s, Some(duration)) {
                    Some(secs) if secs.is_finite() => {
                        actions.push(Action::SeekTo {
                            seconds: Some(secs.max(0.0)),
                            percent: None,
                        });
                        self.open = false;
                    }
                    _ => self.error = Some(i18n::tr("Please enter a valid time").into()),
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
                    if primary_button(ui, i18n::tr("Copy"), 90.0, 24.0) {
                        ui.copy_text(self.body.clone());
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            vec2(90.0, 24.0),
                            egui::Button::new(
                                RichText::new(i18n::tr("Close"))
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
    /// `false` replaces the playlist on submit, `true` appends to it.
    pub append: bool,
    focus_pending: bool,
}

impl Default for UrlDialog {
    fn default() -> Self {
        Self {
            open: false,
            url: String::new(),
            loop_input: "0".into(),
            error: None,
            append: false,
            focus_pending: false,
        }
    }
}

impl UrlDialog {
    pub fn open(&mut self, append: bool) {
        self.open = true;
        self.error = None;
        self.focus_pending = true;
        self.append = append;
    }

    pub fn show(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut submit = false;
        Window::new(i18n::tr("Play URL"))
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -30.0))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                heading(ui, i18n::tr("Enter URL:"));
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
                if std::mem::take(&mut self.focus_pending) {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    label(ui, i18n::tr("Loop count (0 = no loop):"));
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
                    if primary_button(ui, i18n::tr("OK"), 84.0, 26.0) {
                        submit = true;
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized(
                            vec2(84.0, 26.0),
                            egui::Button::new(
                                RichText::new(i18n::tr("Cancel"))
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
                self.error = Some(i18n::tr("Please enter a valid URL").into());
            } else {
                match self.loop_input.trim().parse::<i64>() {
                    Ok(loop_s) if (0..=86_400).contains(&loop_s) => {
                        self.error = None;
                        actions.push(Action::PlayUrl {
                            url: u,
                            loop_s,
                            append: self.append,
                        });
                        self.open = false;
                    }
                    _ => {
                        self.error = Some(
                            i18n::tr("Loop count must be a whole number from 0 to 86400").into(),
                        );
                    }
                }
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

    /// Open the panel fresh: clear any drag state left over from a previous session,
    /// so the periodic refresh can never stay disabled by a stale `loading` flag.
    pub fn open_panel(&mut self) {
        self.open = true;
        self.loading = false;
        self.next_refresh = 0.0;
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
        let mut close = false;
        Window::new("Control Panel")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .frame(dlg_frame())
            .default_width(470.0)
            .default_height(500.0)
            .max_height((ctx.content_rect().height() - 32.0).max(240.0))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(450.0);
                if dialog_header(ui, i18n::tr("Control panel")) {
                    close = true;
                }
                tabs(
                    ui,
                    "control-panel",
                    &[
                        i18n::tr("Audio"),
                        i18n::tr("Video"),
                        i18n::tr("Subtitles"),
                        i18n::tr("Playback"),
                    ],
                    &mut self.tab,
                );
                ui.separator();
                ui.add_space(4.0);
                let scroll_height = ui.available_height().max(260.0);
                egui::ScrollArea::vertical()
                    .id_salt("cp-scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_height)
                    .show(ui, |ui| match self.tab {
                        0 => self.audio_tab(ui, settings, actions),
                        1 => self.video_tab(ui, actions),
                        2 => self.subtitle_tab(ui, actions),
                        _ => self.playback_tab(ui, actions),
                    });
                ui.add_space(10.0);
                if button(ui, "control-panel-close", i18n::tr("Close")) {
                    self.open = false;
                }
            });
        if close {
            self.open = false;
        }
        self.open = self.open && open;
    }

    fn audio_tab(&mut self, ui: &mut egui::Ui, settings: &mut Settings, actions: &mut Vec<Action>) {
        let mut track_items = vec![
            (i18n::tr("Auto").to_string(), TrackSelection::Auto),
            (i18n::tr("Off").to_string(), TrackSelection::None),
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
        row(ui, i18n::tr("Audio track"), |ui| {
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
        row(ui, i18n::tr("Audio volume (%)"), |ui| {
            let mut volume = self.state.audio.primary_volume;
            let max_volume = settings.volume_max.max(100) as f64;
            if spin(ui, "audio-volume", &mut volume, 1.0, 0.0, max_volume, "%") {
                self.state.audio.primary_volume = volume;
                actions.push(Action::SetVolume(volume.round() as i64));
            }
        });
        let mut enabled = self.state.eq_enabled;
        let mut eq = self.state.eq.clone();
        let mut eq_changed = false;

        ui.horizontal(|ui| {
            if check(ui, "eq-enable", i18n::tr("Enable equalizer"), &mut enabled) {
                eq_changed = true;
            }
            ui.add_space(8.0);
            label(ui, i18n::tr("Preset..."));
            let items: Vec<(String, String)> = EQ_PRESETS
                .iter()
                .map(|(name, _)| ((*name).to_string(), (*name).to_string()))
                .collect();
            let none = String::new();
            if let Some(name) = combo(ui, "eq-preset", &items, &none, 110.0)
                && let Some((_, values)) = EQ_PRESETS.iter().find(|(n, _)| *n == name.as_str())
            {
                eq = values.to_vec();
                enabled = true;
                eq_changed = true;
            }
        });
        ui.add_space(6.0);

        // Ten vertical band sliders, one per EQ_FREQS entry.
        let band_w = 36.0;
        let total_w = band_w * EQ_FREQS.len() as f32;
        let (rect, _) = ui.allocate_exact_size(vec2(total_w, 150.0), Sense::hover());
        let p = ui.painter_at(rect);
        // 0 dB reference line across all bands.
        let zero_y = rect.top() + 8.0 + 55.0;
        p.line_segment(
            [
                pos2(rect.left() + 2.0, zero_y),
                pos2(rect.right() - 2.0, zero_y),
            ],
            Stroke::new(1.0, active_colors().line),
        );
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
            if check(
                ui,
                "normalize",
                i18n::tr("Normalize volume"),
                &mut normalize,
            ) {
                actions.push(Action::SetNormalize(normalize));
                self.state.normalize = normalize;
            }
            ui.add_space(8.0);
            label(ui, i18n::tr("Audio sync (sec):"));
            let mut d = self.state.audio_delay;
            if spin(ui, "audio-delay", &mut d, 0.05, -60.0, 60.0, "") {
                actions.push(Action::SetAudioDelay(d));
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
            (i18n::tr("Brightness"), "brightness", 0usize),
            (i18n::tr("Contrast"), "contrast", 1),
            (i18n::tr("Saturation"), "saturation", 2),
            (i18n::tr("Hue"), "hue", 3),
            (i18n::tr("Gamma"), "gamma", 4),
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
                    actions.push(Action::SetImageAdjust(prop, v));
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
        if button(ui, "img-reset", i18n::tr("Reset")) {
            actions.push(Action::ImageReset);
            self.state.brightness = 0.0;
            self.state.contrast = 0.0;
            self.state.saturation = 0.0;
            self.state.hue = 0.0;
            self.state.gamma = 0.0;
        }
    }

    fn subtitle_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        row(ui, i18n::tr("Subtitle sync (sec)"), |ui| {
            let mut v = self.state.sub_delay;
            if spin(ui, "sub-delay", &mut v, 0.5, -600.0, 600.0, "") {
                actions.push(Action::SetSubDelay(v));
                self.state.sub_delay = v;
            }
        });
        row(ui, i18n::tr("Size"), |ui| {
            let mut v = self.state.sub_scale * 100.0;
            let r = ui.add(
                egui::Slider::new(&mut v, 30.0..=300.0)
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
                actions.push(Action::SetSubScale(v / 100.0));
                self.state.sub_scale = v / 100.0;
            }
        });
        row(ui, i18n::tr("Vertical position"), |ui| {
            let mut v = self.state.sub_pos;
            let r = ui.add(
                egui::Slider::new(&mut v, 0.0..=100.0)
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
                actions.push(Action::SetSubPos(v));
                self.state.sub_pos = v;
            }
        });
        ui.add_space(4.0);
        let mut visible = self.state.sub_visible;
        if check(ui, "sub-vis", i18n::tr("Show subtitles"), &mut visible) {
            actions.push(Action::ToggleSubVisibility);
            self.state.sub_visible = visible;
        }
        ui.add_space(8.0);
        if button(ui, "sub-reset", i18n::tr("Reset")) {
            actions.push(Action::SubReset);
        }
    }

    fn playback_tab(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        row(ui, i18n::tr("Speed"), |ui| {
            let mut v = self.state.speed * 100.0;
            let r = ui.add(
                egui::Slider::new(&mut v, 20.0..=400.0)
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
        if button(ui, "normal-speed", i18n::tr("Normal Speed")) {
            actions.push(Action::SetSpeed(1.0));
            self.state.speed = 1.0;
        }
        ui.add_space(8.0);
        label(ui, i18n::tr("A-B repeat"));
        ui.horizontal(|ui| {
            if button(ui, "ab-a", i18n::tr("Set A")) {
                actions.push(Action::AbSet('a'));
            }
            ui.add_space(4.0);
            if button(ui, "ab-b", i18n::tr("Set B")) {
                actions.push(Action::AbSet('b'));
            }
            ui.add_space(4.0);
            if button(ui, "ab-clear", i18n::tr("Clear")) {
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

#[derive(Default)]
pub struct PreferencesDialog {
    pub open: bool,
    pub tab: usize,
    pub temp: Settings,
    orig_hwdec: bool,
    /// Set when the user presses OK or Apply; the player swaps it in and re-applies.
    pub result: Option<(Settings, bool)>,
}

impl PreferencesDialog {
    pub fn open(&mut self, current: &Settings) {
        self.open = true;
        self.temp = current.clone();
        self.orig_hwdec = current.hwdec;
        self.result = None;
    }

    pub fn select_tab(&mut self, name: &str) {
        for (i, t) in ["General", "Playback", "Subtitles", "Audio", "Network"]
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
            .resizable(true)
            .frame(dlg_frame())
            .default_height(520.0)
            .max_height((ctx.content_rect().height() - 32.0).max(240.0))
            .default_width(560.0)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(540.0);
                if dialog_header(ui, i18n::tr("Preferences")) {
                    close = true;
                }
                tabs(
                    ui,
                    "preferences",
                    &[
                        i18n::tr("General"),
                        i18n::tr("Playback"),
                        i18n::tr("Subtitles"),
                        i18n::tr("Audio"),
                        i18n::tr("Network"),
                    ],
                    &mut self.tab,
                );
                ui.separator();
                ui.add_space(4.0);
                let scroll_height = ui.available_height().max(280.0);
                egui::ScrollArea::vertical()
                    .id_salt("pref-scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_height)
                    .show(ui, |ui| match self.tab {
                        0 => self.tab_general(ui),
                        1 => self.tab_playback(ui),
                        2 => self.tab_subtitles(ui),
                        3 => self.tab_audio(ui),
                        _ => self.tab_network(ui),
                    });
                ui.add_space(10.0);
                let (ok, cancel, apply) = dialog_footer(ui, true, true, true);
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
            i18n::tr("Resize the window to the video when a file opens"),
            &mut autofit,
        ) {
            t.autofit = autofit;
        }
        row(ui, i18n::tr("Largest window size (of the screen)"), |ui| {
            let mut pct = t.autofit_max_pct as f64;
            if spin(ui, "autofit-pct", &mut pct, 1.0, 30.0, 100.0, "%") {
                t.autofit_max_pct = pct as i64;
            }
        });
        let mut resume = t.resume;
        if check(
            ui,
            "resume",
            i18n::tr("Remember the playback position of each file"),
            &mut resume,
        ) {
            t.resume = resume;
        }
        let mut ont = t.on_top.clone();
        row(ui, i18n::tr("Always on top"), |ui| {
            let items: Vec<(String, String)> = [
                (i18n::tr("Never"), "never"),
                (i18n::tr("Always"), "always"),
                (i18n::tr("While playing"), "playing"),
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
            i18n::tr("Show the remaining time instead of the total"),
            &mut show_remaining,
        ) {
            t.show_remaining = show_remaining;
        }
        let mut hdr = t.hdr_tone_mapping;
        if check(
            ui,
            "hdr-tone-mapping",
            i18n::tr("Tone-map HDR video for standard displays"),
            &mut hdr,
        ) {
            t.hdr_tone_mapping = hdr;
        }
        row(ui, i18n::tr("HDR tone mapping"), |ui| {
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
        row(ui, i18n::tr("Fullscreen monitor"), |ui| {
            let mut index = t.fullscreen_monitor as f64;
            if spin(ui, "fullscreen-monitor", &mut index, 1.0, 0.0, 8.0, "") {
                t.fullscreen_monitor = index as usize;
            }
        });
        row(ui, i18n::tr("Screenshot folder"), |ui| {
            let mut dir = t.screenshot_dir.clone();
            if line_edit(ui, "shot-dir", &mut dir, 200.0, "~/Pictures/MacPlayer") {
                t.screenshot_dir = dir;
            }
            ui.add_space(4.0);
            if button(ui, "shot-browse", i18n::tr("Browse…"))
                && let Some(p) = rfd::FileDialog::new().pick_folder()
            {
                t.screenshot_dir = p.to_string_lossy().to_string();
            }
        });
        row(ui, i18n::tr("Theme"), |ui| {
            let items: Vec<(String, String)> =
                [(i18n::tr("Dark"), "dark"), (i18n::tr("Light"), "light")]
                    .iter()
                    .map(|(l, v)| (l.to_string(), v.to_string()))
                    .collect();
            let cur = t.theme.clone();
            if let Some(v) = combo(ui, "pref-theme", &items, &cur, 130.0) {
                t.theme = v;
            }
        });
        row(ui, i18n::tr("Accent color"), |ui| {
            let items: Vec<(String, String)> = [
                (i18n::tr("Gold"), "gold"),
                (i18n::tr("Blue"), "blue"),
                (i18n::tr("Green"), "green"),
                (i18n::tr("Rose"), "rose"),
                (i18n::tr("Graphite"), "graphite"),
            ]
            .iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect();
            let cur = t.accent.clone();
            if let Some(v) = combo(ui, "pref-accent", &items, &cur, 130.0) {
                t.accent = v;
            }
        });
        row(ui, i18n::tr("Language"), |ui| {
            let items: Vec<(String, String)> =
                [(i18n::tr("English"), "en"), (i18n::tr("Persian"), "fa")]
                    .iter()
                    .map(|(l, v)| (l.to_string(), v.to_string()))
                    .collect();
            let cur = t.language.clone();
            if let Some(v) = combo(ui, "pref-language", &items, &cur, 130.0) {
                t.language = v;
            }
        });
    }

    fn tab_playback(&mut self, ui: &mut egui::Ui) {
        for (key, text) in [
            ("seek_small", i18n::tr("Seek step: Left / Right")),
            ("seek_medium", i18n::tr("Seek step: Ctrl + Left / Right")),
            ("seek_large", i18n::tr("Seek step: Shift + Left / Right")),
            (
                "seek_huge",
                i18n::tr("Seek step: Ctrl + Alt + Left / Right"),
            ),
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
        row(ui, i18n::tr("Volume step"), |ui| {
            let t = &mut self.temp;
            let mut v = t.volume_step;
            if spin(ui, "volume-step", &mut v, 1.0, 1.0, 20.0, "%") {
                t.volume_step = v;
            }
        });
        row(ui, i18n::tr("Maximum volume (%)"), |ui| {
            let t = &mut self.temp;
            let mut v = t.volume_max as f64;
            if spin(ui, "volume-max", &mut v, 5.0, 100.0, 200.0, "%") {
                t.volume_max = v as i64;
            }
        });
        let t = &mut self.temp;
        let mut hwdec = t.hwdec;
        if check(
            ui,
            "hwdec",
            i18n::tr("Use hardware video decoding when available"),
            &mut hwdec,
        ) {
            t.hwdec = hwdec;
        }
    }

    fn tab_subtitles(&mut self, ui: &mut egui::Ui) {
        let t = &mut self.temp;
        let mut use_default = t.sub_font.is_empty();
        row(ui, i18n::tr("Font"), |ui| {
            let mut family = t.sub_font.clone();
            if line_edit(ui, "sub-font", &mut family, 200.0, "sans-serif") {
                t.sub_font = family;
            }
        });
        ui.add_space(2.0);
        if check(
            ui,
            "sub-font-default",
            i18n::tr("Use the default font"),
            &mut use_default,
        ) && use_default
        {
            t.sub_font.clear();
        }
        row(ui, i18n::tr("Size"), |ui| {
            let t = &mut self.temp;
            let mut v = t.sub_size as f64;
            if spin(ui, "sub-size", &mut v, 1.0, 10.0, 150.0, "") {
                t.sub_size = v as i64;
            }
        });
        let t = &mut self.temp;
        let mut bold = t.sub_bold;
        if check(ui, "sub-bold", i18n::tr("Bold"), &mut bold) {
            t.sub_bold = bold;
        }
        row(ui, i18n::tr("Text encoding for non-Unicode files"), |ui| {
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
        row(ui, i18n::tr("Load subtitles automatically"), |ui| {
            let t = &mut self.temp;
            let items: Vec<(String, String)> = [
                (i18n::tr("Same name as the video"), "exact"),
                (i18n::tr("Names containing the video name"), "fuzzy"),
                (i18n::tr("Every subtitle in the folder"), "all"),
                (i18n::tr("Do not load automatically"), "no"),
            ]
            .iter()
            .map(|(l, m)| (l.to_string(), m.to_string()))
            .collect();
            let cur = t.sub_auto.clone();
            if let Some(v) = combo(ui, "sub-auto", &items, &cur, 190.0) {
                t.sub_auto = v;
            }
        });
        row(ui, i18n::tr("Preferred subtitle languages"), |ui| {
            let t = &mut self.temp;
            let mut v = t.preferred_sub_lang.clone();
            if line_edit(ui, "sub-lang", &mut v, 170.0, "e.g. fa,per,en") {
                t.preferred_sub_lang = v;
            }
        });
        ui.add_space(6.0);
        heading(ui, i18n::tr("Subtitle Style"));
        let palette = |ui: &mut egui::Ui, id: &str, current: &mut String| {
            let items: Vec<(String, String)> = SUB_COLORS
                .iter()
                .map(|(hex, label)| (i18n::tr(label).to_string(), (*hex).to_string()))
                .collect();
            let mut cur = current.clone();
            if let Some(v) = combo(ui, id, &items, &cur, 130.0) {
                cur = v;
            }
            *current = cur;
        };
        row(ui, i18n::tr("Text color"), |ui| {
            let t = &mut self.temp;
            palette(ui, "sub-color", &mut t.sub_color);
        });
        row(ui, i18n::tr("Border width"), |ui| {
            let t = &mut self.temp;
            let mut v = t.sub_border_size;
            if spin(ui, "sub-border-size", &mut v, 0.5, 0.0, 10.0, "px") {
                t.sub_border_size = v;
            }
        });
        row(ui, i18n::tr("Border color"), |ui| {
            let t = &mut self.temp;
            palette(ui, "sub-border-color", &mut t.sub_border_color);
        });
        row(ui, i18n::tr("Shadow"), |ui| {
            let t = &mut self.temp;
            let mut v = t.sub_shadow_offset;
            if spin(ui, "sub-shadow", &mut v, 0.5, 0.0, 10.0, "px") {
                t.sub_shadow_offset = v;
            }
        });
        let t = &mut self.temp;
        let mut back = t.sub_back_enable;
        if check(ui, "sub-back", i18n::tr("Background box"), &mut back) {
            t.sub_back_enable = back;
        }
        if t.sub_back_enable {
            row(ui, i18n::tr("Background color"), |ui| {
                let t = &mut self.temp;
                palette(ui, "sub-back-color", &mut t.sub_back_color);
            });
        }
    }

    fn tab_audio(&mut self, ui: &mut egui::Ui) {
        row(ui, i18n::tr("Preferred audio languages"), |ui| {
            let t = &mut self.temp;
            let mut v = t.preferred_audio_lang.clone();
            if line_edit(ui, "audio-lang", &mut v, 170.0, "e.g. en,fa") {
                t.preferred_audio_lang = v;
            }
        });
        let t = &mut self.temp;
        let mut pitch = t.audio_pitch_correction;
        if check(
            ui,
            "audio-pitch",
            i18n::tr("Keep pitch when changing speed"),
            &mut pitch,
        ) {
            t.audio_pitch_correction = pitch;
        }
        row(ui, i18n::tr("Audio passthrough (S/PDIF)"), |ui| {
            let t = &mut self.temp;
            let items: Vec<(String, String)> = SPDIF_MODES
                .iter()
                .map(|(value, label)| (i18n::tr(label).to_string(), (*value).to_string()))
                .collect();
            let cur = t.audio_spdif.clone();
            if let Some(v) = combo(ui, "audio-spdif", &items, &cur, 190.0) {
                t.audio_spdif = v;
            }
        });
        let t = &mut self.temp;
        let mut exclusive = t.audio_exclusive;
        if check(
            ui,
            "audio-exclusive",
            i18n::tr("Exclusive audio output"),
            &mut exclusive,
        ) {
            t.audio_exclusive = exclusive;
        }
        row(ui, i18n::tr("Channel layout"), |ui| {
            let t = &mut self.temp;
            let items: Vec<(String, String)> = [
                (i18n::tr("Auto"), "auto"),
                (i18n::tr("Stereo"), "stereo"),
                (i18n::tr("Reversed stereo"), "reverse"),
                ("5.1", "5.1"),
                ("7.1", "7.1"),
            ]
            .iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect();
            let cur = t.audio_channels.clone();
            if let Some(v) = combo(ui, "audio-channels", &items, &cur, 130.0) {
                t.audio_channels = v;
            }
        });
        row(ui, i18n::tr("Default visualizer"), |ui| {
            let t = &mut self.temp;
            let items: Vec<(String, String)> = [
                (i18n::tr("Off"), "off"),
                (i18n::tr("Wave"), "wave"),
                (i18n::tr("Spectrum"), "spectrum"),
            ]
            .iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect();
            let cur = t.visualizer.clone();
            if let Some(v) = combo(ui, "visualizer", &items, &cur, 130.0) {
                t.visualizer = v;
            }
        });
        let t = &mut self.temp;
        let mut auto_viz = t.auto_visualizer;
        if check(
            ui,
            "auto-visualizer",
            i18n::tr("Show visualizer for audio files"),
            &mut auto_viz,
        ) {
            t.auto_visualizer = auto_viz;
        }
    }

    fn tab_network(&mut self, ui: &mut egui::Ui) {
        row(ui, i18n::tr("Stream cache (sec, 0 = auto)"), |ui| {
            let t = &mut self.temp;
            let mut v = t.cache_secs as f64;
            if spin(ui, "cache-secs", &mut v, 1.0, 0.0, 600.0, "") {
                t.cache_secs = v as i64;
            }
        });
        row(ui, i18n::tr("Demuxer cache (MiB, 0 = auto)"), |ui| {
            let t = &mut self.temp;
            let mut v = t.demuxer_max_mb as f64;
            if spin(ui, "demuxer-max", &mut v, 16.0, 0.0, 4096.0, "") {
                t.demuxer_max_mb = v as i64;
            }
        });
        row(ui, i18n::tr("HTTP user agent"), |ui| {
            let t = &mut self.temp;
            let mut v = t.http_user_agent.clone();
            if line_edit(ui, "http-user-agent", &mut v, 240.0, "Mozilla/5.0") {
                t.http_user_agent = v;
            }
        });
        row(ui, i18n::tr("HTTP headers"), |ui| {
            let t = &mut self.temp;
            let mut v = t.http_headers.clone();
            if line_edit(
                ui,
                "http-headers",
                &mut v,
                240.0,
                "Referer: https://example.com",
            ) {
                t.http_headers = v;
            }
        });
        ui.add_space(4.0);
        label(ui, i18n::tr("One header per line, format: Key: Value"));
        ui.add_space(4.0);
        label(
            ui,
            i18n::tr("These apply to URLs; local files are unaffected"),
        );
    }
}

/// Fixed palette offered for subtitle colors.
const SUB_COLORS: &[(&str, &str)] = &[
    ("#ffffff", "White"),
    ("#ffff00", "Yellow"),
    ("#00ffff", "Cyan"),
    ("#ffb000", "Amber"),
    ("#00ff00", "Green"),
    ("#000000", "Black"),
];

/// Passthrough presets: (mpv audio-spdif value, label).
const SPDIF_MODES: &[(&str, &str)] = &[
    ("", "None"),
    ("ac3", "AC3 (Dolby Digital)"),
    ("dts", "DTS"),
    ("ac3,dts", "AC3 + DTS"),
    ("eac3,ac3,dts", "E-AC3 + AC3 + DTS"),
    ("truehd,eac3,ac3,dts", "TrueHD + E-AC3 + AC3 + DTS"),
    (
        "dts-hd,truehd,eac3,ac3,dts",
        "Everything (DTS-HD, TrueHD, ...)",
    ),
];

// -- History dialog --------------------------------------------------------------------------------

/// Watched-file history list with per-row play buttons.
#[derive(Default)]
pub struct HistoryDialog {
    pub open: bool,
}

impl HistoryDialog {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        history: &[crate::settings::HistoryEntry],
        actions: &mut Vec<Action>,
    ) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut close = false;
        Window::new(i18n::tr("History"))
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .default_height(420.0)
            .frame(dlg_frame())
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(500.0);
                if dialog_header(ui, i18n::tr("History")) {
                    close = true;
                }
                let scroll_height = ui.available_height().max(200.0);
                egui::ScrollArea::vertical()
                    .id_salt("history-scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_height)
                    .show(ui, |ui| {
                        if history.is_empty() {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(i18n::tr("Nothing watched yet"))
                                    .size(12.0)
                                    .color(active_colors().text_dim),
                            );
                        }
                        let colors = active_colors();
                        let row_font = skin::font(12.0, false);
                        let meta_font = skin::mono(10.0);
                        for (index, entry) in history.iter().take(200).enumerate() {
                            let row_h = 40.0;
                            let row_w = ui.available_width();
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(row_w, row_h), Sense::click());
                            let resp =
                                ui.interact(rect, Id::new(("history-row", index)), Sense::click());
                            let p = ui.painter_at(rect);
                            if resp.hovered() {
                                p.rect_filled(rect, 6.0, colors.button_hover);
                            }
                            // Accent play chip on the left.
                            let chip = Rect::from_center_size(
                                pos2(rect.left() + 22.0, rect.center().y),
                                vec2(24.0, 24.0),
                            );
                            p.rect_filled(chip, 6.0, colors.select);
                            skin::draw_icon(
                                &p,
                                "play",
                                chip.center().x + 0.5,
                                chip.center().y,
                                colors.accent,
                                colors.select,
                            );
                            // Title over progress meta.
                            let title_x = chip.right() + 10.0;
                            let title_w = (row_w - (title_x - rect.left()) - 150.0).max(60.0);
                            let title = skin::elide(&p, &entry.title, &row_font, title_w);
                            p.text(
                                pos2(title_x, rect.top() + 7.0),
                                Align2::LEFT_TOP,
                                title,
                                row_font.clone(),
                                if resp.hovered() {
                                    colors.text_bright
                                } else {
                                    colors.text_e4
                                },
                            );
                            let progress = if entry.duration > 0.0 {
                                format!(
                                    "{} / {}  ·  ",
                                    crate::util::fmt_time(entry.position),
                                    crate::util::fmt_time(entry.duration)
                                )
                            } else {
                                format!("{}  ·  ", crate::util::fmt_time(entry.position))
                            };
                            p.text(
                                pos2(title_x, rect.bottom() - 9.0),
                                Align2::LEFT_BOTTOM,
                                format!(
                                    "{progress}{}",
                                    crate::util::format_unix_time(entry.played_at)
                                ),
                                meta_font.clone(),
                                colors.text_dim,
                            );
                            if resp.clicked() {
                                actions.push(Action::PlayHistory(entry.path.clone()));
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if button(ui, "history-clear", i18n::tr("Clear History")) && !history.is_empty()
                    {
                        actions.push(Action::ClearHistory);
                    }
                    ui.add_space(4.0);
                    if button(ui, "history-close", i18n::tr("Close")) {
                        self.open = false;
                    }
                });
            });
        if close {
            self.open = false;
        }
        self.open = self.open && open;
    }
}

// -- settings popover -----------------------------------------------------------------------------

/// The small quick-settings popover from the title-bar gear: theme, language and the
/// two most common toggles. Everything rarer lives in the full Preferences dialog.
#[derive(Debug, Default)]
pub struct SettingsPopover {
    pub open: bool,
    /// The click that opens the popover arrives in the same frame `show` first runs;
    /// the outside-close check must skip that frame or it closes instantly.
    pub just_opened: bool,
}

impl SettingsPopover {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        anchor: Rect,
        settings: &mut Settings,
        actions: &mut Vec<Action>,
    ) {
        if !self.open {
            return;
        }
        let colors = active_colors();
        let width = 292.0;
        let pos = pos2((anchor.right() - width).max(8.0), anchor.bottom() + 6.0);
        let mut keep_open = true;
        let area = egui::Area::new(egui::Id::new("settings-popover"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.dialog_bg)
                    .stroke(Stroke::new(1.0, colors.dialog_line))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(6))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 2,
                        color: colors.shadow,
                    })
                    .show(ui, |ui| {
                        // Pin both ends: an Area's available width runs to the screen
                        // edge, so a min-width alone lets the rows stretch wide.
                        ui.set_min_width(width - 12.0);
                        ui.set_max_width(width - 12.0);
                        let theme_items: Vec<(String, String)> =
                            [(i18n::tr("Dark"), "dark"), (i18n::tr("Light"), "light")]
                                .iter()
                                .map(|(l, v)| (l.to_string(), v.to_string()))
                                .collect();
                        popover_row(ui, i18n::tr("Theme"), |ui| {
                            if popover_seg(ui, &theme_items, &mut settings.theme) {
                                actions.push(Action::SetThemeLive(settings.theme.clone()));
                            }
                        });
                        let lang_items: Vec<(String, String)> =
                            [(i18n::tr("English"), "en"), (i18n::tr("Persian"), "fa")]
                                .iter()
                                .map(|(l, v)| (l.to_string(), v.to_string()))
                                .collect();
                        popover_row(ui, i18n::tr("Language"), |ui| {
                            if popover_seg(ui, &lang_items, &mut settings.language) {
                                actions.push(Action::SetLanguageLive(settings.language.clone()));
                            }
                        });
                        if popover_toggle(
                            ui,
                            i18n::tr("Resume where you left off"),
                            settings.resume,
                        ) {
                            actions.push(Action::ToggleResume);
                        }
                        if popover_toggle(ui, i18n::tr("Hardware decoding"), settings.hwdec) {
                            actions.push(Action::SetHwdec(!settings.hwdec));
                        }
                        ui.separator();
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(i18n::tr("All settings are in Preferences"))
                                        .size(11.5)
                                        .color(colors.text_9a),
                                )
                                .fill(Color32::TRANSPARENT),
                            )
                            .clicked()
                        {
                            actions.push(Action::ShowPreferences(None));
                            keep_open = false;
                        }
                    });
            });
        // Click anywhere outside the popover's real rect closes it — except on the
        // opening frame, whose click belongs to the gear button itself.
        if self.just_opened {
            self.just_opened = false;
        } else if ctx.input(|i| i.pointer.any_click())
            && !area
                .response
                .rect
                .contains(ctx.input(|i| i.pointer.latest_pos()).unwrap_or(pos))
        {
            keep_open = false;
        }
        self.open = keep_open;
    }
}

/// One labelled row of the settings popover.
fn popover_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new(label)
                .size(12.0)
                .color(active_colors().text_e4),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add(ui);
            ui.add_space(6.0);
        });
    });
    ui.add_space(2.0);
}

/// Small segmented control; mutates `current` and returns true when it changed.
fn popover_seg(ui: &mut egui::Ui, items: &[(String, String)], current: &mut String) -> bool {
    let colors = active_colors();
    let mut changed = false;
    ui.horizontal(|ui| {
        for (label, value) in items {
            let selected = value == current;
            let text = RichText::new(label.as_str()).size(11.0).color(if selected {
                colors.text_e4
            } else {
                colors.text_9a
            });
            let btn = egui::Button::new(text)
                .fill(if selected {
                    colors.button_hover
                } else {
                    Color32::TRANSPARENT
                })
                .corner_radius(6.0);
            if ui.add(btn).clicked() && !selected {
                *current = value.clone();
                changed = true;
            }
        }
    });
    changed
}

/// Small toggle row; returns true when clicked (the caller applies the flip).
/// The knob slides between ends with a short eased animation.
fn popover_toggle(ui: &mut egui::Ui, label: &str, on: bool) -> bool {
    let colors = active_colors();
    let response = ui
        .horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new(label)
                    .size(12.0)
                    .color(active_colors().text_e4),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 20.0), Sense::click());
                let painter = ui.painter_at(rect);
                let fill = if on {
                    colors.accent
                } else {
                    colors.button_hover
                };
                painter.rect_filled(rect, 10.0, fill);
                let slide = ui.ctx().animate_value_with_time(
                    Id::new(("popover-toggle", label)),
                    if on { 1.0 } else { 0.0 },
                    0.12,
                );
                let knob_x = rect.left() + 11.0 + slide * (rect.width() - 22.0);
                painter.circle_filled(pos2(knob_x, rect.center().y), 8.0, colors.text_e4);
            });
        })
        .response;
    response.clicked()
}
