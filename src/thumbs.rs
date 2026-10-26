//! Seekbar hover thumbnails and playlist row previews: a background worker decodes
//! preview frames with ffmpeg (one process per frame, PNG to a pipe) and the UI turns
//! them into egui textures on demand.
//!
//! Two job lanes feed one worker: the *priority* lane carries the currently playing
//! file (its seek-bar previews must appear promptly) and is drained before the FIFO
//! lane that fills playlist row previews as they scroll into view.

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};

/// Preview frames generated per file.
const THUMB_COUNT: usize = 10;
/// Preview width in pixels; the height follows the video aspect.
const THUMB_W: i32 = 168;
/// Per-frame decode budget before the worker kills ffmpeg.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// Files whose previews are kept in memory (each set is ten small PNGs, so two
/// dozen files stay well under ~20 MB while comfortably covering a screenful
/// of playlist rows without regenerating on every scroll).
const CACHE_FILES: usize = 24;
/// Cap on the `requested` dedupe set; clearing it merely allows regeneration.
const MAX_REQUESTED: usize = 8192;

struct Job {
    path: String,
    duration: f64,
}

/// ffmpeg-backed preview generator; degrades to a no-op when ffmpeg is absent.
pub struct Thumbnails {
    /// FIFO lane for playlist row previews.
    jobs: Option<Sender<Job>>,
    /// Priority lane: the currently playing file (seek-bar previews).
    priority: Option<Sender<Job>>,
    /// Wakeup channel shared by both lanes; without it the worker could block
    /// on the FIFO lane while a priority job waits on the other channel.
    wake: Option<Sender<()>>,
    results: Receiver<(String, Vec<Option<Vec<u8>>>)>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Decoded PNG bytes per file, in playback order.
    frames: HashMap<String, Vec<Option<Vec<u8>>>>,
    /// Insertion order of `frames` keys for the small LRU.
    order: Vec<String>,
    /// GPU textures created from `frames`, lazily, on the UI thread.
    textures: HashMap<String, Vec<Option<TextureHandle>>>,
    /// Paths already queued or generated, so rows are queued exactly once.
    requested: HashSet<String>,
    /// The file the seek-bar pipeline owns; row queuing skips it.
    active: String,
    // -- embedded cover art (audio files) ---------------------------------
    cover_jobs: Option<Sender<String>>,
    cover_results: Receiver<(String, Option<Vec<u8>>)>,
    cover_worker: Option<JoinHandle<()>>,
    cover_stop: Arc<AtomicBool>,
    /// Extracted cover PNG per file (one 256 px frame).
    covers: HashMap<String, Vec<u8>>,
    /// Paths whose extraction was attempted (success or not).
    covers_tried: HashSet<String>,
    cover_textures: HashMap<String, TextureHandle>,
}

impl Thumbnails {
    pub fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (priority_tx, priority_rx) = channel::<Job>();
        let (wake_tx, wake_rx) = channel::<()>();
        let (result_tx, result_rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = which("ffmpeg").and_then(|ffmpeg| {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("thumbs".into())
                .spawn(move || worker_loop(ffmpeg, job_rx, priority_rx, wake_rx, result_tx, stop))
                .ok()
        });
        // Cover-art extraction thread (audio files; single small ffmpeg pass).
        let (cover_tx, cover_rx) = channel::<String>();
        let (cover_result_tx, cover_result_rx) = channel();
        let cover_stop = Arc::new(AtomicBool::new(false));
        let cover_worker = which("ffmpeg").and_then(|ffmpeg| {
            let cover_stop = Arc::clone(&cover_stop);
            std::thread::Builder::new()
                .name("covers".into())
                .spawn(move || {
                    while let Ok(path) = cover_rx.recv() {
                        if cover_stop.load(Ordering::Acquire) {
                            break;
                        }
                        let art = extract_cover(&ffmpeg, &path, &cover_stop);
                        if cover_result_tx.send((path, art)).is_err() {
                            break;
                        }
                    }
                })
                .ok()
        });
        Self {
            jobs: worker.is_some().then_some(job_tx),
            priority: worker.is_some().then_some(priority_tx),
            wake: worker.is_some().then_some(wake_tx),
            results: result_rx,
            stop,
            worker,
            frames: HashMap::new(),
            order: Vec::new(),
            textures: HashMap::new(),
            requested: HashSet::new(),
            active: String::new(),
            cover_jobs: cover_worker.is_some().then_some(cover_tx),
            cover_results: cover_result_rx,
            cover_worker,
            cover_stop,
            covers: HashMap::new(),
            covers_tried: HashSet::new(),
            cover_textures: HashMap::new(),
        }
    }

    /// True when previews can be generated (ffmpeg found on this machine).
    pub fn available(&self) -> bool {
        self.priority.is_some()
    }

