# Changelog

All notable changes to MacPlayer are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [1.9.4] - 2026-10-26

Full-codebase audit: 24 defects found and fixed (8 major, 16 minor).

### Fixed

- Quitting after adding a large playlist can no longer deadlock the probe
  worker; results are delivered without blocking and teardown always frees
  the ffprobe thread.
- The playlist drawer can no longer overlap the floating transport buttons
  on narrow windows (drawer resize and layout are both bounded by the chrome
  the controls need).
- The mini player's 480x300 window size is no longer persisted as the app's
  window size; the pre-mini geometry survives restarts.
- A URL "loop count" no longer leaks into later playback: it is cleared for
  local files and explicitly reset when the count is 0.
- Subtitles keep their margin when the window is resized or fullscreen
  changes (the mpv margin is re-derived from the live viewport height).
- Engine/renderer teardown order is safe on unwind paths too: the render
  context is freed before the mpv core in every drop path (mpv render.h
  requires this; undefined behavior otherwise).
- Playlist row previews beyond the sixth no longer stay as placeholders —
  the per-frame decode budget charges only first-use decodes, never cached
  textures.
- Tab strips in the Go to / Control Panel / Preferences dialogs use scoped
  ids, so clicking a tab in one dialog can no longer affect another dialog
  open at the same time.
- Control Panel sliders for subtitle size/position and playback speed no
  longer snap back mid-drag from the periodic refresh.
- Numeric fields with fractional steps (audio sync 0.05 s, subtitle sync
  0.5 s, border/shadow) display their decimals instead of "0".
- Rapidly switching files no longer queues stale priority thumbnail jobs
  ahead of the current file, and cached files are not re-decoded.
- Old-generation probe backlog entries are discarded instead of pinning
  queue capacity forever.
- Menu panels include their top padding: the last item is no longer clipped
  and the stray clickable strip outside the panel is gone.
- Undo re-resolves the playing row by path, so the progress marker cannot
  desync from the file mpv is actually playing.
- Dragging media plus subtitles onto the playlist while playing now loads
  the subtitles on the current file instead of dropping them.
- The idle-screen play button no longer triggers a second file dialog from
  the deferred full-window click.
- Paused seeks chain correctly even when mpv takes longer than 0.6 s to
  report the new position.
- The play/pause flash fades smoothly when paused (covered by the repaint
  cadence).
- Folder opening routes through the background worker for every batch, so
  even the routing stat of a stalled network mount cannot block the UI; the
  "Reading folder..." notice appears only after a short delay.
- While the resume dialog is visible, transport clicks are kept for the next
  frame instead of being discarded.
- mpv-side engine shutdown no longer skips core teardown, and failed async
  commands (e.g. a corrupt subtitle passed to sub-add) are logged instead of
  vanishing.
- The single-instance handoff only acknowledges files the player actually
  received, and `~/file.mp4` passed quoted on the command line expands to
  the home directory.

## [1.9.3] - 2026-10-26

### Fixed

- Fixed settings save races and stale background writes.
- Fixed stale folder-expansion results replacing newer media selections.
- Fixed thumbnail priority jobs waiting behind FIFO work.
- Fixed volume boost persistence, loop handling, EQ replacement and visualizer cleanup.
- Fixed mini-player window restoration and gesture hit testing.
- Fixed stream option escaping, load failure reporting and screenshot errors.
- Added libmpv API compatibility checks and safer settings/log file permissions.
- Prevented folder scans from traversing nested symlinks outside the selected directory.
- Added XSPF playlist loading and safer M3U export handling.

## [1.9.2] - 2026-10-26

The interface, packaging and playback pipeline all receive a major polish pass
for the first public release.

### Interface

- New idle welcome screen: four-corner gradient backdrop with a dot grid,
  a hover-animated play affordance and keyboard hints.
- Floating chrome (control bar, top actions, scrims) fades in and out instead of
  snapping; the playlist drawer slides over the video with easing while the
  right-hand controls reflow alongside it.
