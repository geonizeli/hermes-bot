//! Themes, read apart from the UI so they can be tested.
//!
//! hermes-bot ships a dark and a light theme and can follow the desktop's
//! light/dark preference. On Omarchy it can also use the live Omarchy theme,
//! from `~/.local/state/omarchy/current/theme/colors.toml`. A theme switch
//! replaces files and directories under `current`, so the watcher re-arms
//! itself after every change instead of trusting one inode. Under Hyprland
//! the corners follow its rounding.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// The colors the UI is drawn with. Field names follow what the UI uses them
/// for; `read_palette` maps Omarchy's keys onto them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub accent: String,
    pub background: String,
    pub surface: String,
    pub raised: String,
    pub foreground: String,
    pub muted: String,
    pub border: String,
    pub danger: String,
    // The theme's terminal colours, for colourful content (code, headings,
    // tool chips). Themes map these freely, so they're used by name only.
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub orange: String,
}

impl Default for Palette {
    fn default() -> Self {
        Palette::dark()
    }
}

impl Palette {
    /// The built-in dark theme: neutral greys and the icon's yellow.
    pub fn dark() -> Self {
        Palette {
            accent: "#FFD60A".into(),
            background: "#0e0e10".into(),
            surface: "#151517".into(),
            raised: "#1e1e21".into(),
            foreground: "#e6e6e6".into(),
            muted: "#8a8a8d".into(),
            border: "#2a2a2e".into(),
            danger: "#e5484d".into(),
            red: "#ff6b6b".into(),
            green: "#3fb950".into(),
            yellow: "#e3b341".into(),
            blue: "#4493f8".into(),
            magenta: "#bc8cff".into(),
            cyan: "#39c5cf".into(),
            orange: "#f0883e".into(),
        }
    }

    /// The built-in light theme. The accent darkens to an amber so it still
    /// reads as text on white.
    pub fn light() -> Self {
        Palette {
            accent: "#b07800".into(),
            background: "#fbfbfa".into(),
            surface: "#f2f2f0".into(),
            raised: "#e8e8e5".into(),
            foreground: "#1d1d1f".into(),
            muted: "#6b6b70".into(),
            border: "#d9d9d5".into(),
            danger: "#d1242f".into(),
            red: "#cf222e".into(),
            green: "#1a7f37".into(),
            yellow: "#9a6700".into(),
            blue: "#0969da".into(),
            magenta: "#8250df".into(),
            cyan: "#1b7c83".into(),
            orange: "#bc4c00".into(),
        }
    }
}

/// The theme the user picked. `None` in the settings means "the default":
/// Omarchy's when it's installed, otherwise the system's preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    /// The built-in dark or light theme, whichever the desktop prefers.
    System,
    Dark,
    Light,
    /// The live Omarchy theme.
    Omarchy,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::Omarchy];

    /// The name saved in the settings and typed after `/skin`.
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
            ThemeChoice::Omarchy => "omarchy",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "Match system",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
            ThemeChoice::Omarchy => "Omarchy",
        }
    }

    pub fn parse(key: &str) -> Option<ThemeChoice> {
        let key = key.trim().to_lowercase();
        ThemeChoice::ALL.into_iter().find(|c| c.key() == key)
    }

    /// The choice in effect: the saved one, or the default when nothing is
    /// saved. Omarchy falls back to the system's when it isn't installed.
    pub fn effective(saved: Option<ThemeChoice>, omarchy: bool) -> ThemeChoice {
        match saved {
            Some(ThemeChoice::Omarchy) | None if omarchy => ThemeChoice::Omarchy,
            Some(ThemeChoice::Omarchy) | None => ThemeChoice::System,
            Some(choice) => choice,
        }
    }
}

/// What the window is drawn with, once the choice meets the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeSource {
    Dark,
    Light,
    Omarchy,
}

pub fn resolve(choice: ThemeChoice, system_dark: bool) -> ThemeSource {
    match choice {
        ThemeChoice::Omarchy => ThemeSource::Omarchy,
        ThemeChoice::Dark => ThemeSource::Dark,
        ThemeChoice::Light => ThemeSource::Light,
        ThemeChoice::System if system_dark => ThemeSource::Dark,
        ThemeChoice::System => ThemeSource::Light,
    }
}

