//! MacPlayer — a native Rust media player built on egui + libmpv.
// The no-unwrap/no-panic lints are a production-code rule; unit tests assert on
// Result and Option everywhere, so relax them for the test harness builds.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod dialogs;
mod engine;
mod ffi;
mod i18n;
mod logging;
mod menus;
mod player;
mod playlist;
mod render;
mod services;
mod settings;
mod skin;
mod thumbs;
mod util;
mod value;
mod widgets;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use player::Player;

fn main() {
    // Crash reports land in the diagnostics log, next to the engine's own errors.
    std::panic::set_hook(Box::new(|info| {
        let message = format!("panic: {info}");
        eprintln!("{message}");
        logging::write(&message);
    }));

    let cli = Cli::parse(std::env::args().skip(1));
    if cli.version {
        println!("MacPlayer {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if cli.help {
        Cli::print_help();
        return;
    }

    let args: Vec<String> = cli
        .paths
        .iter()
        .map(|path| normalize_cli_path(path))
        .collect();
    let settings = settings::Settings::load();

    // Single instance: if another copy is already running, hand the files over and exit.
    if !cli.new_instance && send_to_running(&args) {
        return;
    }
    let (ipc_server, ipc_rx): (Option<InstanceServer>, Receiver<Vec<String>>) = if cli.new_instance
    {
        let (_tx, rx) = channel();
        (None, rx)
    } else {
        match InstanceServer::start() {
            Ok((server, rx)) => (Some(server), rx),
            Err(error) => {
                eprintln!("could not start the single-instance listener: {error}");
                logging::write(&format!("single-instance listener: {error}"));
                return;
            }
        }
    };

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("MacPlayer")
        .with_decorations(!cli.fullscreen)
        .with_inner_size([settings.window_w as f32, settings.window_h as f32])
        // The video fills the window and every control floats above it, so the window
        // only needs to stay above the smallest comfortable player surface.
        .with_min_inner_size([480.0, 270.0])
        .with_fullscreen(cli.fullscreen);
    if cli.fullscreen {
        viewport = viewport.with_monitor(settings.fullscreen_monitor);
    }
    if let (Some(x), Some(y)) = (settings.window_x, settings.window_y)
        && x.is_finite()
        && y.is_finite()
    {
        viewport = viewport.with_position([x as f32, y as f32]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "MacPlayer",
        options,
        Box::new(move |creation_context| {
            skin::apply(&creation_context.egui_ctx);
            Ok(Box::new(Player::new(
                creation_context.egui_ctx.clone(),
                args.clone(),
                ipc_rx,
                settings.clone(),
                cli.verbose,
                cli.fullscreen,
            )))
        }),
    ) {
        eprintln!("eframe error: {error}");
        logging::write(&format!("eframe: {error}"));
    }
    drop(ipc_server);
}

#[derive(Debug, Default)]
struct Cli {
    paths: Vec<String>,
    fullscreen: bool,
    new_instance: bool,
    verbose: bool,
    help: bool,
    version: bool,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut cli = Self::default();
        let mut options = true;
        for arg in args {
            if options && arg == "--" {
                options = false;
                continue;
            }
            if options {
                match arg.as_str() {
                    "-f" | "--fullscreen" => {
                        cli.fullscreen = true;
                        continue;
                    }
                    "--new" => {
                        cli.new_instance = true;
                        continue;
                    }
                    "--verbose" => {
                        cli.verbose = true;
                        continue;
                    }
                    "-h" | "--help" => {
                        cli.help = true;
                        continue;
                    }
                    "--version" => {
                        cli.version = true;
                        continue;
                    }
                    _ => {}
                }
            }
            cli.paths.push(arg);
        }
        cli
    }

    fn print_help() {
        println!(
            "Usage: macplayer [OPTIONS] [FILES...]

Options:
    -f, --fullscreen  Start in full screen
        --new         Start without reusing the existing instance
        --verbose     Request verbose libmpv logging
    -h, --help        Show this help
        --version     Show the version"
        );
    }
}

fn normalize_cli_path(path: &str) -> String {
    if crate::util::is_url(path) {
        return path.to_string();
    }
    let expanded = expand_tilde(path, std::env::var("HOME").ok().as_deref());
    let path = std::path::Path::new(&expanded);
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path).to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

/// Expand a leading `~/` (or bare `~`) against `home`. Quoted tilde paths from
/// launchers that never glob would otherwise become `<cwd>/~/...` silently.
fn expand_tilde(path: &str, home: Option<&str>) -> String {
    if let Some(home) = home {
        if let Some(rest) = path.strip_prefix("~/") {
            return format!("{home}/{rest}");
        }
        if path == "~" {
            return home.to_string();
        }
    }
    path.to_string()
}

fn ipc_path() -> std::path::PathBuf {
    let uid = libc_getuid();
    std::env::temp_dir().join(format!("macplayer-{uid}.sock"))
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn libc_getuid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

#[cfg(not(unix))]
fn libc_getuid() -> u32 {
    0
}

/// Try to hand `paths` to an already-running instance over a Unix socket.
fn send_to_running(paths: &[String]) -> bool {
    let path = ipc_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return false;
        }
        Err(error) => {
            eprintln!("could not contact the existing MacPlayer instance: {error}");
            return true;
        }
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let payload = serde_json::json!({ "paths": paths }).to_string();
    if stream.write_all(payload.as_bytes()).is_err() {
        return true;
    }
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut ack = [0u8; 1];
    if stream.read_exact(&mut ack).is_err() {
        eprintln!("the existing MacPlayer instance did not acknowledge the request");
    } else if ack[0] != 1 {
        eprintln!("the existing MacPlayer instance rejected the request");
    }
    // A successful connect proves that another instance owns the socket.  Do not
    // start a second player merely because the request was rejected or timed out.
    true
}

const MAX_IPC_BYTES: usize = 64 * 1024;

struct InstanceServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl InstanceServer {
    fn start() -> Result<(Self, Receiver<Vec<String>>), String> {
        let path = ipc_path();
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(first_error) if path.exists() => {
                match UnixStream::connect(&path) {
                    Ok(_) => {
                        return Err("another MacPlayer instance is already running".into());
                    }
                    Err(error)
                        if !matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                        ) =>
                    {
                        return Err(format!("{} (socket probe failed: {error})", path.display()));
                    }
                    Err(_) => {}
                }
                let _ = std::fs::remove_file(&path);
                UnixListener::bind(&path).map_err(|error| {
                    format!(
                        "{} (stale socket recovery also failed: {error})",
                        first_error
                    )
                })?
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("set listener nonblocking: {error}"))?;

        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("macplayer-ipc".into())
            .spawn(move || instance_loop(listener, tx, thread_stop))
            .map_err(|error| format!("spawn listener: {error}"))?;

        Ok((
            Self {
                path,
                stop,
                thread: Some(thread),
            },
            rx,
        ))
    }
}

