# MacPlayer

MacPlayer is a native Rust media player for macOS. It uses egui for the custom
interface and libmpv for playback, so it supports the formats, codecs,
subtitles and network sources supported by mpv.

## Requirements

- macOS 13 (Ventura) or newer on Apple Silicon (`arm64`) or Intel (`x86_64`)
- Rust `1.95.0` (selected automatically by `rust-toolchain.toml`)
- Homebrew `mpv` for development-time native playback
- Apple Command Line Tools (`swift` and `iconutil`) for app icon generation

Install the development dependency once:

```sh
brew install mpv
export MACPLAYER_LIBMPV="$(brew --prefix mpv)/lib/libmpv.2.dylib"
```

The environment variable is optional when Homebrew is installed in a standard
location. It is useful when several libmpv versions are installed.

## Build and run from source

From this directory:

```sh
cargo fetch --locked
make build
./target/release/macplayer ~/Movies/example.mp4
```

For development:

```sh
cargo run --release -- ~/Movies/example.mp4
```

The command-line interface is:

```text
macplayer [OPTIONS] [FILES...]
  -f, --fullscreen  start in full screen
      --new         do not reuse the running instance
      --verbose     request verbose libmpv logging
  -h, --help        show help
      --version     show the version
```

Opening a second copy normally sends its files to the running MacPlayer
instance. Use `--new` to force a separate window.

## Installable macOS package

MacPlayer targets macOS 13 (Ventura) and newer on both Intel and Apple Silicon.
The release workflow builds a single Universal bundle containing both `arm64`
and `x86_64` slices, so users can download one file without identifying their
Mac's processor. The Rust binary is compiled with `MACOSX_DEPLOYMENT_TARGET=13.0`.

For a local single-architecture package, pass the target explicitly. For an
Apple Silicon package:

```sh
make package-macos ARCH=arm64
```

The command creates separate, single-architecture artifacts:

```text
dist/macos-arm64/MacPlayer.app
dist/macos-arm64/MacPlayer-macos-arm64.zip
dist/macos-arm64/MacPlayer-macos-arm64.dmg   # when hdiutil is available
```

For an Intel package, use an Intel Rust target and a matching x86_64 libmpv:

```sh
MACPLAYER_LIBMPV=/path/to/x86_64/libmpv.2.dylib make package-macos ARCH=x86_64
```

This creates the corresponding files under `dist/macos-x86_64/`. The package
script builds with `--offline --locked` and validates that the application,
libmpv, and bundled non-system dylibs contain exactly the requested
architecture. It does not turn an arm64 libmpv into an Intel build: on an
Apple Silicon host, an x86_64 package requires an installed
`x86_64-apple-darwin` Rust target and an actual x86_64 libmpv (for example,
from an Intel Homebrew installation or another compatible build). If either
is unavailable, packaging stops with an error.

`MacPlayer.app` includes the MacPlayer executable and the libmpv runtime under
`Contents/Frameworks`. The package script also adds the app's private
Frameworks directory to the dynamic loader path and registers media, subtitle
and playlist file associations in `Info.plist`. It generates a high-resolution
MacPlayer play-button icon and embeds it in the app bundle.

The package script runs on macOS and supports native or cross-target builds
when the requested Rust target and matching libmpv are available. For public
distribution, sign and notarize the app with an Apple Developer ID before
uploading it to GitHub; an unsigned first launch may be blocked by Gatekeeper.

If libmpv is in a non-standard location:

```sh
MACPLAYER_LIBMPV=/path/to/libmpv.2.dylib make package-macos ARCH=arm64
```

### Universal package

After building both single-architecture app bundles, merge them on macOS:

```sh
make package-macos-universal \
  ARM_APP=dist/macos-arm64/MacPlayer.app \
  X86_APP=dist/macos-x86_64/MacPlayer.app \
  OUT=dist/macos-universal
```

The merger validates the executable and every bundled dylib before using
`lipo`, then signs the resulting bundle ad hoc and creates:

```text
dist/macos-universal/MacPlayer.app
dist/macos-universal/MacPlayer-macos-universal.zip
dist/macos-universal/MacPlayer-macos-universal.dmg   # when hdiutil is available
```

Both slices must be built from matching dependency versions. The merger refuses
to produce a partial Universal bundle when a dylib is missing, single-arch, or
present in only one input bundle.

The GitHub Actions workflow in `.github/workflows/release.yml` builds the two
slices on native Intel and Apple Silicon runners, merges them, and attaches the
Universal ZIP and DMG to a GitHub Release when a `v*` tag is pushed.

For public distribution, replace the local ad-hoc signature with an Apple
Developer ID signature and notarize the app. A signature is required for a
smooth Gatekeeper experience but is not needed to verify the architecture merge.

## Tests and reproducible checks

Dependencies are locked in `Cargo.lock`; the project test command is offline:

```sh
make test
make check
make build
cargo fmt --all -- --check
```

`make test` currently covers settings validation, playlist
selection and probing, IPC payloads, mpv ABI layouts, renderer helpers and
click-to-open submenu state.

The tests do not require a display, audio device or libmpv. Real playback and
the app bundle require a macOS GUI/audio host and libmpv.

## Docker / Linux development

The Dockerfile is retained for a reproducible Linux build/runtime experiment;
it is not a macOS installer:

```sh
make docker-build
docker run --rm --name macplayer \
  --volume "$PWD/media:/media:ro" \
  macplayer:latest /media/example.mp4
```

## Features

- Hardware-decoded playback through libmpv when available, with software fallback.
- Drag-and-drop files, folders, URLs and external subtitles.
- Playlist queueing, repeat, shuffle, sorting, duplicate/missing-file cleanup
  and drag reorder.
- Resume positions, recent files, automatic window fitting and single-instance
  file forwarding.
- Subtitle stream selection, secondary subtitles, encoding, size, position and
  synchronisation controls.
- Audio stream/device selection, volume, mute, delay, normalisation and a
  ten-band equalizer.
- Chapters, A-B repeat, speed control, frame stepping, zoom, pan, rotation,
  deinterlacing and screenshots.
- Open one or more local paths or URLs directly from the clipboard with
  `Ctrl+Shift+V` (`Cmd+Shift+V` on macOS).
- Persistent bookmarks with direct seek actions, per-file audio/subtitle track preferences,
  HDR-to-SDR tone mapping controls, fullscreen display selection, clipboard frame capture,
  and live playlist filtering.
- Custom keyboard and mouse controls, including shortcuts that work with
  non-Latin keyboard layouts.
- Preferences and Control Panel both include an explicit Exit button.

## Settings

On macOS settings are stored at:

```text
~/Library/Application Support/MacPlayer/settings.json
```

## License and third-party components

MacPlayer is distributed under the license in `LICENSE`. Playback is provided
by [mpv](https://mpv.io) and FFmpeg through the libmpv runtime.
