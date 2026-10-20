//! Playlist panel: items list with drag reorder, context menus and background duration probing.
//!
//! Playlist panel with drag reorder, context menus and background duration probing.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{Context, Id, Pos2, Rect, Sense, Stroke, TextEdit, pos2, vec2};

use crate::menus::{self, Action, Entry, MenuState};
use crate::skin;
use crate::util::{fmt_time, is_url, natural_cmp, title_of};
use crate::widgets::{IconBtn, text_button};

pub const ROW_H: f32 = 22.0;
pub const HEADER_H: f32 = 26.0;
pub const FOOTER_H: f32 = 27.0;

pub struct Item {
    pub path: String,
    pub title: String,
    pub duration: Option<f64>,
}

#[derive(Default)]
pub struct PlaylistOut {
    pub play: Option<usize>,
    pub add_files: bool,
    pub add_folder: bool,
    pub add_url: bool,
    pub close: bool,
    pub repeat: bool,
    pub shuffle: bool,
    pub changed: bool,
}

enum Job {
    Probe(String),
    Stop,
}

/// Fills in durations with ffprobe in the background (skipped when ffprobe is missing).
struct DurationProber {
    jobs: Option<Sender<Job>>,
    results: Receiver<(String, f64)>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl DurationProber {
    fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (result_tx, result_rx) = channel::<(String, f64)>();
        let stop = Arc::new(AtomicBool::new(false));
        match which("ffprobe") {
            Some(ffprobe) => {
                let worker_stop = Arc::clone(&stop);
                let worker = std::thread::Builder::new()
                    .name("ffprobe".into())
                    .spawn(move || {
                        while let Ok(job) = job_rx.recv() {
                            if worker_stop.load(Ordering::Acquire) {
                                break;
                            }
                            match job {
                                Job::Stop => break,
                                Job::Probe(path) => {
                                    if let Some(duration) = probe(&ffprobe, &path, &worker_stop)
                                        && result_tx.send((path, duration)).is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                    })
                    .ok();
                Self {
                    jobs: worker.is_some().then_some(job_tx),
                    results: result_rx,
                    stop,
                    worker,
                }
            }
            None => Self {
                jobs: None,
                results: result_rx,
                stop,
                worker: None,
            },
        }
    }

    fn probe(&self, path: &str) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::Probe(path.to_string()));
        }
    }

    fn drain(&self, items: &mut [Item]) {
        while let Ok((path, duration)) = self.results.try_recv() {
            if !duration.is_finite() || duration <= 0.0 {
                continue;
            }
            for item in items.iter_mut() {
                if item.path == path && item.duration.is_none() {
                    item.duration = Some(duration);
                }
            }
        }
    }
}

impl Drop for DurationProber {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn which(program: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn probe(ffprobe: &str, path: &str, stop: &AtomicBool) -> Option<f64> {
    let mut child = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            path,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if stop.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                if !status.success() {
                    return None;
                }
                let duration = String::from_utf8(output.stdout)
                    .ok()?
                    .trim()
                    .parse::<f64>()
                    .ok()?;
                return (duration.is_finite() && duration > 0.0).then_some(duration);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn open_folder(path: &str) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(target_os = "macos"))]
    let program = "xdg-open";
    let _ = Command::new(program).arg(path).spawn();
}

#[derive(Clone, Copy, PartialEq)]
enum MenuKind {
    Add,
    Del,
    Sort,
    Context(usize),
}

pub struct Playlist {
    pub items: Vec<Item>,
    pub current: Option<usize>,
    pub selected: Vec<usize>,
    query: String,
    anchor: Option<usize>,
    pub scroll: f32,
    pub repeat_mode: String,
    pub shuffle: bool,
    pub has_focus: bool,
    drag: Option<usize>,
    menu: MenuState,
    menu_kind: Option<MenuKind>,
    prober: DurationProber,
}

impl Default for Playlist {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            current: None,
            selected: Vec::new(),
            query: String::new(),
            anchor: None,
            scroll: 0.0,
            repeat_mode: "off".into(),
            shuffle: false,
            has_focus: false,
            drag: None,
            menu: MenuState::default(),
            menu_kind: None,
            prober: DurationProber::new(),
        }
    }
}

impl Playlist {
    // -- model ---------------------------------------------------------------------------------

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn path(&self, row: usize) -> Option<&str> {
        self.items.get(row).map(|item| item.path.as_str())
    }

