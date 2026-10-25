# Changelog

All notable changes to MacPlayer are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [1.9.0] - 2026-10-24

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
  active, and folder expansion skipping symlinked folders (with symlink-cycle
  protection).
- "Add URL..." from the playlist now appends to it instead of replacing it;
  only "Open URL..." in the File menu starts a fresh list.
- Playlists larger than the probe queue no longer lose row durations; overflow
  is retried from a bounded backlog.
- Per-file aspect ratio and speed, track selection, resume positions,
  bookmarks, A-B repeat, equalizer and sleep timer behavior unchanged and
  covered by the expanded test suite.

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
