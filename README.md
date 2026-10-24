# MacPlayer

> A fast, native media player for macOS — built with Rust, egui and libmpv.

[![Build macOS Release](https://github.com/hamidreza-tn/MacPlayer/actions/workflows/release.yml/badge.svg)](https://github.com/hamidreza-tn/MacPlayer/actions/workflows/release.yml)

MacPlayer is a focused desktop player for local media, network sources,
subtitles and playlists. It uses libmpv for playback and a custom egui
interface for a responsive, keyboard-friendly experience.

## Highlights

- Native macOS application with a custom, decoration-free title bar.
- One Universal release for Apple Silicon and Intel Macs.
- Hardware decoding when supported, with software fallback.
- Local files, folders, URLs, playlists and external subtitles.
- Playlist queueing, sorting, repeat, shuffle, filtering and drag reorder,
  with live playback progress, total duration and auto-reveal of the
  playing row.
- Subtitle and audio track selection, synchronization and language preferences.
- Chapters, A/B repeat, speed control, frame stepping, zoom, pan and rotation.
- HDR-to-SDR tone mapping, equalizer controls and screenshot capture.
- Bookmarks, resume positions, recent files and single-instance forwarding.
- Clipboard opening for local paths and URLs.
- Keyboard shortcuts that use macOS conventions such as `Cmd` and `Option`.
- Light and dark themes with five accent colors, switchable live in Preferences.
- A `Cmd+K` command palette that searches the full menu tree and shows each
  command's shortcut.
- English and Persian (RTL-shaped) UI, selectable in Preferences.
- Seekbar hover thumbnails (generated in the background with ffmpeg).
- Watch history, playlist export to `.m3u8`, and a sleep timer.
- Loop modes, volume boost up to 200%, and per-file aspect/speed memory.
- yt-dlp stream quality picker, DVD/Blu-ray folder playback, HTTP header and
  user-agent overrides for streams.
- Audio passthrough (S/PDIF), exclusive output, channel layout, pitch-preserving
  speed change, video sync/interpolation, scaler profiles and toggleable video
  filters (deband/denoise/sharpen).
- A music-mode audio visualizer (wave/spectrum) for audio-only files.
- Trackpad gestures: pinch for fullscreen, `Ctrl`/`Cmd`+scroll to zoom the
  video, two-finger horizontal scroll to seek.
- Media keys via `F7`/`F8`/`F9` (previous / play-pause / next) and a PiP-style
  always-on-top mini player window.
- Size-capped rotating diagnostic log for crash reports and playback issues.

## Download

Open the [Releases](https://github.com/hamidreza-tn/MacPlayer/releases) page and
download the latest Universal package:

```text
MacPlayer-macos-universal.dmg
```

It runs on:

- Apple Silicon: M1, M2, M3, M4 and newer
- Intel: x86_64 Macs

MacPlayer targets **macOS 13 Ventura and newer**. The packaged application is
compiled with `MACOSX_DEPLOYMENT_TARGET=13.0`.

> Local ad-hoc builds are suitable for testing. For public distribution, use
> an Apple Developer ID signature and notarize the application so Gatekeeper
> can verify it smoothly.

## Build from source

### Requirements

- macOS 13 or newer
- Rust `1.95.0` — selected automatically by `rust-toolchain.toml`
- Homebrew `mpv` and its `libmpv` runtime
- Apple Command Line Tools for `swift` and `iconutil`

Install the native playback dependency:

```sh
brew install mpv
export MACPLAYER_LIBMPV="$(brew --prefix mpv)/lib/libmpv.2.dylib"
```

`MACPLAYER_LIBMPV` is optional when Homebrew uses its standard location. Set it
explicitly when multiple libmpv installations are available.

### Run the player

From the repository root:

```sh
cargo fetch --locked
make build
./target/release/macplayer ~/Movies/example.mp4
```

For an incremental development build:

```sh
cargo run --release -- ~/Movies/example.mp4
```

The command-line interface is:

```text
macplayer [OPTIONS] [FILES...]

Options:
    -f, --fullscreen  Start in full screen
        --new         Start without reusing the existing instance
        --verbose     Request verbose libmpv logging
    -h, --help        Show this help
        --version     Show the version
```

Opening another copy normally forwards its files to the running MacPlayer
instance. Use `--new` when a separate window is required.

## Create macOS packages

### Single-architecture packages

Build an Apple Silicon package:

```sh
make package-macos ARCH=arm64
```

Build an Intel package:

```sh
rustup target add x86_64-apple-darwin
MACPLAYER_LIBMPV=/path/to/x86_64/libmpv.2.dylib \
  make package-macos ARCH=x86_64
```

Each command creates a separate output directory:

```text
dist/macos-arm64/
dist/macos-x86_64/
```

The packaging script validates that the executable, libmpv and bundled dylibs
contain exactly the requested architecture. It refuses to use a mismatched or
Universal dependency for a single-architecture package.

### Universal package

After both single-architecture app bundles are available, merge them on macOS:

```sh
make package-macos-universal \
  ARM_APP=dist/macos-arm64/MacPlayer.app \
  X86_APP=dist/macos-x86_64/MacPlayer.app \
  OUT=dist/macos-universal
```

The merger validates the executable and every bundled dylib, combines matching
files with `lipo`, signs the result ad hoc and creates:

```text
dist/macos-universal/MacPlayer.app
dist/macos-universal/MacPlayer-macos-universal.zip
dist/macos-universal/MacPlayer-macos-universal.dmg
```

The DMG is created when `hdiutil` is available. A partial Universal bundle is
never produced: missing files, mismatched framework inventories or incorrect
architecture slices stop the merge.

## Automated releases

The workflow in
[`.github/workflows/release.yml`](.github/workflows/release.yml) builds:

1. An arm64 app on a native Apple Silicon runner.
2. An x86_64 app on a native Intel runner.
3. A validated Universal app, ZIP and DMG.

To publish a release:

```sh
git tag v0.1.0
git push origin v0.1.0
```

GitHub Actions then attaches the Universal packages to the matching GitHub
Release. The workflow also supports manual runs from the **Actions** tab.

## Tests and checks

The project keeps its dependency versions in `Cargo.lock` and uses offline,
locked commands for repeatable local checks:

```sh
make test
make check
make build
cargo fmt --all -- --check
```

The test suite covers settings validation, playlist behavior, IPC payloads,
mpv ABI layouts, renderer helpers, keyboard/menu state and window layout logic.
The tests do not require a display, audio device or libmpv. Real playback and
macOS packaging require a macOS GUI host and libmpv.

## Docker / Linux development

The Dockerfile provides a reproducible Linux build/runtime experiment. It is
not a macOS installer:

```sh
make docker-build
docker run --rm --name macplayer \
  --volume "$PWD/media:/media:ro" \
  macplayer:latest /media/example.mp4
```

## Settings and generated files

On macOS, settings are stored at:

```text
~/Library/Application Support/MacPlayer/settings.json
```

Diagnostics (engine errors, crash reports) are written next to it, rotated at
512 KB with one `.old` generation kept:

```text
~/Library/Application Support/MacPlayer/Logs/macplayer.log
```

Build output and local artifacts are intentionally ignored:

```text
target/
dist/
.DS_Store
```

## Project structure

```text
src/                         Rust application
assets/fonts/                Bundled Vazirmatn font (SIL OFL 1.1, see OFL.txt)
macos/                       App metadata and icon generation
scripts/                    macOS packaging tools
.github/workflows/           Automated Universal releases
Cargo.toml / Cargo.lock      Dependencies and reproducible versions
Makefile                     Common development commands
Dockerfile                   Linux build/runtime experiment
```

## Third-party components

Playback is provided by [mpv](https://mpv.io) and FFmpeg through the libmpv
runtime. The interface and application logic are written in Rust with
[egui](https://github.com/emilk/egui) and [eframe](https://github.com/emilk/eframe).