    pub fn paths(&self) -> Vec<String> {
        self.items.iter().map(|item| item.path.clone()).collect()
    }

    pub fn current_row(&self) -> Option<usize> {
        self.current
    }

    fn visible_rows(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.items.len()).collect();
        }
        self.items
            .iter()
            .enumerate()
            .filter_map(|(row, item)| {
                (item.title.to_lowercase().contains(&query)
                    || item.path.to_lowercase().contains(&query))
                .then_some(row)
            })
            .collect()
    }

    fn make_item(&self, path: &str) -> Item {
        Item {
            path: path.to_string(),
            title: title_of(path),
            duration: None,
        }
    }

    pub fn set_items(&mut self, paths: &[String]) {
        self.items = paths.iter().map(|path| self.make_item(path)).collect();
        for item in &self.items {
            if !is_url(&item.path) {
                self.prober.probe(&item.path);
            }
        }
        self.current = None;
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.scroll = 0.0;
    }

    pub fn add(&mut self, paths: &[String]) -> usize {
        let first = self.items.len();
        for path in paths {
            let item = self.make_item(path);
            if !is_url(path) {
                self.prober.probe(path);
            }
            self.items.push(item);
        }
        first
    }

    pub fn set_current(&mut self, row: Option<usize>) {
        self.current = row.filter(|row| *row < self.items.len());
        if let Some(row) = self.current {
            self.selected = vec![row];
            self.anchor = Some(row);
        } else {
            self.selected.clear();
            self.anchor = None;
        }
    }

    pub fn set_duration(&mut self, path: &str, duration: f64) {
        if !duration.is_finite() || duration <= 0.0 {
            return;
        }
        for item in self.items.iter_mut() {
            if item.path == path && item.duration.is_none() {
                item.duration = Some(duration);
            }
        }
    }

    pub fn remove_selected(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let selected: std::collections::HashSet<usize> = self.selected.iter().copied().collect();
        let before = self.items.len();
        let current = self.current;
        let mut index = 0usize;
        self.items.retain(|_| {
            let keep = !selected.contains(&index);
            index += 1;
            keep
        });
        self.current = current.and_then(|current| {
            if selected.contains(&current) || current >= before {
                None
            } else {
                Some(current - selected.iter().filter(|row| **row < current).count())
            }
        });
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.items.len() != before
    }

    pub fn clear(&mut self) -> bool {
        if self.items.is_empty() {
            return false;
        }
        self.items.clear();
        self.current = None;
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.scroll = 0.0;
        true
    }

    pub fn remove_missing(&mut self) -> bool {
        let before = self.items.len();
        let current_path = self
            .current
            .and_then(|row| self.items.get(row).map(|item| item.path.clone()));
        self.items
            .retain(|item| is_url(&item.path) || Path::new(&item.path).exists());
        self.current =
            current_path.and_then(|path| self.items.iter().position(|item| item.path == path));
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.items.len() != before
    }

    pub fn remove_duplicates(&mut self) -> bool {
        let before = self.items.len();
        let current_path = self
            .current
            .and_then(|row| self.items.get(row).map(|item| item.path.clone()));
        let mut seen = std::collections::HashSet::new();
        self.items.retain(|item| seen.insert(item.path.clone()));
        self.current =
            current_path.and_then(|path| self.items.iter().position(|item| item.path == path));
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.items.len() != before
    }

    pub fn sort_by(&mut self, key: &str) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let current_path = self
            .current
            .and_then(|row| self.items.get(row).map(|item| item.path.clone()));
        match key {
            "name" => self.items.sort_by(|a, b| natural_cmp(&a.title, &b.title)),
            "path" => self.items.sort_by(|a, b| natural_cmp(&a.path, &b.path)),
            "duration" => self.items.sort_by(|a, b| {
                a.duration
                    .unwrap_or(0.0)
                    .total_cmp(&b.duration.unwrap_or(0.0))
            }),
            "reverse" => self.items.reverse(),
            "random" => {
                use rand::seq::SliceRandom;
                self.items.shuffle(&mut rand::rng());
            }
            _ => return false,
        }
        self.current =
            current_path.and_then(|path| self.items.iter().position(|item| item.path == path));
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        true
    }

    /// Row to play after the current one, or `None` when the list is exhausted.
    pub fn step(&mut self, direction: i32, repeat: &str, shuffle: bool) -> Option<usize> {
        let count = self.items.len();
        if count == 0 {
            return None;
        }
        let row = self.current.map(|row| row as i64).unwrap_or(-1);
        if shuffle && count > 1 {
            use rand::RngExt;
            let mut rng = rand::rng();
            loop {
                let pick = rng.random_range(0..count as i64);
                if pick != row {
                    return Some(pick as usize);
                }
            }
        }
        let next = row + direction as i64;
        if next >= 0 && (next as usize) < count {
            return Some(next as usize);
        }
        if repeat == "all" {
            return Some(next.rem_euclid(count as i64) as usize);
        }
        None
    }

    pub fn set_modes(&mut self, repeat: &str, shuffle: bool) {
        self.repeat_mode = repeat.to_string();
        self.shuffle = shuffle;
    }

    fn select(&mut self, row: usize, shift: bool, ctrl: bool) {
        if ctrl {
            if let Some(index) = self.selected.iter().position(|r| *r == row) {
                self.selected.remove(index);
            } else {
                self.selected.push(row);
            }
            self.anchor = Some(row);
        } else if shift {
            if let Some(anchor) = self.anchor {
                if self.query.trim().is_empty() {
                    self.selected = (anchor.min(row)..=anchor.max(row)).collect();
                } else {
                    let visible_rows = self.visible_rows();
                    let anchor = visible_rows.iter().position(|visible| *visible == anchor);
                    let row_position = visible_rows.iter().position(|visible| *visible == row);
                    if let (Some(anchor), Some(row_position)) = (anchor, row_position) {
                        self.selected = visible_rows
                            [anchor.min(row_position)..=anchor.max(row_position)]
                            .to_vec();
                    } else {
                        self.selected = vec![row];
                    }
                }
            } else {
                self.selected = vec![row];
                self.anchor = Some(row);
            }
        } else {
            self.selected = vec![row];
            self.anchor = Some(row);
        }
        self.selected.sort_unstable();
    }

    // -- drawing -------------------------------------------------------------------------------

    /// Draw the panel into `rect` and report what the player should do.
    pub fn ui(&mut self, ui: &mut egui::Ui, rect: Rect, ctx: &Context) -> PlaylistOut {
        let mut out = PlaylistOut::default();
        self.prober.drain(&mut self.items);

        let colors = skin::active_colors();
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, ui.style().visuals.window_corner_radius, colors.bg);
        let w = rect.width();
        let h = rect.height();

        // -- header -----------------------------------------------------------------------------
        let font_id = skin::font(12.0, false);
        let label = "Playlist";
        let label_w = skin::text_width(&painter, label, &font_id) + 24.0;
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(label_w, HEADER_H)),
            0.0,
            colors.panel,
        );
        painter.rect_filled(
            Rect::from_min_size(
                pos2(rect.left(), rect.top() + HEADER_H - 2.0),
                vec2(label_w, 2.0),
            ),
            0.0,
            colors.accent,
        );
        painter.line_segment(
            [
                pos2(rect.left() + label_w, rect.top()),
                pos2(rect.left() + label_w, rect.top() + HEADER_H),
            ],
            Stroke::new(1.0, colors.line),
        );
        painter.text(
            Rect::from_min_size(rect.min, vec2(label_w, HEADER_H - 1.0)).center(),
            egui::Align2::CENTER_CENTER,
            label,
            font_id.clone(),
            colors.text_bright,
        );
        let count = self.items.len();
        let visible_rows = self.visible_rows();
        let visible_count = visible_rows.len();
        if count > 0 {
            let total: f64 = self.items.iter().filter_map(|item| item.duration).sum();
            let mut info = if self.query.trim().is_empty() {
                format!("{count} item{}", if count == 1 { "" } else { "s" })
            } else {
                format!(
                    "{visible_count}/{count} item{}",
                    if count == 1 { "" } else { "s" }
                )
            };
            if total > 0.0 {
                info.push_str(&format!("  ·  {}", fmt_time(total)));
            }
            let search_left = (rect.right() - 164.0).max(rect.left() + label_w + 4.0);
            let info_room = (search_left - (rect.left() + label_w + 10.0) - 8.0).max(0.0);
            info = skin::elide(&painter, &info, &font_id, info_room);
            painter.text(
                pos2(rect.left() + label_w + 10.0, rect.top() + HEADER_H / 2.0),
                egui::Align2::LEFT_CENTER,
                info,
                font_id,
                colors.text_dim,
            );
        }
        let search_rect = Rect::from_min_max(
            pos2(
                (rect.right() - 164.0).max(rect.left() + label_w + 4.0),
                rect.top() + 3.0,
            ),
            pos2(rect.right() - 30.0, rect.top() + HEADER_H - 3.0),
        );
        if search_rect.width() > 20.0 {
            let response = ui.put(
                search_rect,
                TextEdit::singleline(&mut self.query)
                    .desired_width(search_rect.width())
                    .hint_text("Filter"),
            );
            if response.changed() {
                self.scroll = 0.0;
            }
        }
        painter.line_segment(
            [
                pos2(rect.left(), rect.top() + HEADER_H),
                pos2(rect.right(), rect.top() + HEADER_H),
            ],
            Stroke::new(1.0, colors.line),
        );
        let close_click = IconBtn::new("close", 26.0, HEADER_H)
            .tip("Close playlist (F6)")
            .color(colors.icon_dim)
            .flat(true)
            .ui(
                ui,
                ui.id().with("pl-close"),
                Rect::from_min_size(pos2(rect.right() - 26.0, rect.top()), vec2(26.0, HEADER_H)),
            );
        out.close = close_click.primary;

        // -- list -------------------------------------------------------------------------------
        let view = Rect::from_min_max(
            pos2(rect.left(), rect.top() + HEADER_H + 1.0),
            pos2(rect.right(), rect.bottom() - FOOTER_H),
        );
        let rows = visible_count as f32 * ROW_H;
        let max_scroll = (rows - view.height()).max(0.0);
        if view.height() > 0.0 {
            let over_view = ctx.pointer_hover_pos().is_some_and(|p| view.contains(p));
            if over_view {
                let wheel = ui.input(|i| -i.smooth_scroll_delta.y);
                if wheel != 0.0 {
                    self.scroll = (self.scroll + wheel).clamp(0.0, max_scroll);
                }
            }
            self.paint_rows(ui, ctx, view, &visible_rows, &mut out);
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);

        // -- footer -----------------------------------------------------------------------------
        painter.line_segment(
            [
                pos2(rect.left(), rect.bottom() - FOOTER_H),
                pos2(rect.right(), rect.bottom() - FOOTER_H),
            ],
            Stroke::new(1.0, colors.line),
        );
        let footer_y = rect.bottom() - FOOTER_H + 1.0;
        let add_rect = Rect::from_min_size(pos2(rect.left(), footer_y), vec2(46.0, FOOTER_H - 1.0));
        let del_rect = Rect::from_min_size(
            pos2(rect.left() + 47.0, footer_y),
            vec2(44.0, FOOTER_H - 1.0),
        );
        let sort_rect = Rect::from_min_size(
            pos2(rect.left() + 92.0, footer_y),
            vec2(52.0, FOOTER_H - 1.0),
        );
        text_button(ui, ui.id().with("pl-add"), add_rect, "Add");
        text_button(ui, ui.id().with("pl-del"), del_rect, "Del");
        text_button(ui, ui.id().with("pl-sort"), sort_rect, "Sort");
        if ui
            .interact(add_rect, ui.id().with("pl-add-hit"), Sense::click())
            .clicked()
        {
            self.open_menu(
                MenuKind::Add,
                add_rect.left_bottom() - vec2(0.0, FOOTER_H - 1.0),
            );
        }
        if ui
            .interact(del_rect, ui.id().with("pl-del-hit"), Sense::click())
            .clicked()
        {
            self.open_menu(
                MenuKind::Del,
                del_rect.left_bottom() - vec2(0.0, FOOTER_H - 1.0),
            );
        }
        if ui
            .interact(sort_rect, ui.id().with("pl-sort-hit"), Sense::click())
            .clicked()
        {
            self.open_menu(
                MenuKind::Sort,
                sort_rect.left_bottom() - vec2(0.0, FOOTER_H - 1.0),
            );
        }
        for x in [add_rect.right(), del_rect.right(), sort_rect.right()] {
            painter.line_segment(
                [pos2(x, footer_y), pos2(x, rect.bottom())],
                Stroke::new(1.0, colors.line),
            );
        }

        let repeat_rect = Rect::from_min_size(
            pos2(rect.right() - 57.0, footer_y),
            vec2(28.0, FOOTER_H - 1.0),
        );
        let shuffle_rect = Rect::from_min_size(
            pos2(rect.right() - 28.0, footer_y),
            vec2(28.0, FOOTER_H - 1.0),
        );
        let repeat_tip = match self.repeat_mode.as_str() {
            "one" => "Repeat: current file",
            "all" => "Repeat: playlist",
            _ => "Repeat: off",
        };
        out.repeat = IconBtn::new("repeat", 28.0, FOOTER_H - 1.0)
            .tip(repeat_tip)
            .color(colors.icon_dim)
            .checked(self.repeat_mode != "off")
            .ui(ui, ui.id().with("pl-repeat"), repeat_rect)
            .primary;
        out.shuffle = IconBtn::new("shuffle", 28.0, FOOTER_H - 1.0)
            .tip("Shuffle")
            .color(colors.icon_dim)
            .checked(self.shuffle)
            .ui(ui, ui.id().with("pl-shuffle"), shuffle_rect)
            .primary;

        // -- menu -------------------------------------------------------------------------------
        if let Some(action) = self.paint_menu(ui, ctx) {
            match action {
                Action::PlayRow(row) => out.play = Some(row),
                Action::OpenFiles { .. } => out.add_files = true,
                Action::OpenFolder { .. } => out.add_folder = true,
                Action::OpenUrl => out.add_url = true,
                Action::PlaylistRemove => out.changed |= self.remove_selected(),
                Action::PlaylistRemoveMissing => out.changed |= self.remove_missing(),
                Action::PlaylistRemoveDuplicates => out.changed |= self.remove_duplicates(),
                Action::PlaylistClear => out.changed |= self.clear(),
                Action::PlaylistSort(key) => out.changed |= self.sort_by(key),
                Action::PlaylistCopyPath(row) => {
                    if let Some(path) = self.path(row) {
                        ctx.copy_text(path.to_string());
                    }
                }
                Action::PlaylistShowInFolder(row) => {
                    if let Some(parent) = self.path(row).and_then(|path| Path::new(path).parent()) {
                        open_folder(&parent.to_string_lossy());
                    }
                }
                _ => {}
            }
        }

        let _ = (w, h);
        out
    }

    fn paint_rows(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        view: Rect,
        visible_rows: &[usize],
        out: &mut PlaylistOut,
    ) {
        let colors = skin::active_colors();
        let painter = ui.painter_at(view);
        let first = (self.scroll / ROW_H).floor() as usize;
        let visible = ((view.height() / ROW_H).ceil() as usize) + 1;
        let last = (first + visible).min(visible_rows.len());
        let font_id = skin::font(12.0, false);
        let num_w = painter
            .fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap("00.".to_string(), font_id.clone(), colors.text_dim)
                    .size()
                    .x
            })
            .max(10.0)
            + 2.0;

        let mut hovered_row: Option<usize> = None;
        for visible_row in first..last {
            let row = visible_rows[visible_row];
            let y = view.top() + visible_row as f32 * ROW_H - self.scroll;
            let row_rect = Rect::from_min_size(pos2(view.left(), y), vec2(view.width(), ROW_H));
            let id = Id::new("pl-row").with(row);
            let response = ui.interact(row_rect, id, Sense::click_and_drag());
            let selected = self.selected.contains(&row);
            let current = self.current == Some(row);
            if response.hovered() {
                hovered_row = Some(row);
            }
            let bg = if selected {
                colors.select
            } else if response.hovered() {
                colors.bg_hover
            } else {
                colors.bg
            };
            painter.rect_filled(row_rect, 0.0, bg);

            let mut right = row_rect.right() - 8.0;
            if let Some(duration) = self.items[row].duration {
                let text = fmt_time(duration);
                let tw = skin::text_width(&painter, &text, &font_id);
                painter.text(
                    pos2(right - tw, row_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font_id.clone(),
                    if current {
                        colors.accent
                    } else {
                        colors.text_dim
                    },
                );
                right = (right - tw - 10.0).max(row_rect.left() + 10.0);
            }
            let number = format!("{}.", visible_row + 1);
            painter.text(
                pos2(row_rect.left() + 8.0 + num_w, row_rect.center().y),
                egui::Align2::RIGHT_CENTER,
                number,
                font_id.clone(),
                if current {
                    colors.accent
                } else {
                    colors.text_dim
                },
            );
            let title_x = row_rect.left() + 8.0 + num_w + 7.0;
            let room = (right - title_x).max(10.0);
            let title = skin::elide(&painter, &self.items[row].title, &font_id, room);
            painter.text(
                pos2(title_x, row_rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                font_id.clone(),
                if current { colors.accent } else { colors.text },
            );

            let mods = ui.input(|i| i.modifiers);
            let ctrl = mods.command || mods.ctrl;
            if response.clicked_by(egui::PointerButton::Primary) {
                self.select(row, mods.shift, ctrl);
                self.has_focus = true;
            }
            if response.double_clicked_by(egui::PointerButton::Primary) {
                self.select(row, false, false);
                self.has_focus = true;
                out.play = Some(row);
            }
            if response.secondary_clicked() {
                self.select(row, false, false);
                self.has_focus = true;
                self.open_menu(
                    MenuKind::Context(row),
                    response.hover_pos().unwrap_or(row_rect.left_top()),
                );
            }

            // Drag to reorder.
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.drag = Some(row);
            }
            if self.drag == Some(row) {
                if response.dragged_by(egui::PointerButton::Primary)
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let insert = self.drop_index_for(pos.y, view, visible_rows.len());
                    let line_y = view.top() + insert as f32 * ROW_H - self.scroll;
                    ui.painter().line_segment(
                        [pos2(view.left(), line_y), pos2(view.right(), line_y)],
                        Stroke::new(2.0, colors.accent),
                    );
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let insert = self.drop_index_for(pos.y, view, visible_rows.len());
                        self.reorder_filtered(row, visible_row, insert, visible_rows);
                        out.changed = true;
                    }
                    self.drag = None;
                }
            }
        }

        // Click on empty space clears the selection; click outside drops list focus.
        if ui.input(|i| i.pointer.primary_pressed())
            && let Some(pos) = ctx.pointer_hover_pos()
            && !self.menu.is_open()
        {
            if view.contains(pos) && hovered_row.is_none() {
                self.selected.clear();
                self.has_focus = true;
            } else if !view.contains(pos)
                && !ui
                    .interact(rect_of(ui), Id::new("pl-panel"), Sense::hover())
                    .hovered()
            {
                self.has_focus = false;
            }
        }

        if self.has_focus && !self.menu.is_open() && !ctx.egui_wants_keyboard_input() {
            self.handle_keys(ctx, out);
        }
    }

    fn drop_index_for(&self, y: f32, view: Rect, row_count: usize) -> usize {
        let rel = y - view.top() + self.scroll;
        ((rel / ROW_H).round() as usize).clamp(0, row_count)
    }

    fn reorder_filtered(
        &mut self,
        from: usize,
        from_visible: usize,
        mut to_visible: usize,
        visible_rows: &[usize],
    ) {
        if from >= self.items.len() || from_visible >= visible_rows.len() {
            return;
        }
        if to_visible > from_visible {
            to_visible -= 1;
        }
        if to_visible == from_visible {
            return;
        }
        let remaining_visible: Vec<usize> = visible_rows
            .iter()
            .copied()
            .filter(|row| *row != from)
            .collect();
        let to = remaining_visible
            .get(to_visible)
            .copied()
            .map(|row| if row > from { row - 1 } else { row })
            .unwrap_or(self.items.len() - 1);
        let item = self.items.remove(from);
        self.items.insert(to.min(self.items.len()), item);
        self.current = self.current.map(|current| {
            if current == from {
                to
            } else if from < current && to >= current {
                current - 1
            } else if from > current && to <= current {
                current + 1
            } else {
                current
            }
        });
        self.selected = vec![to];
        self.anchor = Some(to);
    }

    fn handle_keys(&mut self, ctx: &Context, out: &mut PlaylistOut) {
        let visible_rows = self.visible_rows();
        let count = visible_rows.len() as i64;
        let current = self
            .selected
            .last()
            .and_then(|row| visible_rows.iter().position(|visible| visible == row))
            .map(|row| row as i64)
            .unwrap_or(-1);
        let mut next: Option<i64> = None;
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && !modifiers.ctrl
                    && !modifiers.alt
                {
                    match key {
                        egui::Key::ArrowUp => next = Some(current - 1),
                        egui::Key::ArrowDown => next = Some(current + 1),
                        egui::Key::Home => next = Some(0),
                        egui::Key::End => next = Some(count - 1),
                        egui::Key::PageUp => next = Some(current - 10),
                        egui::Key::PageDown => next = Some(current + 10),
                        egui::Key::Enter => {
                            if let Some(row) = self.selected.last().copied() {
                                if row < self.items.len() {
                                    out.play = Some(row);
                                }
                            }
                        }
                        egui::Key::Delete => out.changed |= self.remove_selected(),
                        egui::Key::Escape => {
                            self.has_focus = false;
                            self.selected.clear();
                        }
                        _ => {}
                    }
                }
            }
        });
        if let Some(next) = next.filter(|_| count > 0) {
            let next = next.clamp(0, count - 1) as usize;
            self.select(visible_rows[next], false, false);
            let top = next as f32 * ROW_H;
            if top < self.scroll {
                self.scroll = top;
            } else if top + ROW_H > self.scroll + 100.0 {
                self.scroll = top + ROW_H - 100.0;
            }
        }
    }

    // -- menus ---------------------------------------------------------------------------------

    fn open_menu(&mut self, kind: MenuKind, at: Pos2) {
        self.menu.open_at(at);
        self.menu_kind = Some(kind);
    }

    fn menu_entries(&self, kind: MenuKind) -> Vec<Entry> {
        match kind {
            MenuKind::Add => vec![
                Entry::item("Add Files...", Action::OpenFiles { append: true }),
                Entry::item("Add Folder...", Action::OpenFolder { append: true }),
                Entry::item("Add URL...", Action::OpenUrl),
            ],
            MenuKind::Del => vec![
                Entry::item("Remove Selected", Action::PlaylistRemove)
                    .enabled(!self.selected.is_empty()),
                Entry::item("Remove Missing Files", Action::PlaylistRemoveMissing),
                Entry::item("Remove Duplicates", Action::PlaylistRemoveDuplicates),
                Entry::Sep,
                Entry::item("Clear Playlist", Action::PlaylistClear),
            ],
            MenuKind::Sort => vec![
                Entry::item("By Name", Action::PlaylistSort("name")),
                Entry::item("By Path", Action::PlaylistSort("path")),
                Entry::item("By Duration", Action::PlaylistSort("duration")),
                Entry::Sep,
                Entry::item("Reverse", Action::PlaylistSort("reverse")),
                Entry::item("Randomize", Action::PlaylistSort("random")),
            ],
            MenuKind::Context(row) => {
                let has_row = row < self.items.len();
                let mut entries = vec![
                    Entry::item("Play", Action::PlayRow(row)).enabled(has_row),
                    Entry::Sep,
                    Entry::item("Add Files...", Action::OpenFiles { append: true }),
                    Entry::item("Add Folder...", Action::OpenFolder { append: true }),
                    Entry::item("Add URL...", Action::OpenUrl),
                    Entry::Sep,
                    Entry::item("Remove", Action::PlaylistRemove)
                        .enabled(!self.selected.is_empty()),
                    Entry::item("Remove Missing Files", Action::PlaylistRemoveMissing),
                    Entry::item("Remove Duplicates", Action::PlaylistRemoveDuplicates),
                    Entry::item("Clear Playlist", Action::PlaylistClear),
                    Entry::Sep,
                    Entry::sub(
                        "Sort By",
                        vec![
                            Entry::item("Name", Action::PlaylistSort("name")),
                            Entry::item("Path", Action::PlaylistSort("path")),
                            Entry::item("Duration", Action::PlaylistSort("duration")),
                            Entry::Sep,
                            Entry::item("Reverse Order", Action::PlaylistSort("reverse")),
                        ],
                    ),
                    Entry::Sep,
                    Entry::item("Copy Path", Action::PlaylistCopyPath(row)).enabled(has_row),
                ];
                let local = has_row && self.path(row).is_some_and(|path| !is_url(path));
                entries.push(
                    Entry::item("Show in File Manager", Action::PlaylistShowInFolder(row))
                        .enabled(local),
                );
                entries
            }
        }
    }

    fn paint_menu(&mut self, ui: &mut egui::Ui, ctx: &Context) -> Option<Action> {
        let Some(kind) = self.menu_kind else {
            return None;
        };
        if !self.menu.is_open() {
            self.menu_kind = None;
            return None;
        }
        let entries = self.menu_entries(kind);
        let action = menus::show(ui, &mut self.menu, "playlist", &entries);
        if action.is_none() && !self.menu.is_open() {
            self.menu_kind = None;
        }
        let _ = ctx;
        action
    }
}