    /// Queue preview generation for the currently playing file; its seek-bar
    /// previews take priority over playlist row work.
    pub fn queue(&mut self, path: &str, duration: f64) {
        if let Some(priority) = &self.priority
            && duration.is_finite()
            && duration > 1.0
        {
            self.active = path.to_string();
            self.requested.insert(path.to_string());
            // Frames still in cache decode on demand via `texture()` —
            // re-queueing would redo the ffmpeg passes for nothing.
            if self.frames.contains_key(path) {
                return;
            }
            let _ = priority.send(Job {
                path: path.to_string(),
                duration,
            });
            if let Some(wake) = &self.wake {
                let _ = wake.send(());
            }
        }
    }

    /// Queue preview generation for a playlist row, once. Rows call this as they
    /// scroll into view; the dedupe set keeps repeat calls free.
    pub fn ensure(&mut self, path: &str, duration: f64) {
        let Some(jobs) = &self.jobs else {
            return;
        };
        if path == self.active || !duration.is_finite() || duration <= 1.0 {
            return;
        }
        if self.requested.len() >= MAX_REQUESTED {
            self.requested.clear();
        }
        if !self.requested.insert(path.to_string()) {
            return;
        }
        let _ = jobs.send(Job {
            path: path.to_string(),
            duration,
        });
        if let Some(wake) = &self.wake {
            let _ = wake.send(());
        }
    }

    /// Take finished preview sets from the worker. Call once per frame.
    pub fn drain(&mut self) {
        while let Ok((path, frames)) = self.results.try_recv() {
            self.remember(path, frames);
        }
        while let Ok((path, art)) = self.cover_results.try_recv() {
            self.covers_tried.insert(path.clone());
            if let Some(png) = art.filter(|png| !png.is_empty()) {
                self.covers.insert(path, png);
            }
        }
    }

    fn remember(&mut self, path: String, frames: Vec<Option<Vec<u8>>>) {
        self.order.retain(|p| p != &path);
        self.order.push(path.clone());
        while self.order.len() > CACHE_FILES {
            let evict = self.order.remove(0);
            self.frames.remove(&evict);
            self.textures.remove(&evict);
            // Evicted files may be previewed again when their row reappears.
            self.requested.remove(&evict);
        }
        // Fresh bytes invalidate any textures built earlier.
        self.textures.remove(&path);
        self.frames.insert(path.clone(), frames);
        // A generated file counts as requested; it never re-queues on its own.
        self.requested.insert(path);
    }

    /// Texture for the preview slot nearest to `seconds`, decoding and
    /// uploading on first use. Must run on the UI thread.
    pub fn texture(
        &mut self,
        ctx: &Context,
        path: &str,
        seconds: f64,
        duration: f64,
    ) -> Option<(&TextureHandle, bool)> {
        let frames = self.frames.get(path)?;
        if frames.is_empty() || duration <= 0.0 || !seconds.is_finite() {
            return None;
        }
        let slot = ((seconds / duration).clamp(0.0, 0.999) * THUMB_COUNT as f64) as usize;
        let slot = slot.min(frames.len() - 1);
        if frames.get(slot)?.is_none() {
            return None;
        }
        let mut fresh = false;
        if self
            .textures
            .get(path)
            .and_then(|slots| slots.get(slot))
            .is_none_or(|handle| handle.is_none())
        {
            let bytes = frames[slot].clone()?;
            let image = decode_png(&bytes)?;
            let handle = ctx.load_texture(
                format!("thumb-{path}-{slot}"),
                image,
                TextureOptions::LINEAR,
            );
            let slots = self
                .textures
                .entry(path.to_string())
                .or_insert_with(|| vec![None; frames.len()]);
            if slot < slots.len() {
                slots[slot] = Some(handle);
            }
            fresh = true;
        }
        let handle = self.textures.get(path)?.get(slot)?.as_ref()?;
        Some((handle, fresh))
    }

    /// Embedded cover-art texture for an audio file. Queues the extraction
    /// once; returns `Some` as soon as the decoded art is uploaded. Returns
    /// the handle plus a freshness flag (first upload this call).
    pub fn cover(&mut self, ctx: &Context, path: &str) -> Option<(&TextureHandle, bool)> {
        if self.covers_tried.contains(path) {
            if !self.covers.contains_key(path) {
                return None;
            }
            if !self.cover_textures.contains_key(path) {
                let png = self.covers.get(path)?;
                let image = decode_png(png)?;
                let handle =
                    ctx.load_texture(format!("cover-{path}"), image, TextureOptions::LINEAR);
                self.cover_textures.insert(path.to_string(), handle);
            }
            return self.cover_textures.get(path).map(|handle| (handle, false));
        }
        if let Some(jobs) = &self.cover_jobs
            && self.covers_tried.insert(path.to_string())
        {
            let _ = jobs.send(path.to_string());
        }
        None
    }

