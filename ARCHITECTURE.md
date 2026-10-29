# Architecture

A short map of how MacPlayer works, for contributors. Version 1.10.0.

## The frame loop

Everything runs on one UI thread inside `Player::frame` (`src/player.rs`):

1. **Drain** — engine messages (mpv property changes, file events), the IPC
   socket (file handoff + CLI settings mutations), remote Now Playing
   commands, the update-check result and folder-expansion answers are
   collected into `self.pending`.
2. **Timers** — autosave (background thread, single-flight), sleep timer,
   control-panel refresh, Now Playing publish (throttled to 1 Hz).
3. **Layout** — `Player::layout` computes the video rect, the floating
   control bar, the top actions and the sliding playlist drawer.
4. **Paint** — the mpv frame is blitted through an egui paint callback
   (`src/render.rs`), then the hand-drawn chrome is painted on top
   (`skin.rs` + `widgets.rs`).
5. **Dispatch** — `self.pending` actions are drained through
   `Player::dispatch`, which maps every `menus::Action` to engine calls,
   property writes or dialog state.

Repaint cadence is the app's main power lever: while playing, mpv's frame
callback drives one repaint per decoded frame; when paused the UI idles at a
coarse keepalive and wakes only for OSD fades, buffering readouts and the
play/pause flash (`repaint_interval_ms`).

## Threads

| Thread | Purpose |
| --- | --- |
| UI (main) | egui frame loop, mpv client-API calls, GL blit |
| `mpv-events` | `mpv_wait_event` loop → channel → UI drain |
| `ffprobe` | playlist duration probing (bounded job channel + backlog) |
| `thumbs` | ffmpeg preview frames, two lanes (priority + FIFO) with a disk cache |
| `covers` | embedded cover-art extraction for audio files |
| `open-expander` | folder walks for the open dialog / drag-drop |
| `settings-save` | single-flight background settings writer |
| `update-check` | one startup query against GitHub Releases |
| instance server | single-instance Unix socket (`IpcMessage`) |

Rule: **the UI thread never does I/O.** Anything slow is handed to a worker
and drained back through a channel.

## Settings ownership

`settings.json` is written atomically (temp file → rename → directory fsync)
by a single-flight background writer. The GUI is the *single writer* while it
runs: CLI mutations (`macplayer --cli …`) are forwarded over the instance
socket as `IpcMessage::SetSetting`/`Clear`/`ResetSettings` and applied to the
GUI's in-memory state, so the next autosave persists them instead of
reverting them. With no GUI running, the CLI writes the file directly.

## Feature toggles

Every optional behavior is a `Settings` bool surfaced in
`Preferences → General → Features` (serde `#[serde(default)]` keeps old
files compatible): continue watching, cover art, hover thumbnails, row
previews, Now Playing, update check. Read them live in the frame loop — no
restart required.

## Headless CLI

`macplayer --cli <command>` never opens a window (`src/cli.rs`): settings,
history, recent, playlist, positions, bookmarks, logs, `open` (forwards to
the running instance), `update-check` and `selftest`. Exit codes 0/1 for
scripts and CI.

## Packaging

`make package-macos ARCH=…` builds single-arch bundles; the universal merge
validates every dylib slice. Pushing a `v*` tag runs the GitHub Actions
release pipeline.
