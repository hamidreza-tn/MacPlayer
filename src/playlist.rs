//! Playlist panel: items list with drag reorder, context menus and background duration probing.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::i18n;

use egui::{Color32, Context, Id, Pos2, Rect, Sense, Stroke, TextEdit, pos2, vec2};

use crate::menus::{self, Action, Entry, MenuState};
use crate::skin;
use crate::thumbs::Thumbnails;
use crate::util::{fmt_time, is_url, natural_cmp, title_of};
use crate::widgets::{IconBtn, text_button};

pub const ROW_H: f32 = 58.0;
pub const HEADER_H: f32 = 86.0;
pub const FOOTER_H: f32 = 42.0;

/// Normalized playback fraction for the current-row progress marker: `None` for
/// non-finite input, otherwise clamped to 0..1.
pub fn progress_fraction(raw: f64) -> Option<f64> {
    raw.is_finite().then(|| raw.clamp(0.0, 1.0))
}

/// Scroll offset that keeps the row at `row_top` fully inside a `view_h` window.
/// Rows above the view pull the scroll up; rows below push it down; rows already
/// visible leave the scroll untouched.
fn reveal_scroll(scroll: f32, row_top: f32, view_h: f32) -> f32 {
    let view_h = view_h.max(ROW_H);
    if row_top < scroll {
        row_top
    } else if row_top + ROW_H > scroll + view_h {
        (row_top + ROW_H - view_h).max(0.0)
    } else {
        scroll
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaStatus {
    Loading,
    Ready,
    Failed(String),
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: String,
    pub title: String,
    pub duration: Option<f64>,
    pub status: MediaStatus,
}

#[derive(Clone, Debug, PartialEq)]
struct PlaylistSnapshot {
    items: Vec<Item>,
    current: Option<usize>,
    selected: Vec<usize>,
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
    /// Row to reveal in the platform file manager (handled by the player,
    /// which owns the platform services).
    pub show_in_folder: Option<usize>,
    pub changed: bool,
}

enum Job {
    Probe { generation: u64, path: String },
    Stop,
}

/// Fills in durations with ffprobe in the background (skipped when ffprobe is missing).
struct DurationProber {
    jobs: Option<SyncSender<Job>>,
    results: Receiver<(u64, String, f64)>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Probe requests that did not fit the bounded job channel; flushed back
    /// a slice at a time by `drain` so large playlists never lose durations.
    backlog: std::sync::Mutex<Vec<(u64, String)>>,
}

impl DurationProber {
    fn new() -> Self {
        let (job_tx, job_rx) = sync_channel::<Job>(64);
        let (result_tx, result_rx) = sync_channel::<(u64, String, f64)>(128);
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
                                Job::Probe { generation, path } => {
                                    if let Some(duration) = probe(&ffprobe, &path, &worker_stop)
                                        && result_tx.send((generation, path, duration)).is_err()
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
                    backlog: std::sync::Mutex::new(Vec::new()),
                }
            }
            None => Self {
                jobs: None,
                results: result_rx,
                stop,
                worker: None,
                backlog: std::sync::Mutex::new(Vec::new()),
            },
        }
    }

    fn probe(&self, generation: u64, path: &str) {
        let Some(jobs) = &self.jobs else {
            return;
        };
        match jobs.try_send(Job::Probe {
            generation,
            path: path.to_string(),
        }) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                // Channel momentarily full (a big playlist was just added);
                // hold the request and retry from `drain`.
                if let Ok(mut backlog) = self.backlog.lock()
                    && backlog.len() < 4096
                {
                    backlog.push((generation, path.to_string()));
                }
            }
        }
    }

    fn drain(&self, generation: u64, items: &mut [Item]) {
        while let Ok((result_generation, path, duration)) = self.results.try_recv() {
            if result_generation != generation {
                continue;
            }
            if !duration.is_finite() || duration <= 0.0 {
                continue;
            }
            for item in items.iter_mut() {
                if item.path == path && item.duration.is_none() {
                    item.duration = Some(duration);
                }
            }
        }
        // Feed the bounded job channel from the backlog a slice at a time, so
        // durations for large playlists arrive eventually instead of never.
        if let Ok(mut backlog) = self.backlog.lock()
            && !backlog.is_empty()
            && let Some(jobs) = &self.jobs
        {
            let mut sent = 0usize;
            let mut kept: Vec<(u64, String)> = Vec::new();
            for (job_generation, path) in backlog.drain(..) {
                if sent < 32
                    && job_generation == generation
                    && jobs
                        .try_send(Job::Probe {
                            generation: job_generation,
                            path: path.clone(),
                        })
                        .is_ok()
                {
                    sent += 1;
                } else {
                    kept.push((job_generation, path));
                }
            }
            *backlog = kept;
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
    crate::util::which(program)
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
    probe_generation: u64,
    undo: Vec<PlaylistSnapshot>,
    redo: Vec<PlaylistSnapshot>,
    menu: MenuState,
    menu_kind: Option<MenuKind>,
    prober: DurationProber,
    /// Playback fraction (0..1) of the currently playing item, refreshed every
    /// frame by the player; `None` when nothing plays or the duration is unknown.
    pub progress: Option<f64>,
    /// Visible list height, refreshed while painting; drives reveal/keyboard scroll.
    view_h: f32,
    /// Pointer offset inside the scrollbar thumb while it is being dragged.
    scroll_drag: Option<f32>,
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
            probe_generation: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            menu: MenuState::default(),
            menu_kind: None,
            prober: DurationProber::new(),
            progress: None,
            view_h: 300.0,
            scroll_drag: None,
        }
    }
}

