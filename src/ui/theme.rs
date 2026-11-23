//! The window's colours as a global: a built-in dark or light theme (or the
//! one the desktop prefers), or the live Omarchy theme where it's installed.
//! Updated live when the choice, the desktop's preference, the Omarchy theme
//! or Hyprland's rounding changes. The component library's theme follows it,
//! so inputs, scrollbars and tooltips match the rest of the window.

use std::path::PathBuf;

use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::*;

use crate::backend::markdown;
use crate::backend::theme::{self, Palette, ThemeChoice, ThemeSource};

#[derive(Clone)]
pub struct Colors {
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    pub background: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub danger: Hsla,
    pub red: Hsla,
    pub green: Hsla,
    pub yellow: Hsla,
    pub blue: Hsla,
    pub magenta: Hsla,
    pub cyan: Hsla,
    pub orange: Hsla,
    /// Hyprland's rounding under Hyprland, otherwise the theme's.
    pub radius: Pixels,
    /// The same palette as strings, for the Markdown renderer.
    pub markdown: markdown::Colors,
}

impl Global for Colors {}

pub fn hex(s: &str) -> Hsla {
    Rgba::try_from(s.trim()).map(Hsla::from).unwrap_or_else(|_| white())
}

/// `top` laid over `base` at `amount` (0..1).
pub fn tint(base: Hsla, top: Hsla, amount: f32) -> Hsla {
    base.blend(top.alpha(amount))
}

impl Colors {
    fn from_palette(p: &Palette, radius: i32) -> Self {
        Colors {
            accent: hex(&p.accent),
            accent_foreground: hex(theme::readable_on(&p.accent)),
            background: hex(&p.background),
            surface: hex(&p.surface),
            raised: hex(&p.raised),
            foreground: hex(&p.foreground),
            muted: hex(&p.muted),
            border: hex(&p.border),
            danger: hex(&p.danger),
            red: hex(&p.red),
            green: hex(&p.green),
            yellow: hex(&p.yellow),
            blue: hex(&p.blue),
            magenta: hex(&p.magenta),
            cyan: hex(&p.cyan),
            orange: hex(&p.orange),
            radius: px(radius.max(0) as f32),
            markdown: markdown::Colors {
                foreground: p.foreground.clone(),
                muted: p.muted.clone(),
                accent: p.accent.clone(),
                surface: p.surface.clone(),
                border: p.border.clone(),
                red: p.red.clone(),
                green: p.green.clone(),
                yellow: p.yellow.clone(),
                blue: p.blue.clone(),
                magenta: p.magenta.clone(),
                cyan: p.cyan.clone(),
                orange: p.orange.clone(),
            },
        }
    }

    /// Whether the background is dark.
    pub fn is_dark(&self) -> bool {
        self.background.l < 0.5
    }
}

pub fn colors(cx: &App) -> &Colors {
    cx.global::<Colors>()
}

/// Paints the component library with the palette.
fn sync_kit(c: &Colors, cx: &mut App) {
    let mode = if c.is_dark() { ThemeMode::Dark } else { ThemeMode::Light };
    if KitTheme::global(cx).mode != mode {
        KitTheme::change(mode, None, cx);
    }
    let c = c.clone();
    KitTheme::update(cx, move |t| {
        t.font_size = px(14.);
        t.radius = c.radius;
        t.radius_lg = c.radius;
        t.shadow = false;
        t.focus_ring = false;
        t.background = c.background;
        t.foreground = c.foreground;
        t.border = c.border;
        t.input = c.border;
        t.ring = c.accent.alpha(0.6);
        t.caret = c.accent;
        t.selection = c.accent.alpha(0.35);
        t.muted = c.surface;
        t.muted_foreground = c.muted;
        t.popover = c.raised;
        t.popover_foreground = c.foreground;
        t.accent = c.raised;
        t.accent_foreground = c.foreground;
        t.primary = c.accent;
        t.primary_hover = c.accent.opacity(0.9);
        t.primary_active = c.accent.opacity(0.8);
        t.primary_foreground = c.accent_foreground;
        t.secondary = c.raised;
        t.secondary_hover = tint(c.raised, c.foreground, 0.06);
        t.secondary_active = tint(c.raised, c.foreground, 0.1);
        t.secondary_foreground = c.foreground;
        t.danger = c.danger;
        t.danger_foreground = readable_on(c.danger);
        // The drawn title bar and frame (desktops that leave decorations to
        // the app).
        t.title_bar = c.surface;
        t.title_bar_border = c.border;
        t.window_border = c.border;
        t.window_radius = c.radius;
        t.link = c.blue;
        t.colors.list = c.surface;
        t.list_hover = c.raised;
        t.list_active = c.accent.alpha(0.18);
        t.list_active_border = c.accent.alpha(0.6);
        t.scrollbar = transparent_black();
        t.scrollbar_thumb = c.muted.alpha(0.35);
        t.scrollbar_thumb_hover = c.muted.alpha(0.6);
        t.button = c.raised;
        t.button_hover = tint(c.raised, c.foreground, 0.06);
        t.button_active = tint(c.raised, c.foreground, 0.1);
        t.button_foreground = c.foreground;
        t.button_primary = c.accent;
        t.button_primary_hover = c.accent.opacity(0.9);
        t.button_primary_active = c.accent.opacity(0.8);
        t.button_primary_foreground = c.accent_foreground;
    });
}