    /// Cheap pre-check: is a texture already uploaded for this slot? Lets
    /// callers draw cached previews without charging a decode budget.
    pub fn peek_ready(&self, path: &str, seconds: f64, duration: f64) -> bool {
        self.frames.get(path).is_some_and(|frames| {
            if frames.is_empty() || duration <= 0.0 || !seconds.is_finite() {
                return false;
            }
            let slot = ((seconds / duration).clamp(0.0, 0.999) * THUMB_COUNT as f64) as usize;
            let slot = slot.min(frames.len() - 1);
            self.textures
                .get(path)
                .and_then(|slots| slots.get(slot))
                .is_some_and(|handle| handle.is_some())
        })
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Dropping both senders wakes the worker from `recv()`.
        self.jobs = None;
        self.priority = None;
        self.wake = None;
        self.cover_jobs = None;
        self.cover_stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(worker) = self.cover_worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    ffmpeg: String,
    jobs: Receiver<Job>,
    priority: Receiver<Job>,
    wake: Receiver<()>,
    results: Sender<(String, Vec<Option<Vec<u8>>>)>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        // The priority lane (currently playing file) is drained first so
        // seek-bar previews never queue behind a batch of row jobs, and it is
        // coalesced to its newest entry: rapid file switches must not make the
        // current file wait behind stale decodes.
        let job = loop {
            match priority.try_recv() {
                Ok(first) => {
                    let mut newest = first;
                    while let Ok(newer) = priority.try_recv() {
                        newest = newer;
                    }
                    break Some(newest);
                }
                Err(TryRecvError::Disconnected) => {}
                Err(TryRecvError::Empty) => {}
            }
            match jobs.try_recv() {
                Ok(job) => break Some(job),
                Err(TryRecvError::Disconnected) => break None,
                Err(TryRecvError::Empty) => {}
            }
            if wake.recv().is_err() {
                break None;
            }
        };
        let Some(job) = job else { break };
        let frames = preview_frames(&ffmpeg, &job.path, job.duration, &stop);
        if results.send((job.path, frames)).is_err() {
            break;
        }
    }
}

/// Decode `THUMB_COUNT` frames spread evenly across the file.
fn preview_frames(
    ffmpeg: &str,
    path: &str,
    duration: f64,
    stop: &AtomicBool,
) -> Vec<Option<Vec<u8>>> {
    (0..THUMB_COUNT)
        .map(|index| {
            let at = duration * (index as f64 + 0.5) / THUMB_COUNT as f64;
            decode_frame(ffmpeg, path, at, stop)
        })
        .collect()
}

fn decode_frame(ffmpeg: &str, path: &str, at: f64, stop: &AtomicBool) -> Option<Vec<u8>> {
    let mut child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-ss",
            &format!("{at:.3}"),
            "-i",
            path,
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={THUMB_W}:-2"),
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + FRAME_TIMEOUT;
    loop {
        if stop.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                return (status.success() && !output.stdout.is_empty()).then_some(output.stdout);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn decode_png(bytes: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        rgba.as_raw(),
    ))
}

/// Pull the first video stream of an audio file (its embedded cover) as a
/// small PNG. Fails cleanly for audio without attached art.
fn extract_cover(ffmpeg: &str, path: &str, stop: &AtomicBool) -> Option<Vec<u8>> {
    let mut child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            path,
            "-frames:v",
            "1",
            "-an",
            "-sn",
            "-vf",
            "scale=256:-2",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + FRAME_TIMEOUT;
    loop {
        if stop.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                return (status.success() && !output.stdout.is_empty()).then_some(output.stdout);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Locate an executable on PATH.
fn which(program: &str) -> Option<String> {
    crate::util::which(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_queues_each_row_once_and_skips_the_active_file() {
        let mut thumbs = Thumbnails::new();
        // No ffmpeg-bound worker in tests (which() may find one; jobs would be
        // real). Assert on the dedupe bookkeeping instead of the channel.
        thumbs.requested.clear();
        thumbs.ensure("/tmp/a.mp4", 30.0);
        let queued_once = thumbs.requested.contains("/tmp/a.mp4");
        thumbs.ensure("/tmp/a.mp4", 30.0);
        assert_eq!(thumbs.requested.contains("/tmp/a.mp4"), queued_once);
        // The active (playing) file belongs to the priority pipeline.
        thumbs.active = "/tmp/active.mp4".into();
        thumbs.ensure("/tmp/active.mp4", 30.0);
        assert!(!thumbs.requested.contains("/tmp/active.mp4"));
        // Non-finite or tiny durations are never queued.
        thumbs.ensure("/tmp/b.mp4", f64::NAN);
        thumbs.ensure("/tmp/b.mp4", 0.5);
        assert!(!thumbs.requested.contains("/tmp/b.mp4"));
    }

    #[test]
    fn eviction_forgets_the_request_so_rows_can_regenerate() {
        let frames = || (0..THUMB_COUNT).map(|i| Some(vec![i as u8; 4])).collect();
        let mut thumbs = Thumbnails::new();
        thumbs.remember("/tmp/old.mp4".into(), frames());
        assert!(thumbs.requested.contains("/tmp/old.mp4"));
        for i in 0..CACHE_FILES {
            thumbs.remember(format!("/tmp/fill-{i}.mp4"), frames());
        }
        assert!(!thumbs.frames.contains_key("/tmp/old.mp4"));
        assert!(!thumbs.requested.contains("/tmp/old.mp4"));
    }
}
