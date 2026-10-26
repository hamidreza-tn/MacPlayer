//! Platform services abstraction: file pickers, clipboard and
//! "open in file manager" behind one trait, so the player core never touches an OS
//! API directly and tests can supply a fake.

use std::path::PathBuf;

pub trait Services {
    /// Pick one or more media files; `start` is the initial directory ("" = let the OS decide).
    fn pick_files(
        &mut self,
        filter_name: &str,
        filter_extensions: &[String],
        start: &str,
    ) -> Vec<String>;

    /// Pick a single file; `start` is the initial directory ("" = let the OS decide).
    fn pick_file(
        &mut self,
        filter_name: &str,
        filter_extensions: &[String],
        start: &str,
    ) -> Option<String>;

    /// Pick a single folder; `start` is the initial directory ("" = let the OS decide).
    fn pick_folder(&mut self, start: &str) -> Option<String>;

    /// Pick a save destination, starting from `file_name` (e.g. "playlist.m3u8").
    fn pick_save_file(&mut self, file_name: &str) -> Option<String>;

    /// Read plain text from the system clipboard.
    fn clipboard_text(&mut self) -> Result<String, String>;

    /// Put RGBA pixels on the clipboard.
    fn clipboard_put_image(
        &mut self,
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    ) -> Result<(), String>;

    /// Reveal `path` in the platform file manager (Finder on macOS).
    fn reveal_path(&self, path: &str);

    /// Open a URL (release page, issue tracker) in the default browser.
    fn open_url(&self, url: &str);
}

/// Real desktop implementation backed by rfd, arboard and `open`/`xdg-open`.
pub struct PlatformServices;

impl Services for PlatformServices {
    fn pick_files(
        &mut self,
        filter_name: &str,
        filter_extensions: &[String],
        start: &str,
    ) -> Vec<String> {
        let extensions: Vec<&str> = filter_extensions.iter().map(String::as_str).collect();
        let mut dialog = rfd::FileDialog::new();
        if !start.is_empty() {
            dialog = dialog.set_directory(start);
        }
        dialog
            .add_filter(filter_name, &extensions)
            .pick_files()
            .map(|files| {
                files
                    .into_iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn pick_file(
        &mut self,
        filter_name: &str,
        filter_extensions: &[String],
        start: &str,
    ) -> Option<String> {
        let extensions: Vec<&str> = filter_extensions.iter().map(String::as_str).collect();
        let mut dialog = rfd::FileDialog::new();
        if !start.is_empty() {
            dialog = dialog.set_directory(start);
        }
        dialog
            .add_filter(filter_name, &extensions)
            .pick_file()
            .map(|path: PathBuf| path.to_string_lossy().to_string())
    }

    fn pick_folder(&mut self, start: &str) -> Option<String> {
        let mut dialog = rfd::FileDialog::new();
        if !start.is_empty() {
            dialog = dialog.set_directory(start);
        }
        dialog
            .pick_folder()
            .map(|path: PathBuf| path.to_string_lossy().to_string())
    }

    fn pick_save_file(&mut self, file_name: &str) -> Option<String> {
        rfd::FileDialog::new()
            .set_file_name(file_name)
            .save_file()
            .map(|path: PathBuf| path.to_string_lossy().to_string())
    }

    fn clipboard_text(&mut self) -> Result<String, String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|error| error.to_string())
    }

    fn clipboard_put_image(
        &mut self,
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    ) -> Result<(), String> {
        let data = arboard::ImageData {
            width,
            height,
            bytes: std::borrow::Cow::Owned(rgba),
        };
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_image(data))
            .map_err(|error| error.to_string())
    }

    fn reveal_path(&self, path: &str) {
        #[cfg(target_os = "macos")]
        {
            // `open -R` reveals the item in Finder; a bare `open` would launch the
            // file with its default application instead.
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = std::process::Command::new("xdg-open").arg(path).spawn();
        }
    }

    fn open_url(&self, url: &str) {
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(url).spawn();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        }
    }
}