impl Playlist {
    // -- model ---------------------------------------------------------------------------------

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_open()
    }

    pub fn close_menu(&mut self) {
        self.menu.close();
        self.menu_kind = None;
    }

    pub fn clear_focus(&mut self) {
        self.has_focus = false;
        self.selected.clear();
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

    fn snapshot(&self) -> PlaylistSnapshot {
        PlaylistSnapshot {
            items: self.items.clone(),
            current: self.current,
            selected: self.selected.clone(),
        }
    }

    fn record_change(&mut self, before: PlaylistSnapshot) {
        if before != self.snapshot() {
            self.undo.push(before);
            self.undo.truncate(100);
            self.redo.clear();
        }
    }

    fn restore(&mut self, snapshot: PlaylistSnapshot) {
        self.items = snapshot.items;
        self.current = snapshot.current.filter(|row| *row < self.items.len());
        self.selected = snapshot
            .selected
            .into_iter()
            .filter(|row| *row < self.items.len())
            .collect();
        self.anchor = self.selected.last().copied();
        self.drag = None;
        self.scroll = 0.0;
        self.probe_generation = self.probe_generation.wrapping_add(1);
        for item in &self.items {
            if !is_url(&item.path) && item.duration.is_none() {
                self.prober.probe(self.probe_generation, &item.path);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(snapshot);
        true
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
            status: if is_url(path) || Path::new(path).is_file() {
                MediaStatus::Ready
            } else {
                MediaStatus::Unavailable
            },
        }
    }

    pub fn set_items(&mut self, paths: &[String]) {
        self.probe_generation = self.probe_generation.wrapping_add(1);
        self.undo.clear();
        self.redo.clear();
        self.items = paths.iter().map(|path| self.make_item(path)).collect();
        for item in &self.items {
            if !is_url(&item.path) {
                self.prober.probe(self.probe_generation, &item.path);
            }
        }
        self.current = None;
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.scroll = 0.0;
    }

    pub fn add(&mut self, paths: &[String]) -> usize {
        let before_snapshot = self.snapshot();
        let first = self.items.len();
        for path in paths {
            let item = self.make_item(path);
            if !is_url(path) {
                self.prober.probe(self.probe_generation, path);
            }
            self.items.push(item);
        }
        self.record_change(before_snapshot);
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

    /// Scroll the list so the playing row stays visible after a file switch.
    /// Rows are positioned by their index within the filtered view, so the reveal
    /// computes the target from there; a row hidden by the filter is a no-op.
    pub fn reveal_current(&mut self) {
        let Some(row) = self.current else {
            return;
        };
        let visible_rows = self.visible_rows();
        let Some(position) = visible_rows.iter().position(|&visible| visible == row) else {
            return;
        };
        let target = reveal_scroll(self.scroll, position as f32 * ROW_H, self.view_h);
        let max = (visible_rows.len() as f32 * ROW_H - self.view_h).max(0.0);
        self.scroll = target.clamp(0.0, max);
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

    pub fn set_status(&mut self, path: &str, status: MediaStatus) {
        if let Some(item) = self.items.iter_mut().find(|item| item.path == path) {
            item.status = status;
        }
    }

    pub fn remove_selected(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let before_snapshot = self.snapshot();
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
        let changed = self.items.len() != before;
        if changed {
            self.record_change(before_snapshot);
        }
        changed
    }

    pub fn clear(&mut self) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let before_snapshot = self.snapshot();
        self.items.clear();
        self.current = None;
        self.selected.clear();
        self.anchor = None;
        self.drag = None;
        self.scroll = 0.0;
        self.record_change(before_snapshot);
        true
    }

    pub fn remove_missing(&mut self) -> bool {
        let before_snapshot = self.snapshot();
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
        let changed = self.items.len() != before;
        if changed {
            self.record_change(before_snapshot);
        }
        changed
    }

    pub fn remove_duplicates(&mut self) -> bool {
        let before_snapshot = self.snapshot();
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
        let changed = self.items.len() != before;
        if changed {
            self.record_change(before_snapshot);
        }
        changed
    }

    pub fn sort_by(&mut self, key: &str) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let before_snapshot = self.snapshot();
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
        self.record_change(before_snapshot);
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
    /// `thumbs` fills the rows' video previews: visible rows draw a real frame
    /// when generated and queue their generation once otherwise.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        ctx: &Context,
        thumbs: &mut Thumbnails,
    ) -> PlaylistOut {
        let mut out = PlaylistOut::default();
        self.prober.drain(self.probe_generation, &mut self.items);

        let colors = skin::active_colors();
        let painter = ui.painter_at(rect);
        // No opaque base fill: the drawer surface behind this widget is translucent
        // (skin drawer_fill), so the video stays faintly visible through the panel.
        let w = rect.width();

        // -- header -----------------------------------------------------------------------------
        let title_font = skin::font(16.0, false);
        let eyebrow_font = skin::font(9.0, false);
        painter.circle_filled(
            pos2(rect.left() + 13.0, rect.top() + 20.0),
            3.0,
            colors.accent,
        );
        painter.text(
            pos2(rect.left() + 23.0, rect.top() + 13.5),
            egui::Align2::LEFT_TOP,
            i18n::tr("UP NEXT"),
            eyebrow_font,
            colors.text_dim,
        );
        painter.text(
            pos2(rect.left() + 23.0, rect.top() + 27.0),
            egui::Align2::LEFT_CENTER,
            i18n::tr("Playlist"),
            title_font.clone(),
            colors.text_bright,
        );
        let count = self.items.len();
        let visible_rows = self.visible_rows();
        let visible_count = visible_rows.len();
        let count_text = format!("{count:02}");
        painter.text(
            pos2(rect.left() + 83.0, rect.top() + 27.0),
            egui::Align2::LEFT_CENTER,
            &count_text,
            skin::font(11.0, false),
            colors.text_dim,
        );
        // Summed duration of the probed items, next to the row count.
        let total: f64 = self.items.iter().filter_map(|item| item.duration).sum();
        if total > 0.0 {
            let count_w = skin::text_width(&painter, &count_text, &skin::font(11.0, false));
            painter.text(
                pos2(rect.left() + 83.0 + count_w + 10.0, rect.top() + 27.0),
                egui::Align2::LEFT_CENTER,
                fmt_time(total),
                skin::mono(9.5),
                colors.text_dim,
            );
        }
        let command_rect = Rect::from_min_size(
            pos2(rect.right() - 42.0, rect.top() + 10.0),
            vec2(30.0, 30.0),
        );
        let command = ui
            .interact(command_rect, ui.id().with("pl-command"), Sense::click())
            .on_hover_text(i18n::tr("Close"));
        painter.rect_filled(
            command_rect,
            7.0,
            if command.hovered() {
                colors.bg_hover
            } else {
                colors.bg
            },
        );
        painter.rect_stroke(
            command_rect,
            7.0,
            Stroke::new(1.0, colors.dialog_line),
            egui::StrokeKind::Outside,
        );
        painter.text(
            command_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            skin::font(13.0, false),
            colors.text_dim,
        );
        out.close = command.clicked();
        let search_rect = Rect::from_min_max(
            pos2(rect.left() + 12.0, rect.top() + 51.0),
            pos2(rect.right() - 12.0, rect.top() + HEADER_H - 10.0),
        );
        if search_rect.width() > 20.0 {
            let response = ui.put(
                search_rect,
                TextEdit::singleline(&mut self.query)
                    .desired_width(search_rect.width())
                    .hint_text(i18n::tr("Filter")),
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
            Stroke::new(1.0, colors.dialog_line),
        );
        // -- list -------------------------------------------------------------------------------
        let view = Rect::from_min_max(
            pos2(rect.left(), rect.top() + HEADER_H + 1.0),
            pos2(rect.right(), rect.bottom() - FOOTER_H),
        );
        self.view_h = view.height().max(ROW_H);
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
            if visible_rows.is_empty() {
                let title = if count == 0 {
                    i18n::tr("Playlist is empty")
                } else {
                    i18n::tr("No matching items")
                };
                let detail = if count == 0 {
                    i18n::tr("Add files, a folder, or a URL to start playing")
                } else {
                    i18n::tr("Try a different filter")
                };
                let center = view.center();
                let card =
                    Rect::from_center_size(center, vec2((view.width() - 22.0).max(0.0), 142.0));
                painter.rect_filled(card, 10.0, colors.bg);
                painter.rect_stroke(
                    card,
                    10.0,
                    Stroke::new(1.0, colors.dialog_line),
                    egui::StrokeKind::Outside,
                );
                painter.circle_filled(pos2(center.x, center.y - 37.0), 17.0, colors.select);
                skin::draw_icon(
                    &painter,
                    if count == 0 { "playlist" } else { "search" },
                    center.x,
                    center.y - 37.0,
                    colors.accent,
                    colors.select,
                );
                painter.text(
                    pos2(center.x, center.y - 5.0),
                    egui::Align2::CENTER_CENTER,
                    title,
                    skin::font(14.0, false),
                    colors.text_bright,
                );
                painter.text(
                    pos2(center.x, center.y + 23.0),
                    egui::Align2::CENTER_CENTER,
                    detail,
                    skin::font(11.0, false),
                    colors.text_dim,
                );
            } else {
                // A handful of first-use PNG decodes per frame keeps opening the
                // drawer hitch-free; the rest fill in over the next frames.
                let mut texture_budget = 6u32;
                self.paint_rows(
                    ui,
                    ctx,
                    view,
                    &visible_rows,
                    &mut out,
                    thumbs,
                    &mut texture_budget,
                );
            }
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);

        // Slim scroll indicator on the list's right edge when content overflows;
        // the thumb is draggable.
        if max_scroll > 0.0 && view.height() > 0.0 {
            let content_h = visible_count as f32 * ROW_H;
            let thumb_h = (view.height() * (view.height() / content_h)).clamp(24.0, view.height());
            let thumb_y = view.top() + (self.scroll / max_scroll) * (view.height() - thumb_h);
            let track_x = view.right() - 4.0;
            let track = Rect::from_min_max(
                pos2(track_x, view.top()),
                pos2(track_x + 2.0, view.bottom()),
            );
            let thumb = Rect::from_min_max(
                pos2(track_x - 0.5, thumb_y),
                pos2(track_x + 2.5, thumb_y + thumb_h),
            );
            let hit = thumb.expand(5.0);
            let resp = ui.interact(hit, Id::new("pl-scrollbar"), Sense::drag());
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let usable = (view.height() - thumb_h).max(1.0);
                let offset = self
                    .scroll_drag
                    .unwrap_or_else(|| (pos.y - thumb.top()).clamp(0.0, thumb_h));
                let frac = ((pos.y - offset - view.top()) / usable).clamp(0.0, 1.0);
                self.scroll = frac * max_scroll;
            }
            if resp.drag_started()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                self.scroll_drag = Some((pos.y - thumb.top()).clamp(0.0, thumb_h));
            }
            if resp.drag_stopped() {
                self.scroll_drag = None;
            }
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            painter.rect_filled(track, 1.0, colors.line);
            painter.rect_filled(thumb, 1.5, colors.accent);
        }

        // -- footer -----------------------------------------------------------------------------
        painter.line_segment(
            [
                pos2(rect.left(), rect.bottom() - FOOTER_H),
                pos2(rect.right(), rect.bottom() - FOOTER_H),
            ],
            Stroke::new(1.0, colors.line),
        );
        let footer_y = rect.bottom() - FOOTER_H + 1.0;
        let compact = w < 220.0;
        let right_button_w = if compact { 24.0 } else { 28.0 };
        let left_room = (w - right_button_w * 2.0 - 4.0).max(0.0);
        let action_w = ((left_room - 2.0) / 3.0).max(24.0);
        let add_rect =
            Rect::from_min_size(pos2(rect.left(), footer_y), vec2(action_w, FOOTER_H - 1.0));
        let del_rect = Rect::from_min_size(
            pos2(add_rect.right() + 1.0, footer_y),
            vec2(action_w, FOOTER_H - 1.0),
        );
        let sort_rect = Rect::from_min_size(
            pos2(del_rect.right() + 1.0, footer_y),
            vec2(action_w, FOOTER_H - 1.0),
        );
        if text_button(
            ui,
            ui.id().with("pl-add"),
            add_rect,
            if compact { "+" } else { i18n::tr("Add") },
        ) {
            self.open_menu(
                MenuKind::Add,
                add_rect.left_bottom() - vec2(0.0, FOOTER_H - 1.0),
            );
        }
        if text_button(
            ui,
            ui.id().with("pl-del"),
            del_rect,
            if compact { "⋯" } else { i18n::tr("More") },
        ) {
            self.open_menu(
                MenuKind::Del,
                del_rect.left_bottom() - vec2(0.0, FOOTER_H - 1.0),
            );
        }
        if text_button(
            ui,
            ui.id().with("pl-sort"),
            sort_rect,
            if compact { "↕" } else { i18n::tr("Sort") },
        ) {
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
            pos2(rect.right() - right_button_w * 2.0, footer_y),
            vec2(right_button_w, FOOTER_H - 1.0),
        );
        let shuffle_rect = Rect::from_min_size(
            pos2(rect.right() - right_button_w, footer_y),
            vec2(right_button_w, FOOTER_H - 1.0),
        );
        out.repeat = IconBtn::new("repeat")
            .tip(i18n::tr("Repeat"))
            .color(colors.icon_dim)
            .checked(self.repeat_mode != "off")
            .ui(ui, ui.id().with("pl-repeat"), repeat_rect)
            .primary;
        out.shuffle = IconBtn::new("shuffle")
            .tip(i18n::tr("Shuffle"))
            .color(colors.icon_dim)
            .checked(self.shuffle)
            .ui(ui, ui.id().with("pl-shuffle"), shuffle_rect)
            .primary;

        // -- menu -------------------------------------------------------------------------------
        if let Some(action) = self.paint_menu(ui) {
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
                Action::PlaylistShowInFolder(row) => out.show_in_folder = Some(row),
                _ => {}
            }
        }

        out
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_rows(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &Context,
        view: Rect,
        visible_rows: &[usize],
        out: &mut PlaylistOut,
        thumbs: &mut Thumbnails,
        texture_budget: &mut u32,
    ) {
        let colors = skin::active_colors();
        let painter = ui.painter_at(view);
        let first = (self.scroll / ROW_H).floor() as usize;
        let visible = ((view.height() / ROW_H).ceil() as usize) + 1;
        let last = (first + visible).min(visible_rows.len());
        let font_id = skin::font(12.0, false);
        let meta_font = skin::font(9.0, false);
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
            if current {
                painter.rect_filled(
                    Rect::from_min_size(row_rect.left_top(), vec2(3.0, row_rect.height())),
                    0.0,
                    colors.accent,
                );
                // Thin playback-progress line along the bottom of the playing row.
                if let Some(fraction) = self.progress {
                    let track = Rect::from_min_max(
                        pos2(row_rect.left() + 10.0, row_rect.bottom() - 4.0),
                        pos2(row_rect.right() - 10.0, row_rect.bottom() - 2.0),
                    );
                    painter.rect_filled(track, 1.0, colors.line);
                    painter.rect_filled(
                        Rect::from_min_max(
                            track.min,
                            pos2(track.left() + track.width() * fraction as f32, track.max.y),
                        ),
                        1.0,
                        colors.accent,
                    );
                }
            }
            if selected {
                painter.rect_stroke(
                    row_rect.shrink(1.0),
                    2.0,
                    Stroke::new(
                        1.0,
                        if self.has_focus {
                            colors.focus
                        } else {
                            colors.text_dim
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
            }

            // Landscape preview slot: the playing file shows a real decoded
            // frame when its thumbnails are ready; every other row gets a tinted
            // placeholder so the list reads as a filmstrip.
            let thumb = Rect::from_min_size(
                pos2(row_rect.left() + 8.0, row_rect.center().y - 16.0),
                vec2(56.0, 32.0),
            );
            let thumb_palette = [
                egui::Color32::from_rgb(0x24, 0x3d, 0x46),
                egui::Color32::from_rgb(0x6a, 0x4b, 0x65),
                egui::Color32::from_rgb(0x5b, 0x4e, 0x82),
                egui::Color32::from_rgb(0x3f, 0x6a, 0x66),
                egui::Color32::from_rgb(0x6d, 0x63, 0x45),
            ];
            let thumb_fill = if current {
                // Video letterbox: dark in both themes, matching the player surface.
                colors.video_bg
            } else if selected {
                colors.button_hover
            } else {
                thumb_palette[row % thumb_palette.len()]
            };
            painter.rect_filled(thumb, 5.0, thumb_fill);
            let mut drew_frame = false;
            if !is_url(&self.items[row].path)
                && let Some(item_duration) = self.items[row].duration
                && item_duration > 1.0
            {
                let path = &self.items[row].path;
                // Budgeted decode: first-use PNG uploads stop after a few per
                // frame so a freshly opened drawer never hitches on one frame.
                if *texture_budget > 0
                    && let Some(handle) =
                        thumbs.texture(ctx, path, item_duration * 0.5, item_duration)
                {
                    // Aspect-fit the frame inside the slot (letterboxed by the fill).
                    let size = handle.size_vec2();
                    let scale = (thumb.width() / size.x).min(thumb.height() / size.y);
                    let draw_rect = Rect::from_center_size(
                        thumb.center(),
                        vec2(size.x * scale, size.y * scale),
                    );
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    painter.image(handle.id(), draw_rect, uv, Color32::WHITE);
                    drew_frame = true;
                    *texture_budget -= 1;
                } else {
                    // Queue once; the worker fills it while the row stays visible.
                    thumbs.ensure(path, item_duration);
                }
            }
            painter.rect_stroke(
                thumb,
                5.0,
                Stroke::new(1.0, colors.line),
                egui::StrokeKind::Inside,
            );
            if current && !drew_frame {
                skin::draw_icon(
                    &painter,
                    "play",
                    thumb.center().x,
                    thumb.center().y,
                    colors.accent,
                    thumb_fill,
                );
            } else if !current {
                let item = &self.items[row];
                let (label, label_color) = match &item.status {
                    MediaStatus::Loading => (i18n::tr("LOAD"), colors.accent),
                    MediaStatus::Ready => {
                        if is_url(&item.path) {
                            (i18n::tr("URL"), colors.text_a8)
                        } else {
                            (i18n::tr("FILE"), colors.text_a8)
                        }
                    }
                    MediaStatus::Failed(_) => (i18n::tr("ERROR"), colors.danger),
                    MediaStatus::Unavailable => (i18n::tr("MISSING"), colors.danger),
                };
                painter.text(
                    thumb.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    meta_font.clone(),
                    label_color,
                );
            }

            let mut right = row_rect.right() - 8.0;
            if let Some(duration) = self.items[row].duration {
                let text = fmt_time(duration);
                let tw = skin::text_width(&painter, &text, &font_id);
                painter.text(
                    pos2(right - tw, row_rect.top() + 14.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    meta_font.clone(),
                    if current {
                        colors.accent
                    } else {
                        colors.text_dim
                    },
                );
                right = (right - tw - 10.0).max(row_rect.left() + 10.0);
            }
            let title_x = row_rect.left() + 74.0;
            let room = (right - title_x).max(10.0);
            let title = skin::elide(&painter, &self.items[row].title, &font_id, room);
            if title.len() < self.items[row].title.len() {
                // Truncated title: let the hover reveal the full name.
                response
                    .clone()
                    .on_hover_text(self.items[row].title.clone());
            }
            painter.text(
                pos2(title_x, row_rect.top() + 27.0),
                egui::Align2::LEFT_TOP,
                title,
                font_id.clone(),
                if current { colors.accent } else { colors.text },
            );
            painter.text(
                pos2(title_x, row_rect.top() + 43.0),
                egui::Align2::LEFT_TOP,
                match &self.items[row].status {
                    MediaStatus::Loading => i18n::tr("Loading…"),
                    MediaStatus::Ready if self.items[row].duration.is_none() => {
                        i18n::tr("Duration unavailable")
                    }
                    MediaStatus::Ready if is_url(&self.items[row].path) => {
                        i18n::tr("Network media")
                    }
                    MediaStatus::Ready => i18n::tr("Local media"),
                    MediaStatus::Failed(_) => i18n::tr("Playback failed"),
                    MediaStatus::Unavailable => i18n::tr("Unavailable"),
                },
                meta_font.clone(),
                match &self.items[row].status {
                    MediaStatus::Failed(_) | MediaStatus::Unavailable => colors.danger,
                    MediaStatus::Loading => colors.accent,
                    MediaStatus::Ready => colors.text_dim,
                },
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
        let before_snapshot = self.snapshot();
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
        self.record_change(before_snapshot);
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
                    && !modifiers.command
                {
                    match key {
                        egui::Key::ArrowUp => next = Some(current - 1),
                        egui::Key::ArrowDown => next = Some(current + 1),
                        egui::Key::Home => next = Some(0),
                        egui::Key::End => next = Some(count - 1),
                        egui::Key::PageUp => next = Some(current - 10),
                        egui::Key::PageDown => next = Some(current + 10),
                        egui::Key::Enter => {
                            if let Some(row) = self.selected.last().copied()
                                && row < self.items.len()
                            {
                                out.play = Some(row);
                            }
                        }
                        egui::Key::Delete => out.changed |= self.remove_selected(),
                        _ => {}
                    }
                }
            }
        });
        if let Some(next) = next.filter(|_| count > 0) {
            let next = next.clamp(0, count - 1) as usize;
            self.select(visible_rows[next], false, false);
            self.scroll = reveal_scroll(self.scroll, next as f32 * ROW_H, self.view_h)
                .clamp(0.0, (count as f32 * ROW_H - self.view_h).max(0.0));
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
                Entry::item(i18n::tr("Add Files..."), Action::OpenFiles { append: true }),
                Entry::item(
                    i18n::tr("Add Folder..."),
                    Action::OpenFolder { append: true },
                ),
                Entry::item(i18n::tr("Add URL..."), Action::OpenUrl),
            ],
            MenuKind::Del => vec![
                Entry::item(i18n::tr("Remove Selected"), Action::PlaylistRemove)
                    .enabled(!self.selected.is_empty()),
                Entry::item(
                    i18n::tr("Remove Missing Files"),
                    Action::PlaylistRemoveMissing,
                ),
                Entry::item(
                    i18n::tr("Remove Duplicates"),
                    Action::PlaylistRemoveDuplicates,
                ),
                Entry::Sep,
                Entry::item(i18n::tr("Clear Playlist"), Action::PlaylistClear),
            ],
            MenuKind::Sort => vec![
                Entry::item(i18n::tr("By Name"), Action::PlaylistSort("name")),
                Entry::item(i18n::tr("By Path"), Action::PlaylistSort("path")),
                Entry::item(i18n::tr("By Duration"), Action::PlaylistSort("duration")),
                Entry::Sep,
                Entry::item(i18n::tr("Reverse"), Action::PlaylistSort("reverse")),
                Entry::item(i18n::tr("Randomize"), Action::PlaylistSort("random")),
            ],
            MenuKind::Context(row) => {
                let has_row = row < self.items.len();
                let mut entries = vec![
                    Entry::item(i18n::tr("Play"), Action::PlayRow(row)).enabled(has_row),
                    Entry::Sep,
                    Entry::item(i18n::tr("Add Files..."), Action::OpenFiles { append: true }),
                    Entry::item(
                        i18n::tr("Add Folder..."),
                        Action::OpenFolder { append: true },
                    ),
                    Entry::item(i18n::tr("Add URL..."), Action::OpenUrl),
                    Entry::Sep,
                    Entry::item(i18n::tr("Remove"), Action::PlaylistRemove)
                        .enabled(!self.selected.is_empty()),
                    Entry::item(
                        i18n::tr("Remove Missing Files"),
                        Action::PlaylistRemoveMissing,
                    ),
                    Entry::item(
                        i18n::tr("Remove Duplicates"),
                        Action::PlaylistRemoveDuplicates,
                    ),
                    Entry::item(i18n::tr("Clear Playlist"), Action::PlaylistClear),
                    Entry::Sep,
                    Entry::sub(
                        i18n::tr("Sort By"),
                        vec![
                            Entry::item(i18n::tr("Name"), Action::PlaylistSort("name")),
                            Entry::item(i18n::tr("Path"), Action::PlaylistSort("path")),
                            Entry::item(i18n::tr("Duration"), Action::PlaylistSort("duration")),
                            Entry::Sep,
                            Entry::item(i18n::tr("Reverse Order"), Action::PlaylistSort("reverse")),
                        ],
                    ),
                    Entry::Sep,
                    Entry::item(i18n::tr("Copy Path"), Action::PlaylistCopyPath(row))
                        .enabled(has_row),
                ];
                let local = has_row && self.path(row).is_some_and(|path| !is_url(path));
                entries.push(
                    Entry::item(
                        i18n::tr("Show in File Manager"),
                        Action::PlaylistShowInFolder(row),
                    )
                    .enabled(local),
                );
                entries
            }
        }
    }

    fn paint_menu(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let kind = self.menu_kind?;
        if !self.menu.is_open() {
            self.menu_kind = None;
            return None;
        }
        let entries = self.menu_entries(kind);
        let action = menus::show(ui.ctx(), &mut self.menu, "playlist", &entries);
        if action.is_none() && !self.menu.is_open() {
            self.menu_kind = None;
        }
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
        Playlist {
            items: paths
                .iter()
                .map(|path| Item {
                    path: (*path).to_string(),
                    title: title_of(path),
                    duration: None,
                    status: MediaStatus::Ready,
                })
                .collect(),
            ..Default::default()
        }
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
    fn playlist_remove_can_be_undone_and_redone() {
        let mut playlist = playlist_with_paths(&["a.mkv", "b.mkv", "c.mkv"]);
        playlist.selected = vec![1];

        assert!(playlist.remove_selected());
        assert_eq!(playlist.paths(), vec!["a.mkv", "c.mkv"]);
        assert!(playlist.can_undo());
        assert!(playlist.undo());
        assert_eq!(playlist.paths(), vec!["a.mkv", "b.mkv", "c.mkv"]);
        assert!(playlist.can_redo());
        assert!(playlist.redo());
        assert_eq!(playlist.paths(), vec!["a.mkv", "c.mkv"]);
    }

    #[test]
    fn new_playlist_mutation_clears_redo() {
        let mut playlist = playlist_with_paths(&["b.mkv", "a.mkv"]);
        assert!(playlist.sort_by("name"));
        assert!(playlist.undo());
        assert!(playlist.sort_by("reverse"));
        assert!(!playlist.can_redo());
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

    #[test]
    fn progress_fraction_is_finite_or_nothing_and_clamped() {
        assert_eq!(progress_fraction(0.5), Some(0.5));
        assert_eq!(progress_fraction(1.5), Some(1.0));
        assert_eq!(progress_fraction(-0.2), Some(0.0));
        assert_eq!(progress_fraction(f64::NAN), None);
        assert_eq!(progress_fraction(f64::INFINITY), None);
    }

    #[test]
    fn reveal_scroll_keeps_the_row_inside_the_view() {
        let row = |index: usize| index as f32 * ROW_H;
        // Visible row: scroll untouched.
        assert_eq!(reveal_scroll(100.0, row(3), 5.0 * ROW_H), 100.0);
        // Row above the view: scroll up to the row top.
        assert_eq!(reveal_scroll(1000.0, row(2), 5.0 * ROW_H), row(2));
        // Row below the view: scroll down just enough to show it.
        let bottom = reveal_scroll(0.0, row(10), 5.0 * ROW_H);
        assert!((bottom - (row(10) + ROW_H - 5.0 * ROW_H)).abs() < 0.01);
        // Never negative.
        assert_eq!(reveal_scroll(0.0, 0.0, 0.0), 0.0);
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