/// What the colours are made from. A global so the theme menu can show the
/// current choice and whether Omarchy's theme is there to pick.
#[derive(Clone, Debug)]
pub struct ThemeState {
    /// The choice saved in the settings; `None` is the default.
    pub saved: Option<ThemeChoice>,
    /// The Omarchy theme's palette, when Omarchy is installed.
    omarchy: Option<Palette>,
    /// Hyprland's rounding, when running under Hyprland.
    hyprland: Option<i32>,
    /// The desktop prefers dark.
    system_dark: bool,
}

impl Global for ThemeState {}

impl ThemeState {
    pub fn omarchy_available(&self) -> bool {
        self.omarchy.is_some()
    }

    /// The choice in effect, with the default worked out.
    pub fn effective(&self) -> ThemeChoice {
        ThemeChoice::effective(self.saved, self.omarchy_available())
    }

    /// The choices worth offering: Omarchy only where it's installed.
    pub fn choices(&self) -> Vec<ThemeChoice> {
        ThemeChoice::ALL.into_iter().filter(|c| *c != ThemeChoice::Omarchy || self.omarchy_available()).collect()
    }

    fn colors(&self) -> Colors {
        let source = theme::resolve(self.effective(), self.system_dark);
        let palette = match source {
            ThemeSource::Omarchy => self.omarchy.clone().unwrap_or_default(),
            ThemeSource::Dark => Palette::dark(),
            ThemeSource::Light => Palette::light(),
        };
        Colors::from_palette(&palette, theme::corner_radius(source, self.hyprland))
    }
}

/// Black or white, whichever reads on `color`.
fn readable_on(color: Hsla) -> Hsla {
    let rgb = color.to_rgb();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    hex(theme::readable_on(&format!("#{:02x}{:02x}{:02x}", byte(rgb.r), byte(rgb.g), byte(rgb.b))))
}

pub fn theme_state(cx: &App) -> &ThemeState {
    cx.global::<ThemeState>()
}

/// Repaints the window from the theme state after any of its inputs changed.
fn update(cx: &mut App, f: impl FnOnce(&mut ThemeState)) {
    let mut state = cx.global::<ThemeState>().clone();
    f(&mut state);
    let colors = state.colors();
    cx.set_global(state);
    sync_kit(&colors, cx);
    cx.set_global(colors);
    cx.refresh_windows();
}

/// Switches to `choice` (`None`: the default). Saving it is the caller's job.
pub fn set_choice(choice: Option<ThemeChoice>, cx: &mut App) {
    if cx.try_global::<ThemeState>().is_some_and(|s| s.saved != choice) {
        update(cx, |s| s.saved = choice);
    }
}

/// Follows the desktop's light/dark preference.
pub fn set_appearance(appearance: WindowAppearance, cx: &mut App) {
    let dark = is_dark(appearance);
    if cx.try_global::<ThemeState>().is_some_and(|s| s.system_dark != dark) {
        update(cx, |s| s.system_dark = dark);
    }
}

fn is_dark(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

fn read_omarchy(dir: &PathBuf) -> Option<Palette> {
    theme::omarchy_installed(dir).then(|| theme::read_palette(dir))
}

enum Change {
    Omarchy(Option<Palette>),
    Radius(i32),
}

/// Sets up the theme with the default choice (the saved one arrives with
/// the settings) and follows its sources while the app runs.
pub fn init(cx: &mut App) {
    let dir = theme::omarchy_current_dir();
    let state = ThemeState {
        saved: None,
        omarchy: read_omarchy(&dir),
        hyprland: theme::hyprland_rounding(),
        system_dark: is_dark(cx.window_appearance()),
    };
    cx.set_global(state);
    update(cx, |_| {});
    // Test builds read the theme once: the watchers wake the UI from their
    // own threads, which GPUI's test scheduler rejects (see `ui::queue`).
    if cfg!(test) {
        return;
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Change>();
    let palette_tx = tx.clone();
    let palette_dir = dir.clone();
    // Watched even when Omarchy isn't there, so installing it is noticed.
    theme::watch_palette(dir, move || {
        let _ = palette_tx.send(Change::Omarchy(read_omarchy(&palette_dir)));
    });
    theme::watch_hyprland_config(move || {
        if let Some(radius) = theme::hyprland_rounding() {
            let _ = tx.send(Change::Radius(radius));
        }
    });
    cx.spawn(async move |cx| {
        while let Some(change) = rx.recv().await {
            cx.update(|cx| match change {
                Change::Omarchy(palette) => update(cx, |s| s.omarchy = palette),
                Change::Radius(r) => update(cx, |s| s.hyprland = Some(r)),
            });
        }
    })
    .detach();
}