impl Drop for InstanceServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn instance_loop(listener: UnixListener, tx: Sender<Vec<String>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let mut payload = Vec::new();
                let valid = (&mut stream)
                    .take((MAX_IPC_BYTES + 1) as u64)
                    .read_to_end(&mut payload)
                    .is_ok()
                    && payload.len() <= MAX_IPC_BYTES;
                let paths = valid.then(|| decode_ipc_payload(&payload)).flatten();
                // Ack success only when the channel accepted the handoff —
                // during teardown the receiver may already be gone, and a
                // false ack silently loses the file.
                let accepted = paths
                    .as_ref()
                    .is_some_and(|paths| tx.send(paths.clone()).is_ok());
                drop(paths);
                let _ = stream.write_all(&[u8::from(accepted)]);
                let _ = stream.flush();
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            // A transient accept error must not kill the listener and leave a
            // socket file nothing can talk to behind; back off and keep serving.
            Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn decode_ipc_payload(payload: &[u8]) -> Option<Vec<String>> {
    let object: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let paths = object.get("paths")?.as_array()?;
    paths
        .iter()
        .map(|path| path.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Cli, decode_ipc_payload, expand_tilde, normalize_cli_path};

    #[test]
    fn tilde_paths_expand_against_home() {
        assert_eq!(expand_tilde("~/a.mkv", Some("/Users/x")), "/Users/x/a.mkv");
        assert_eq!(expand_tilde("~", Some("/Users/x")), "/Users/x");
        // No home or no tilde: pass through untouched.
        assert_eq!(expand_tilde("~/a.mkv", None), "~/a.mkv");
        assert_eq!(expand_tilde("/abs/a.mkv", Some("/Users/x")), "/abs/a.mkv");
        assert_eq!(expand_tilde("not~home", Some("/Users/x")), "not~home");
    }

    #[test]
    fn ipc_payload_accepts_only_string_paths() {
        assert_eq!(
            decode_ipc_payload(br#"{"paths":["/tmp/a.mp4","https://example.test/v"]}"#),
            Some(vec!["/tmp/a.mp4".into(), "https://example.test/v".into()])
        );
        assert_eq!(decode_ipc_payload(br#"{"paths":[42]}"#), None);
        assert_eq!(decode_ipc_payload(br#"{}"#), None);
    }

    #[test]
    fn cli_options_are_not_treated_as_media_paths() {
        let cli = Cli::parse([
            "--fullscreen".into(),
            "--verbose".into(),
            "--new".into(),
            "movie.mkv".into(),
        ]);
        assert!(cli.fullscreen);
        assert!(cli.verbose);
        assert!(cli.new_instance);
        assert_eq!(cli.paths, vec!["movie.mkv"]);
    }

    #[test]
    fn cli_paths_are_absolute_before_ipc_forwarding() {
        assert!(std::path::Path::new(&normalize_cli_path("movie.mkv")).is_absolute());
        assert_eq!(
            normalize_cli_path("https://example.test/movie.mp4"),
            "https://example.test/movie.mp4"
        );
    }
}