/// Corner radius: Hyprland's rounding when running under it, so the window
/// matches its neighbours; otherwise sharp for Omarchy (its default) and a
/// gentle rounding for the built-in themes.
pub fn corner_radius(source: ThemeSource, hyprland: Option<i32>) -> i32 {
    match (hyprland, source) {
        (Some(r), _) => r.max(0),
        (None, ThemeSource::Omarchy) => 0,
        (None, _) => 6,
    }
}

/// Whether an Omarchy theme is there to use.
pub fn omarchy_installed(current_dir: &Path) -> bool {
    current_dir.join("theme/colors.toml").is_file()
}

pub fn omarchy_current_dir() -> PathBuf {
    home().join(".local/state/omarchy/current")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/".into())
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Reads `<current>/theme/colors.toml`. Any key that's missing or malformed
/// keeps its default, so a partial theme still renders.
pub fn read_palette(current_dir: &Path) -> Palette {
    let mut palette = Palette::default();
    let Ok(text) = std::fs::read_to_string(current_dir.join("theme/colors.toml")) else {
        return palette;
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        return palette;
    };
    let pick = |keys: &[&str], slot: &mut String| {
        for key in keys {
            if let Some(value) = table.get(*key).and_then(|v| v.as_str()) {
                if is_hex_color(value) {
                    *slot = value.to_string();
                    return;
                }
            }
        }
    };
    pick(&["accent"], &mut palette.accent);
    pick(&["background"], &mut palette.background);
    pick(&["dark_background", "background"], &mut palette.surface);
    pick(&["lighter_background", "selection"], &mut palette.raised);
    pick(&["bright_foreground", "foreground"], &mut palette.foreground);
    pick(&["light_foreground", "dark_foreground"], &mut palette.muted);
    pick(&["selection", "muted"], &mut palette.border);
    pick(&["red", "bright_red"], &mut palette.danger);
    pick(&["red", "bright_red"], &mut palette.red);
    pick(&["green", "bright_green"], &mut palette.green);
    pick(&["yellow", "bright_yellow"], &mut palette.yellow);
    pick(&["blue", "bright_blue"], &mut palette.blue);
    pick(&["magenta", "bright_magenta"], &mut palette.magenta);
    pick(&["cyan", "bright_cyan"], &mut palette.cyan);
    pick(&["orange", "yellow"], &mut palette.orange);
    palette
}

/// Black or white, whichever reads on `hex` (WCAG relative luminance).
pub fn readable_on(hex: &str) -> &'static str {
    if !is_hex_color(hex) {
        return "black";
    }
    let channel = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5);
    if luminance > 0.179 { "black" } else { "white" }
}

/// Calls `on_change` (debounced) whenever the Omarchy theme may have changed.
/// Runs on its own thread for the life of the process.
pub fn watch_palette(current_dir: PathBuf, on_change: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let Ok(mut watcher) = notify::recommended_watcher(move |_| {
            let _ = tx.send(());
        }) else {
            return;
        };
        loop {
            // Watch the nearest existing ancestor too, so installing Omarchy
            // or the first theme switch is picked up without a restart.
            let mut ancestor = current_dir.clone();
            while !ancestor.exists() && ancestor.pop() {}
            for path in [ancestor.clone(), current_dir.clone(), current_dir.join("theme")] {
                let _ = watcher.unwatch(&path);
                if path.exists() {
                    let _ = watcher.watch(&path, RecursiveMode::NonRecursive);
                }
            }
            if rx.recv().is_err() {
                return;
            }
            // A switch touches many files at once; let it settle.
            std::thread::sleep(Duration::from_millis(80));
            while rx.try_recv().is_ok() {}
            on_change();
        }
    });
}

/// Hyprland's effective `decoration:rounding`, asked over its IPC socket.
/// `None` outside Hyprland.
pub fn hyprland_rounding() -> Option<i32> {
    let mut stream = UnixStream::connect(hyprland_socket_dir()?.join(".socket.sock")).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    stream.write_all(b"j/getoption decoration:rounding").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    parse_rounding(&reply)
}