- Redesigned playlist drawer: accent header with total duration, per-row
  playback progress line, automatic reveal of the playing row, a draggable
  scrollbar thumb, and landscape video previews on every row (generated in the
  background as rows scroll into view).
- Command palette (`Cmd+K`) with full-width rows and right-aligned shortcut
  hints; the palette, the context menu and the keyboard all share one command
  source.
- Menus gain a short fade-in, hand-drawn submenu chevrons and macOS-style hover.
- All dialogs are re-skinned: accent header dot, segmented tab strips, primary
  (accent-filled) action buttons, fully clickable checkboxes, a redesigned
  Resume dialog and a row-based watch history.
- Sliders, text selection and list highlights follow the chosen accent color.
- Redesigned audio-only screen (disc mark, title and elapsed time), a
  drag-and-drop accent ring, and a fade-out for on-screen messages.
- Gesture fixes: trackpad pinch (only) toggles full screen, `Ctrl`/`Cmd`+scroll
  zooms the video, two-finger horizontal scroll seeks.
- Universal double-triangle rewind / fast-forward transport icons, a quiet `—`
  time readout when nothing is loaded, and fixed popover/palette widths that no
  longer stretch to the screen edge.

### Playback

- Fixed invalid `video-sync=auto` being sent to mpv on startup; the sync modes
  now map onto mpv's real option values.
- Fixed rejected `target-peak=0` and mistyped `sub-margin-y` property writes.
- Fixed the idle-screen play button losing clicks to the full-window video
  hit target, playlist reveal scrolling to the wrong row when a filter was
  active, and folder expansion traversing symlinked folders outside the
  selected directory.
- "Add URL..." from the playlist now appends to it instead of replacing it;
  only "Open URL..." in the File menu starts a fresh list.
- Playlists larger than the probe queue no longer lose row durations; overflow
  is retried from a bounded backlog.
- Per-file aspect ratio and speed, track selection, resume positions,
  bookmarks, A-B repeat, equalizer and sleep timer behavior unchanged and
  covered by the expanded test suite.

### Performance

- Repaint cadence is now power-aware: during playback the interface follows
  mpv's decoded-frame callback (a 24 fps video repaints ~24×/s instead of a
  fixed 60×/s), and when paused the UI drops to a coarse keepalive — waking
  only for OSD fades and buffering readouts. The main process idles at a
  fraction of its previous CPU during playback.
- Hardware decoding prefers direct VideoToolbox→GL interop over copy-back on
  macOS, skipping the per-frame GPU→RAM→GPU round-trip (notably at 4K), with
  copy mode and `auto-safe` kept as automatic per-file fallbacks.
- The 10 s settings autosave persists from a background thread, so the periodic
  JSON write + fsync can no longer bump the UI.
- Playlist row previews decode at most a few frames per frame budget, keeping
  drawer opening hitch-free even with a screenful of rows.
- Opening a folder expands off the UI thread on a persistent worker (large
  folders no longer freeze the window); playback of files and URLs keeps the
  instant synchronous path.
- Release builds now use full LTO.

### Diagnostics & platform

- Size-capped rotating diagnostic log at
  `~/Library/Application Support/MacPlayer/Logs/macplayer.log`, including a
  panic hook so crash reports are captured.
- Keyboard handling no longer clones the input event list every frame.
- App bundle declares the video App Store category, `NSPrincipalClass` and
  media/subtitle/playlist document types for Finder integration.

### Licensing

- The project is now released under the MIT License (see `LICENSE`); the
  bundled Vazirmatn font remains under SIL OFL 1.1.

## [0.1.2] - earlier

Initial feature-complete player: libmpv playback, playlist drawer with
drag-reorder and undo, subtitles, chapters, bookmarks, resume, equalizer,
HDR tone mapping, streaming options, sleep timer, mini player, command
palette, English/Persian UI and Universal macOS packaging.