fn rect_of(ui: &egui::Ui) -> Rect {
    ui.max_rect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn executable_script(contents: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("macplayer-playlist-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ffprobe-test");
        std::fs::write(&script, contents).unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();
        (dir, script)
    }

    fn playlist_with_paths(paths: &[&str]) -> Playlist {
        let mut playlist = Playlist::default();
        playlist.items = paths
            .iter()
            .map(|path| Item {
                path: (*path).to_string(),
                title: title_of(path),
                duration: None,
            })
            .collect();
        playlist
    }

    #[test]
    fn removing_before_current_keeps_current_item_selected_by_index() {
        let mut playlist = playlist_with_paths(&["a.mkv", "b.mkv", "c.mkv"]);
        playlist.current = Some(2);
        playlist.selected = vec![0];

        assert!(playlist.remove_selected());
        assert_eq!(playlist.paths(), vec!["b.mkv", "c.mkv"]);
        assert_eq!(playlist.current_row(), Some(1));
    }

    #[test]
    fn invalid_selection_does_not_report_a_change() {
        let mut playlist = playlist_with_paths(&["a.mkv"]);
        playlist.selected = vec![99];
        assert!(!playlist.remove_selected());
        assert_eq!(playlist.paths(), vec!["a.mkv"]);
    }

    #[test]
    fn replacing_items_resets_shift_selection_anchor() {
        let mut playlist =
            playlist_with_paths(&["https://example.test/a", "https://example.test/b"]);
        playlist.set_current(Some(1));
        playlist.set_items(&[
            "https://example.test/new-a".to_string(),
            "https://example.test/new-b".to_string(),
        ]);
        playlist.select(0, true, false);
        assert_eq!(playlist.selected, vec![0]);
    }

    #[test]
    fn current_row_rejects_out_of_range_values() {
        let mut playlist = playlist_with_paths(&["https://example.test/a"]);
        playlist.selected = vec![0];
        playlist.set_current(Some(4));
        assert_eq!(playlist.current_row(), None);
        assert!(playlist.selected.is_empty());
    }

    #[test]
    fn filter_matches_titles_and_paths_without_reordering_items() {
        let mut playlist = playlist_with_paths(&["/media/alpha.mkv", "/media/Beta.mkv"]);
        playlist.query = "BETA".into();

        assert_eq!(playlist.visible_rows(), vec![1]);
        assert_eq!(
            playlist.paths(),
            vec!["/media/alpha.mkv", "/media/Beta.mkv"]
        );
    }

    #[test]
    fn clearing_filter_keeps_selection_on_original_row() {
        let mut playlist =
            playlist_with_paths(&["/media/alpha.mkv", "/media/beta.mkv", "/media/gamma.mkv"]);
        playlist.query = "beta".into();
        playlist.selected = vec![1];
        playlist.query.clear();

        assert_eq!(playlist.visible_rows(), vec![0, 1, 2]);
        assert_eq!(playlist.selected, vec![1]);
        assert_eq!(playlist.path(playlist.selected[0]), Some("/media/beta.mkv"));
    }

    #[test]
    fn filtered_reorder_changes_matching_items_without_reordering_hidden_items() {
        let mut playlist = playlist_with_paths(&[
            "/media/alpha.mkv",
            "/media/keep-one.mkv",
            "/media/beta.mkv",
            "/media/keep-two.mkv",
            "/media/gamma.mkv",
        ]);
        playlist.query = "keep".into();
        let visible_rows = playlist.visible_rows();
        playlist.reorder_filtered(1, 0, 2, &visible_rows);

        assert_eq!(
            playlist.paths(),
            vec![
                "/media/alpha.mkv",
                "/media/beta.mkv",
                "/media/keep-two.mkv",
                "/media/gamma.mkv",
                "/media/keep-one.mkv",
            ]
        );
    }

    #[test]
    fn non_finite_duration_is_ignored() {
        let mut playlist = playlist_with_paths(&["https://example.test/a"]);
        playlist.set_duration("https://example.test/a", f64::NAN);
        playlist.set_duration("https://example.test/a", f64::INFINITY);
        assert_eq!(playlist.items[0].duration, None);
    }

    #[cfg(unix)]
    #[test]
    fn duration_probe_reads_successful_finite_output() {
        let (dir, script) = executable_script(b"#!/bin/sh\nprintf '12.5\\n'\n");
        let stop = AtomicBool::new(false);
        assert_eq!(
            probe(&script.to_string_lossy(), "clip.mkv", &stop),
            Some(12.5)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
