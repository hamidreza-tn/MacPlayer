//! Headless command-line interface: `macplayer --cli <command> [args]`.
//!
//! Exposes the player's data and logic without opening a window — settings,
//! history, playlist, positions, bookmarks, logs, single-instance handoff,
//! the update check and a self-test. Every command prints plain text lines
//! and exits 0 on success, 1 on failure, so scripts and CI can rely on it.

use std::io::Write;

use crate::settings::Settings;

/// Entry point for `macplayer --cli …`; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let Some(command) = args.first() else {
        print_help();
        return 1;
    };
    let rest = &args[1..];
    match command.as_str() {
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        "info" => cmd_info(),
        "settings" => cmd_settings(rest),
        "history" => cmd_history(rest),
        "recent" => cmd_recent(rest),
        "playlist" => cmd_playlist(rest),
        "positions" => cmd_positions(rest),
        "bookmarks" => cmd_bookmarks(rest),
        "logs" => cmd_logs(rest),
        "open" => cmd_open(rest),
        "update-check" => cmd_update_check(),
        "selftest" => cmd_selftest(),
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            1
        }
    }
}

fn print_help() {
    println!(
        "Usage: macplayer --cli <COMMAND> [ARGS]

Commands:
  info                       show app/config paths and versions
  settings list              list every setting as JSON
  settings get <key>         print one setting
  settings set <key> <value> write one setting (JSON or plain text)
  settings reset             restore defaults
  history [N]                list watched files (newest first)
  history clear              clear watch history
  recent list                list recent files
  recent clear               clear recent files
  playlist list              list the saved playlist
  playlist add <path>...     append paths to the saved playlist
  playlist clear             clear the saved playlist
  positions [N]              list resume positions
  positions clear            clear resume positions
  bookmarks [N]              list bookmarks
  bookmarks clear            clear bookmarks
  logs [N]                   tail the diagnostic log (default 40)
  open <path>...             hand files/URLs to the running instance
  update-check               query GitHub for a newer release
  selftest                   run internal sanity checks"
    );
}

fn load() -> Settings {
    Settings::load()
}

fn save(settings: &Settings) -> Result<(), String> {
    settings.try_save()
}

fn print_json(value: &serde_json::Value) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(error) => eprintln!("encode error: {error}"),
    }
}

// -- commands -------------------------------------------------------------------------------------

fn cmd_info() -> i32 {
    let settings = load();
    print_json(&serde_json::json!({
        "app": "MacPlayer",
        "version": env!("CARGO_PKG_VERSION"),
        "config_dir": crate::settings::config_dir().to_string_lossy(),
        "settings_file": crate::settings::config_file().to_string_lossy(),
        "log_file": crate::logging::log_file().to_string_lossy(),
        "schema_version": settings.schema_version,
        "language": settings.language,
        "theme": settings.theme,
        "accent": settings.accent,
    }));
    0
}

