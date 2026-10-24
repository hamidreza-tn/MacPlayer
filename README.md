<div align="center">

# MacPlayer

**A fast, native media player for macOS — built with Rust, egui and libmpv.**

Local files, network streams, playlists and subtitles in one quiet, keyboard-first
window. Hardware decoded, fully themeable, English and Persian.

[![Build macOS Release](https://github.com/hamidreza-tn/MacPlayer/actions/workflows/release.yml/badge.svg)](https://github.com/hamidreza-tn/MacPlayer/actions/workflows/release.yml)
[![Release](https://img.shields.io/github/v/release/hamidreza-tn/MacPlayer)](https://github.com/hamidreza-tn/MacPlayer/releases)
![Platform](https://img.shields.io/badge/platform-macOS%2013%2B%20·%20Apple%20Silicon%20·%20Intel-blue)
![Written in](https://img.shields.io/badge/written%20in-Rust-orange)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**[Download](#download) · [Features](#features) · [Shortcuts](#keyboard-shortcuts) · [Build](#build-from-source) · [Changelog](CHANGELOG.md)**

</div>

---

## Table of contents

- [Why MacPlayer](#why-macplayer)
- [Features](#features)
- [Download](#download)
- [Build from source](#build-from-source)
- [Command line](#command-line)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Settings, history and diagnostics](#settings-history-and-diagnostics)
- [Project structure](#project-structure)
- [Development](#development)
- [Docker / Linux experiment](#docker--linux-experiment)
- [Contributing](#contributing)
- [Acknowledgements](#acknowledgements)
- [License](#license)

## Why MacPlayer

Most macOS players are either heavy (a web engine per window) or minimal to a
fault. MacPlayer takes a third path: **libmpv does the playback** — battle-tested,
hardware-accelerated, format-everything — while a hand-drawn **egui interface**
stays out of the way. The video fills the window; controls float, fade and hide
until you move the pointer. One Rust binary, no Electron, no external runtime
beyond libmpv.

- **Quiet by default** — floating chrome fades out while you watch; the cursor hides with it.
- **Keyboard-first** — a macOS-convention keymap plus a `⌘K` command palette over the entire menu tree.
- **Bilingual** — complete English and Persian (RTL-shaped) UI, switchable live.
- **Honest engineering** — atomic settings saves, a size-capped rotating diagnostic log, crash reporting, and 69 unit tests that need no display or audio device.

## Features

**Playback**

- libmpv engine with hardware decoding (VideoToolbox on Apple Silicon / Intel) and software fallback
- Local files, folders (recursive), URLs, `m3u`/`m3u8`/`pls`/`cue` playlists, DVD (`VIDEO_TS`) and Blu-ray (`BDMV`) folders
- Resume positions, per-file aspect & speed memory, bookmarks, A-B repeat, chapters, loop modes
- Frame stepping, speed 0.2×–12× with pitch preservation, zoom/pan/rotate/flip, image adjustments
- HDR-to-SDR tone mapping (bt.2390 / reinhard / mobius / hable), deinterlace, deband / denoise / sharpen filters, scaler profiles, frame interpolation
- Sleep timer, audio-only visualizer (wave / spectrum), PiP-style always-on-top mini player

**Audio & subtitles**

- Track selection with language preferences, external subtitle/audio loading, secondary subtitle track
- Subtitle styling (font, size, color, border, shadow, background), encoding override, sync in 0.5 s steps
- 10-band equalizer with presets, dynamic range normalization, volume boost to 200%
- S/PDIF bitstream passthrough, exclusive output mode, channel layout control
- Network streams: yt-dlp quality picker, HTTP user-agent & custom headers, stream/demuxer cache sizing

**Playlist**

- Drag-to-reorder with undo/redo, multi-select, shift-range select, keyboard navigation
- Sort by name / path / duration / reverse / random, dedupe, remove missing files
- Live playback progress on the playing row, total duration in the header, filter-as-you-type
- Background duration probing and seekbar hover thumbnails (via ffmpeg, when present)
- Watch history, recent files, export to `.m3u8`

**Interface**

- Light & dark themes with five accent colors — applied live
- `⌘K` command palette with shortcut hints, sharing one command source with the menus
- Fullscreen, windowed and mini-player layouts; trackpad gestures (pinch → fullscreen, `⌘`/`Ctrl`-scroll → zoom, two-finger horizontal scroll → seek)
- Single-instance file forwarding, drag-and-drop (files, folders, subtitles), clipboard opening
- Media keys `F7`/`F8`/`F9` and hardware media-key support

## Download

Grab the latest Universal package from the
[**Releases**](https://github.com/hamidreza-tn/MacPlayer/releases) page:

```text
MacPlayer-macos-universal.dmg
```

- Runs on **Apple Silicon** (M1–M4 and newer) and **Intel** Macs
- Targets **macOS 13 Ventura and newer** (packaged with `MACOSX_DEPLOYMENT_TARGET=13.0`)
- The bundle is ad-hoc signed; on first launch macOS may ask to confirm — see
  [Release notes](https://github.com/hamidreza-tn/MacPlayer/releases) for details
- First launch opens the welcome screen: press `⌘O`, drag a video in, or hit play

> For public distribution outside GitHub, sign with an Apple **Developer ID**
> and notarize so Gatekeeper verifies the app silently.

## Build from source

### Requirements

- macOS 13 or newer
- Rust `1.95.0` — selected automatically by `rust-toolchain.toml`
- Homebrew `mpv` (provides `libmpv`)
- Apple Command Line Tools (`swift`, `iconutil` for packaging)

```sh
brew install mpv
export MACPLAYER_LIBMPV="$(brew --prefix mpv)/lib/libmpv.2.dylib"

cargo fetch --locked
make build
./target/release/macplayer ~/Movies/example.mp4
```

`MACPLAYER_LIBMPV` is optional when Homebrew uses its standard location; set it
explicitly when multiple libmpv installations exist.

For day-to-day work:

```sh
cargo run --release -- ~/Movies/example.mp4
```

### Command line

```text
macplayer [OPTIONS] [FILES...]

Options:
    -f, --fullscreen  Start in full screen
        --new         Start without reusing the existing instance
        --verbose     Request verbose libmpv logging
    -h, --help        Show this help
        --version     Show the version
```

Opening another copy normally **forwards its files to the running instance**
over a Unix socket; use `--new` when a separate window is genuinely needed.

## Keyboard shortcuts

Steps (`5 s / 30 s / 60 s / 300 s` and the volume step) are configurable in
**Preferences → Playback**. The full map lives in `src/player.rs::keymap`.

**Playback & files**

| Action | Keys |
| --- | --- |
| Play / pause | `Space` |
| Fullscreen | `Enter`, `F11`, trackpad pinch |
| Stop | `F4` |
| Previous / next file | `PageUp` / `PageDown`, `F7` / `F9` |
| Open files / folder / URL | `⌘O` (`F3`) / `F2` / `⌘U` |
| Open from clipboard | `⌘⇧V` |
| Go to time / percent | `G` |
| Add bookmark | `B` |
| Playback info overlay | `Tab` |

**Seeking**

| Action | Keys |
| --- | --- |
| Small step (default 5 s) | `←` / `→` |
| Medium (30 s) | `⌘←` / `⌘→` |
| Large (60 s) | `⇧←` / `⇧→` |
| Huge (300 s) | `⌘⌥←` / `⌘⌥→` |
| Seek to start | `Backspace`, `Home` |
| Two-finger horizontal scroll | Seek by the small step |

**Speed & frames**

| Action | Keys |
| --- | --- |
| Faster / slower (0.1×) | `C` / `X` |
| Reset to 1× | `Z` |
| Frame step forward / back | `F` / `D` |
| A-B repeat: set A / set B / clear | `[` / `]` / `\` |
| Chapter prev / next | `⇧⇞` / `⇧⇟` |

**Audio & subtitles**

| Action | Keys |
| --- | --- |
| Volume / mute | `↑` / `↓`, `M` |
| Subtitle menu / cycle track | `L` / `⌥L` |
| Audio menu / cycle track | `A` / `⌥A` |
| Show / hide subtitles | `⌥H` |
| Subtitle sync −0.5 s / +0.5 s / reset | `,` / `.` / `/` |
| Audio sync −0.05 s / +0.05 s / reset | `⇧,` / `⇧.` / `⇧/` |
| Move subtitles up / down | `⌥↑` / `⌥↓` |
| Subtitle size | `⌥⇞` / `⌥⇟` |
| Load external subtitle | `⌥O` |

**Video & image**

| Action | Keys |
| --- | --- |
| Zoom in / out | `9` / `1` |
| Pan (left / right / up / down) | `4` / `6` / `8` / `2` |
| Reset pan & scan | `5` |
| Rotate 90° | `⌥K` |
| Flip horizontal / vertical | `⌘Z` / `⌘V` |
| Brightness −/+ | `W` / `E` |
| Contrast −/+ | `R` / `T` |
| Saturation −/+ | `Y` / `U` |
| Hue −/+ | `I` / `O` |
| Reset image adjustments | `Q` |
| Window 50 / 100 / 150 / 200 % | `⌥1`–`⌥4` |
| Zoom video with wheel | `⌘`/`Ctrl` + scroll |

**Window & app**

| Action | Keys |
| --- | --- |
| Command palette | `⌘K` |
| Playlist drawer | `⌘P`, `F6`, `Esc` closes |
| Preferences | `⌘,`, `F5` |
| Always-on-top cycle | `⌘T` |
| Screenshot / with subtitles | `⌘E` / `⌘⌥E` |
| Copy frame to clipboard | `⌘C` / `⌘⌥C` |
| Playlist undo / redo | `⌘⌥Z` / `⌘⌥⇧Z` |
| About / system info | `F1` / `⌘F1` |
| Close window / quit | `⌘W` / `⌘Q` |

## Settings, history and diagnostics

Everything user-facing persists under one directory (moved with `XDG_CONFIG_HOME`
on non-mac builds):

```text
~/Library/Application Support/MacPlayer/
├── settings.json     # one JSON file: window geometry, playback, theme, per-file memory
└── Logs/
    └── macplayer.log # diagnostic log; rotates at 512 KB, keeps one .old generation
```

- `settings.json` is written **atomically** (temp file → `rename` → directory fsync),
  so a crash or power loss can never truncate it; damaged individual values are
  repaired against sane defaults on load.
- The log captures engine errors, mpv warnings and panic reports. Delete the file
  any time; it is rebuilt on demand.
- Uninstalling is complete after removing the folder.

## Project structure

```text
src/                         Rust application
├── main.rs                  CLI, single-instance IPC server, panic hook
├── player.rs                App state, chrome layout, keymap, action dispatch
├── engine.rs                libmpv facade + event thread
├── render.rs                mpv↔OpenGL FBO plumbing behind the egui paint callback
├── skin.rs                  Themes, accents, metrics, hand-drawn glyph set
├── widgets.rs               Seek bar, volume bar, OSD, icon buttons
├── playlist.rs              Playlist model + drawer UI, background duration probe
├── menus.rs                 Menu model & painter (context menu, command palette source)
├── dialogs.rs               Preferences, control panel, GoTo/URL/history dialogs
├── settings.rs              Schema, normalization, atomic persistence
├── thumbs.rs                ffmpeg-backed seekbar previews
├── services.rs              Platform shell (file dialogs, clipboard, reveal)
├── i18n.rs                  English/Persian string table
├── logging.rs               Rotating diagnostic log
└── util.rs, value.rs, ffi.rs

assets/fonts/                Bundled Vazirmatn font (SIL OFL 1.1, see OFL.txt)
macos/                       App metadata and icon generation
scripts/                     macOS packaging tools
.github/workflows/           Automated Universal releases
```

## Development

Dependency versions are pinned in `Cargo.lock`; local checks run offline:

```sh
make check       # fast type check
make test        # unit tests — no display, audio device or libmpv required
make clippy      # lints (unwrap/expect/panic denied in non-test code)
make fmt         # formatting
make build       # release build
```

The lint regime is strict by design: `unsafe_code` is denied crate-wide except
for the scoped C boundary (`ffi.rs`, `engine.rs`, `render.rs`, `value.rs`), and
`unwrap`/`expect`/`panic` are denied in production paths.

### Create macOS packages

Single-architecture bundles:

```sh
make package-macos ARCH=arm64    # → dist/macos-arm64/MacPlayer.app
rustup target add x86_64-apple-darwin
MACPLAYER_LIBMPV=/path/to/x86_64/libmpv.2.dylib \
  make package-macos ARCH=x86_64
```

Merge into a validated Universal bundle (checks every dylib's slices, `lipo`
merges, ad-hoc signs, then builds the DMG):

```sh
make package-macos-universal \
  ARM_APP=dist/macos-arm64/MacPlayer.app \
  X86_APP=dist/macos-x86_64/MacPlayer.app \
  OUT=dist/macos-universal
```

Pushing a tag (`git tag v1.9.0 && git push origin v1.9.0`) makes
[GitHub Actions](.github/workflows/release.yml) build both architectures natively,
merge them, and attach the ZIP + DMG to the matching GitHub Release. Manual runs
are supported from the **Actions** tab. The packaging script refuses mismatched
architectures — a partial Universal bundle is never produced.

## Docker / Linux experiment

The Dockerfile is a reproducible Linux build/runtime experiment — **not** a macOS
installer:

```sh
make docker-build
docker run --rm --name macplayer \
  --volume "$PWD/media:/media:ro" \
  macplayer:latest /media/example.mp4
```

## Contributing

Issues and pull requests are welcome. Please:

- keep `make test && make clippy && cargo fmt --all -- --check` green,
- add or extend tests for behavior changes (the suite is display-free by design),
- add user-facing strings through `i18n` (`tr(...)`) with a Persian entry,
- describe user-visible changes in `CHANGELOG.md`.

## Acknowledgements

- [mpv](https://mpv.io) and [FFmpeg](https://ffmpeg.org) — playback, via the libmpv runtime
- [egui](https://github.com/emilk/egui) & [eframe](https://github.com/emilk/eframe) — immediate-mode interface
- [Vazirmatn](https://github.com/rastikerdar/vazirmatn) — bundled Persian font (SIL OFL 1.1, see [`assets/fonts/OFL.txt`](assets/fonts/OFL.txt))
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) — optional on-demand component for online streams

## License

MacPlayer is released under the [MIT License](LICENSE) — free to use, modify,
redistribute and ship, even commercially, with no warranty. If you fork it,
keeping the copyright notice and this license is all that is asked in return.

The bundled Vazirmatn font is distributed under its own
[SIL OFL 1.1](assets/fonts/OFL.txt) license, and playback relies on the
[mpv](https://mpv.io)/[FFmpeg](https://ffmpeg.org) runtime installed alongside it
(LGPL/GPL depending on build), which remains governed by its own terms.
