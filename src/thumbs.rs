//! Seekbar hover thumbnails: a background worker decodes a handful of preview
//! frames with ffmpeg (one process per frame, PNG to a pipe) and the UI turns
//! them into egui textures on demand.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};

/// Preview frames generated per file.
const THUMB_COUNT: usize = 10;
/// Preview width in pixels; the height follows the video aspect.
const THUMB_W: i32 = 168;
/// Per-frame decode budget before the worker kills ffmpeg.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// Files whose previews are kept in memory (each is ~10 small PNGs).
const CACHE_FILES: usize = 4;

struct Job {
    path: String,
    duration: f64,
}

/// ffmpeg-backed preview generator; degrades to a no-op when ffmpeg is absent.
pub struct Thumbnails {
    jobs: Option<Sender<Job>>,
    results: Receiver<(String, Vec<Option<Vec<u8>>>)>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Decoded PNG bytes per file, in playback order.
    frames: HashMap<String, Vec<Option<Vec<u8>>>>,
    /// Insertion order of `frames` keys for the small LRU.
    order: Vec<String>,
    /// GPU textures created from `frames`, lazily, on the UI thread.
    textures: HashMap<String, Vec<Option<TextureHandle>>>,
}

impl Thumbnails {
    pub fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (result_tx, result_rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = which("ffmpeg").and_then(|ffmpeg| {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("thumbs".into())
                .spawn(move || worker_loop(ffmpeg, job_rx, result_tx, stop))
                .ok()
        });
        Self {
            jobs: worker.is_some().then_some(job_tx),
            results: result_rx,
            stop,
            worker,
            frames: HashMap::new(),
            order: Vec::new(),
            textures: HashMap::new(),
        }
    }

    /// True when previews can be generated (ffmpeg found on this machine).
    pub fn available(&self) -> bool {
        self.jobs.is_some()
    }

    /// Queue preview generation for a file; replaces any pending request.
    pub fn queue(&mut self, path: &str, duration: f64) {
        if let Some(jobs) = &self.jobs
            && duration.is_finite()
            && duration > 1.0
        {
            // A pending job for the same path is superseded by the new one;
            // the worker skips stale duplicates itself.
            let _ = jobs.send(Job {
                path: path.to_string(),
                duration,
            });
        }
    }

    /// Take finished preview sets from the worker. Call once per frame.
    pub fn drain(&mut self) {
        while let Ok((path, frames)) = self.results.try_recv() {
            self.remember(path, frames);
        }
    }

    fn remember(&mut self, path: String, frames: Vec<Option<Vec<u8>>>) {
        self.order.retain(|p| p != &path);
        self.order.push(path.clone());
        while self.order.len() > CACHE_FILES {
            let evict = self.order.remove(0);
            self.frames.remove(&evict);
            self.textures.remove(&evict);
        }
        // Fresh bytes invalidate any textures built earlier.
        self.textures.remove(&path);
        self.frames.insert(path, frames);
    }

    /// Texture for the preview slot nearest to `seconds`, decoding and
    /// uploading on first use. Must run on the UI thread.
    pub fn texture(
        &mut self,
        ctx: &Context,
        path: &str,
        seconds: f64,
        duration: f64,
    ) -> Option<&TextureHandle> {
        let frames = self.frames.get(path)?;
        if frames.is_empty() || duration <= 0.0 || !seconds.is_finite() {
            return None;
        }
        let slot = ((seconds / duration).clamp(0.0, 0.999) * THUMB_COUNT as f64) as usize;
        let slot = slot.min(frames.len() - 1);
        if frames.get(slot)?.is_none() {
            return None;
        }
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
        }
        self.textures.get(path)?.get(slot)?.as_ref()
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // A closed sender wakes the worker from `recv()`.
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    ffmpeg: String,
    jobs: Receiver<Job>,
    results: Sender<(String, Vec<Option<Vec<u8>>>)>,
    stop: Arc<AtomicBool>,
) {
    let mut pending: Option<Job> = None;
    while !stop.load(Ordering::Acquire) {
        // Coalesce queued jobs: only the newest request matters.
        let job = match pending.take().or_else(|| jobs.recv().ok()) {
            Some(job) => job,
            None => break,
        };
        while let Ok(newer) = jobs.try_recv() {
            pending = Some(newer);
        }
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

/// Locate an executable on PATH.
fn which(program: &str) -> Option<String> {
    crate::util::which(program)
}