fn cmd_settings(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("list") => {
            let settings = load();
            match serde_json::to_value(&settings) {
                Ok(value) => {
                    print_json(&value);
                    0
                }
                Err(error) => {
                    eprintln!("encode error: {error}");
                    1
                }
            }
        }
        Some("get") => {
            let Some(key) = args.get(1) else {
                eprintln!("usage: settings get <key>");
                return 1;
            };
            let settings = load();
            match serde_json::to_value(&settings) {
                Ok(value) => match value.get(key) {
                    Some(found) => {
                        print_json(found);
                        0
                    }
                    None => {
                        eprintln!("unknown key: {key}");
                        1
                    }
                },
                Err(error) => {
                    eprintln!("encode error: {error}");
                    1
                }
            }
        }
        Some("set") => {
            let (Some(key), Some(raw)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: settings set <key> <value>");
                return 1;
            };
            // Values parse as JSON first (booleans, numbers, strings with
            // quotes); a bare word is treated as a string.
            let value = serde_json::from_str::<serde_json::Value>(raw)
                .unwrap_or_else(|_| serde_json::Value::String(raw.clone()));
            let mut settings = load();
            let map = serde_json::Map::from_iter([(key.clone(), value)]);
            if !settings.apply_patch(map) {
                eprintln!("could not apply {key} (unknown key or bad value)");
                return 1;
            }
            match save(&settings) {
                Ok(()) => {
                    println!("{key} saved");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        Some("reset") => {
            let settings = Settings::default();
            match save(&settings) {
                Ok(()) => {
                    println!("settings reset to defaults");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: settings list | get <key> | set <key> <value> | reset");
            1
        }
    }
}

fn cmd_history(args: &[String]) -> i32 {
    let settings = load();
    match args.first().map(String::as_str) {
        Some("clear") => {
            let mut settings = settings;
            settings.clear_history();
            match save(&settings) {
                Ok(()) => {
                    println!("history cleared");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        _ => {
            let limit = args
                .first()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(20);
            for entry in settings.history.iter().take(limit) {
                println!(
                    "{}\t{}\t{} / {}\t{}",
                    crate::util::format_unix_time(entry.played_at),
                    entry.title,
                    crate::util::fmt_time(entry.position),
                    crate::util::fmt_time(entry.duration),
                    entry.path,
                );
            }
            0
        }
    }
}

fn cmd_recent(args: &[String]) -> i32 {
    let settings = load();
    match args.first().map(String::as_str) {
        Some("clear") => {
            let mut settings = settings;
            settings.recent.clear();
            match save(&settings) {
                Ok(()) => {
                    println!("recent cleared");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        _ => {
            for path in &settings.recent {
                println!("{path}");
            }
            0
        }
    }
}

fn cmd_playlist(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("add") => {
            let paths = &args[1..];
            if paths.is_empty() {
                eprintln!("usage: playlist add <path>...");
                return 1;
            }
            let mut settings = load();
            for path in crate::util::expand_paths(paths) {
                settings.playlist_paths.push(path);
            }
            match save(&settings) {
                Ok(()) => {
                    println!("playlist now holds {} items", settings.playlist_paths.len());
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        Some("clear") => {
            let mut settings = load();
            settings.playlist_paths.clear();
            settings.playlist_current = None;
            match save(&settings) {
                Ok(()) => {
                    println!("playlist cleared");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        _ => {
            let settings = load();
            for (index, path) in settings.playlist_paths.iter().enumerate() {
                let marker = if settings.playlist_current.as_deref() == Some(path) {
                    "*"
                } else {
                    " "
                };
                println!("{marker}{}. {path}", index + 1);
            }
            0
        }
    }
}

fn cmd_positions(args: &[String]) -> i32 {
    let settings = load();
    match args.first().map(String::as_str) {
        Some("clear") => {
            let mut settings = settings;
            settings.positions.clear();
            match save(&settings) {
                Ok(()) => {
                    println!("positions cleared");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        _ => {
            let limit = args
                .first()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(50);
            for (path, seconds) in settings.positions.iter().rev().take(limit) {
                println!("{}\t{}\t{path}", crate::util::fmt_time(*seconds), seconds);
            }
            0
        }
    }
}

fn cmd_bookmarks(args: &[String]) -> i32 {
    let settings = load();
    match args.first().map(String::as_str) {
        Some("clear") => {
            let mut settings = settings;
            settings.bookmarks.clear();
            match save(&settings) {
                Ok(()) => {
                    println!("bookmarks cleared");
                    0
                }
                Err(error) => {
                    eprintln!("save failed: {error}");
                    1
                }
            }
        }
        _ => {
            let limit = args
                .first()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(50);
            for bookmark in settings.bookmarks.iter().take(limit) {
                println!(
                    "{}\t{}\t{}",
                    crate::util::fmt_time(bookmark.time),
                    bookmark.title,
                    bookmark.path,
                );
            }
            0
        }
    }
}

fn cmd_logs(args: &[String]) -> i32 {
    let limit = args
        .first()
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(40);
    let path = crate::logging::log_file();
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!("(log file is empty or absent: {})", path.display());
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(limit);
    for line in &lines[start..] {
        println!("{line}");
    }
    0
}

fn cmd_open(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: open <path>...");
        return 1;
    }
    if crate::send_to_running(args) {
        println!("handed {} item(s) to the running instance", args.len());
        0
    } else {
        eprintln!("no running instance accepted the handoff");
        1
    }
}

fn cmd_update_check() -> i32 {
    match crate::update::check_for_update() {
        Some(tag) => {
            println!("update available: {tag}");
            println!("{}", crate::update::releases_page());
            0
        }
        None => {
            println!("up to date (or offline)");
            0
        }
    }
}

fn cmd_selftest() -> i32 {
    let mut failures = 0usize;
    let mut check = |name: &str, ok: bool| {
        println!("{:>6}  {name}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            failures += 1;
        }
    };

    // Settings roundtrip through a temp file.
    let dir = std::env::temp_dir().join(format!("macplayer-cli-{}", std::process::id()));
    let path = dir.join("settings.json");
    let roundtrip = (|| {
        std::fs::create_dir_all(&dir).ok()?;
        let settings = Settings::default();
        settings.try_save_to(&path).ok()?;
        let loaded = Settings::load_from(&path);
        Some(loaded == settings)
    })() == Some(true);
    check("settings roundtrip", roundtrip);

    // Patch application (the settings set path).
    let mut settings = Settings::default();
    let patched = settings.apply_patch(serde_json::Map::from_iter([(
        "volume".to_string(),
        serde_json::json!(37),
    )])) && settings.volume == 37;
    check("settings patch apply", patched);

    // Folder expansion on a real temp tree.
    let media_dir = dir.join("media");
    let _ = std::fs::create_dir_all(&media_dir);
    let clip = media_dir.join("clip.mkv");
    let expanded = std::fs::write(&clip, b"x").is_ok()
        && crate::util::expand_paths(&[dir.to_string_lossy().into_owned()])
            .iter()
            .any(|p| p.ends_with("clip.mkv"));
    check("folder expansion", expanded);

    // Pure helpers.
    check(
        "url detection",
        crate::util::is_url("https://example.test/v"),
    );
    check(
        "natural ordering",
        crate::util::natural_cmp("ep2", "ep10") == std::cmp::Ordering::Less,
    );
    check(
        "time parsing",
        crate::util::parse_time("01:30", None) == Some(90.0),
    );
    check(
        "version compare",
        crate::update::parse_version("v1.9.4") == Some((1, 9, 4)),
    );

    // Config directory writability.
    let writable = std::fs::create_dir_all(crate::settings::config_dir()).is_ok();
    check("config dir writable", writable);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::io::stdout().flush();
    if failures == 0 {
        println!("all checks passed");
        0
    } else {
        println!("{failures} check(s) failed");
        1
    }
}