pub fn parse_rounding(reply: &str) -> Option<i32> {
    let value: serde_json::Value = serde_json::from_str(reply).ok()?;
    let rounding = value.get("int")?.as_i64()?;
    (rounding >= 0).then_some(rounding as i32)
}

fn hyprland_socket_dir() -> Option<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(PathBuf::from(runtime).join("hypr").join(signature))
}

/// Calls `on_change` whenever Hyprland reloads its config, which is when the
/// rounding can change. Listens on the event socket rather than polling.
pub fn watch_hyprland_config(on_change: impl Fn() + Send + 'static) {
    let Some(dir) = hyprland_socket_dir() else { return };
    std::thread::spawn(move || {
        loop {
            if let Ok(stream) = UnixStream::connect(dir.join(".socket2.sock")) {
                for line in BufReader::new(stream).lines() {
                    match line {
                        Ok(event) if event.starts_with("configreloaded>>") => on_change(),
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            }
            // Hyprland restarted or isn't up yet; try again shortly.
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_without_omarchy() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_palette(dir.path()), Palette::default());
    }

    #[test]
    fn reads_the_omarchy_palette() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("theme")).unwrap();
        std::fs::write(
            dir.path().join("theme/colors.toml"),
            "accent = \"#e68e0d\"\nbackground = \"#121212\"\nlighter_background = \"#1e1e1e\"\nforeground = \"nope\"\n",
        )
        .unwrap();
        let palette = read_palette(dir.path());
        assert_eq!(palette.accent, "#e68e0d");
        assert_eq!(palette.background, "#121212");
        assert_eq!(palette.raised, "#1e1e1e");
        assert_eq!(palette.foreground, Palette::default().foreground);
    }

    #[test]
    fn defaults_to_omarchy_only_where_it_is_installed() {
        assert_eq!(ThemeChoice::effective(None, true), ThemeChoice::Omarchy);
        assert_eq!(ThemeChoice::effective(None, false), ThemeChoice::System);
        assert_eq!(ThemeChoice::effective(Some(ThemeChoice::Omarchy), false), ThemeChoice::System);
        assert_eq!(ThemeChoice::effective(Some(ThemeChoice::Light), true), ThemeChoice::Light);
    }

    #[test]
    fn system_follows_the_desktop_preference() {
        assert_eq!(resolve(ThemeChoice::System, true), ThemeSource::Dark);
        assert_eq!(resolve(ThemeChoice::System, false), ThemeSource::Light);
        assert_eq!(resolve(ThemeChoice::Dark, false), ThemeSource::Dark);
        assert_eq!(resolve(ThemeChoice::Omarchy, false), ThemeSource::Omarchy);
    }

    #[test]
    fn corners_follow_hyprland_then_the_theme() {
        assert_eq!(corner_radius(ThemeSource::Dark, Some(0)), 0);
        assert_eq!(corner_radius(ThemeSource::Light, Some(10)), 10);
        assert_eq!(corner_radius(ThemeSource::Omarchy, None), 0);
        assert_eq!(corner_radius(ThemeSource::Dark, None), 6);
    }

    #[test]
    fn theme_names_round_trip() {
        for choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::parse(choice.key()), Some(choice));
        }
        assert_eq!(ThemeChoice::parse(" Dark "), Some(ThemeChoice::Dark));
        assert_eq!(ThemeChoice::parse("solarized"), None);
    }

    #[test]
    fn built_in_themes_pick_readable_text() {
        assert_eq!(readable_on(&Palette::light().background), "black");
        assert_eq!(readable_on(&Palette::dark().background), "white");
    }

    #[test]
    fn omarchy_is_installed_when_its_colors_exist() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!omarchy_installed(dir.path()));
        std::fs::create_dir(dir.path().join("theme")).unwrap();
        std::fs::write(dir.path().join("theme/colors.toml"), "").unwrap();
        assert!(omarchy_installed(dir.path()));
    }

    #[test]
    fn picks_readable_text() {
        assert_eq!(readable_on("#FFD60A"), "black");
        assert_eq!(readable_on("#112233"), "white");
    }

    #[test]
    fn parses_hyprctl_rounding() {
        assert_eq!(parse_rounding(r#"{"option":"decoration:rounding","int":8,"set":true}"#), Some(8));
        assert_eq!(parse_rounding("garbage"), None);
    }
}
