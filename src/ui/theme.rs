//! The Omarchy palette and Hyprland rounding as a global, updated live when
//! the theme changes. The component library's theme follows it, so inputs,
//! scrollbars and tooltips match the rest of the window.

use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::*;

use crate::backend::markdown;
use crate::backend::theme::{self, Palette};

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
    /// Hyprland's rounding; 0 (sharp) outside Hyprland.
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

    /// Whether the background is dark (most Omarchy themes are).
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

fn apply(palette: Palette, radius: Option<i32>, cx: &mut App) {
    let radius = radius.unwrap_or_else(|| cx.try_global::<Colors>().map(|c| c.radius.as_f32() as i32).unwrap_or(0));
    let colors = Colors::from_palette(&palette, radius);
    sync_kit(&colors, cx);
    cx.set_global(colors);
    cx.refresh_windows();
}

enum Change {
    Palette(Palette),
    Radius(i32),
}

/// Reads the palette and rounding, and follows them while the app runs.
pub fn init(cx: &mut App) {
    let dir = theme::omarchy_current_dir();
    // Corners stay sharp outside Hyprland, like Omarchy's default.
    apply(theme::read_palette(&dir), Some(theme::hyprland_rounding().unwrap_or(0)), cx);
    // Test builds read the theme once: the watchers wake the UI from their
    // own threads, which GPUI's test scheduler rejects (see `ui::queue`).
    if cfg!(test) {
        return;
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Change>();
    let palette_tx = tx.clone();
    let palette_dir = dir.clone();
    theme::watch_palette(dir, move || {
        let _ = palette_tx.send(Change::Palette(theme::read_palette(&palette_dir)));
    });
    theme::watch_hyprland_config(move || {
        if let Some(radius) = theme::hyprland_rounding() {
            let _ = tx.send(Change::Radius(radius));
        }
    });
    cx.spawn(async move |cx| {
        while let Some(change) = rx.recv().await {
            cx.update(|cx| match change {
                Change::Palette(p) => apply(p, None, cx),
                Change::Radius(r) => {
                    let mut c = colors(cx).clone();
                    c.radius = px(r.max(0) as f32);
                    sync_kit(&c, cx);
                    cx.set_global(c);
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}
