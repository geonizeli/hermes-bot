//! Small pieces the views share: avatars, chips, buttons, menus, dialogs.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, Theme as KitTheme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::motion;
use super::theme::{Colors, colors, tint};

/// An identified element. Test builds register it so headless UI tests can
/// find and click it; elsewhere this is a plain `ident(key)`.
pub type Ident = gpui_kit::base::ObservedElement<Stateful<Div>>;

pub fn ident(key: impl Into<ElementId>) -> Ident {
    use gpui_kit::TestSupportExt as _;
    div().id(key).test_support()
}

pub fn id(prefix: &str, key: impl std::fmt::Display) -> ElementId {
    ElementId::Name(SharedString::from(format!("{prefix}-{key}")))
}

pub fn mono(cx: &App) -> SharedString {
    KitTheme::global(cx).mono_font_family.clone()
}

/// The colour a bot's initial sits on, from its name so each bot keeps one.
pub fn name_hue(name: &str) -> Hsla {
    let mut h: u32 = 0;
    for ch in name.encode_utf16() {
        h = (h * 31 + ch as u32) % 360;
    }
    hsla(h as f32 / 360.0, 0.45, 0.42, 1.0)
}

/// A bot's avatar: a picture cropped to a circle, or a drawn face shown as
/// it is. Until it loads, or when the bot has none, its initial on a circle.
pub fn avatar(name: &str, picture: Option<PathBuf>, face: bool, size: f32) -> AnyElement {
    let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
    let hue = name_hue(name);
    let fallback = move || {
        div()
            .size(px(size))
            .rounded_full()
            .bg(hue)
            .flex()
            .items_center()
            .justify_center()
            .text_color(white())
            .text_size(px((size * 0.44).round()))
            .font_weight(FontWeight::SEMIBOLD)
            .child(initial.clone())
            .into_any_element()
    };
    match picture {
        Some(path) => {
            let fb = fallback.clone();
            let fb2 = fallback.clone();
            img(path)
                .size(px(size))
                .flex_none()
                .when(!face, |i| i.rounded_full().object_fit(ObjectFit::Cover))
                .when(face, |i| i.object_fit(ObjectFit::Contain))
                .with_fallback(move || fb())
                .with_loading(move || fb2())
                .into_any_element()
        }
        None => fallback(),
    }
}

pub fn spinner(size: f32) -> impl IntoElement {
    Spinner::new().with_size(px(size))
}

/// A dot that pulses while something runs.
pub fn pulse_dot(key: impl Into<ElementId>, color: Hsla, size: f32) -> impl IntoElement {
    div().size(px(size)).flex_none().rounded_full().bg(color).with_animation(
        key,
        Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.3, 1.0)),
        |el, delta| el.opacity(delta),
    )
}

/// A small rounded control (the composer's model and effort pickers).
pub fn chip(key: impl Into<ElementId>, text: impl Into<SharedString>, active: bool, c: &Colors) -> Ident {
    let (bg, border, fg) = if active {
        (c.accent.alpha(0.18), c.accent.alpha(0.6), c.foreground)
    } else {
        (transparent_black(), c.border, c.muted)
    };
    let hover_bg = if active { bg } else { c.surface };
    ident(key)
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(c.radius)
        .border_1()
        .border_color(border)
        .bg(bg)
        .hover(move |s| s.bg(hover_bg))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(fg)
        .max_w(px(280.))
        .overflow_hidden()
        .child(div().truncate().child(text.into()))
}

/// A square icon button with a glyph (panel headers).
pub fn glyph_button(key: impl Into<ElementId>, glyph: &'static str, tip: &'static str, c: &Colors) -> Ident {
    let hover = c.raised;
    ident(key)
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(c.radius)
        .hover(move |s| s.bg(hover))
        .cursor_pointer()
        .text_size(px(14.))
        .text_color(c.muted)
        .child(glyph)
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
}

/// A text button; `primary` fills it with the accent.
pub fn button(key: impl Into<ElementId>, text: impl Into<SharedString>, primary: bool, enabled: bool, c: &Colors) -> Ident {
    let (bg, hover, fg) = if primary {
        (c.accent, c.accent.opacity(0.88), c.accent_foreground)
    } else {
        (c.raised.alpha(0.6), c.raised, c.foreground)
    };
    ident(key)
        .h(px(30.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(c.radius)
        .bg(bg)
        .when(!primary, |d| d.border_1().border_color(c.border))
        .text_size(px(13.))
        .text_color(fg)
        .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
        .when(!enabled, |d| d.opacity(0.5))
        .child(text.into())
}

/// A muted text link that brightens on hover ("Edit", "Copy").
pub fn text_link(key: impl Into<ElementId>, text: impl Into<SharedString>, c: &Colors) -> Ident {
    let hover = c.foreground;
    ident(key)
        .text_size(px(12.))
        .text_color(c.muted)
        .cursor_pointer()
        .hover(move |s| s.text_color(hover))
        .child(text.into())
}

pub type Action = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct MenuItem {
    pub label: SharedString,
    pub checked: bool,
    pub enabled: bool,
    pub action: Option<Action>,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        MenuItem { label: label.into(), checked: false, enabled: true, action: Some(Rc::new(action)) }
    }

    pub fn separator() -> Self {
        MenuItem { label: SharedString::default(), checked: false, enabled: false, action: None }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// A popup menu at a point in the window.
pub struct PopupMenu {
    pub position: Point<Pixels>,
    pub items: Vec<MenuItem>,
    /// Opens upward from the point (menus at the bottom of the window).
    pub upward: bool,
}

impl PopupMenu {
    pub fn new(position: Point<Pixels>, items: Vec<MenuItem>) -> Self {
        PopupMenu { position, items, upward: false }
    }

    pub fn upward(mut self) -> Self {
        self.upward = true;
        self
    }

    /// Draws the menu; `close` runs when an item is picked or the user
    /// clicks elsewhere.
    pub fn render(&self, close: Action, cx: &App) -> impl IntoElement {
        let c = colors(cx);
        let any_checks = self.items.iter().any(|i| i.checked);
        let items = self.items.iter().enumerate().map(|(ix, item)| {
            if item.action.is_none() {
                return div().h(px(1.)).my(px(4.)).bg(c.border).into_any_element();
            }
            let action = item.action.clone();
            let close = close.clone();
            let hover = c.surface;
            ident(id("menu-item", ix))
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(c.radius)
                .text_size(px(13.))
                .text_color(if item.enabled { c.foreground } else { c.muted })
                .when(item.enabled, |d| {
                    d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(move |_, window, cx| {
                        close(window, cx);
                        if let Some(action) = &action {
                            action(window, cx);
                        }
                    })
                })
                .when(any_checks, |d| {
                    d.child(div().w(px(12.)).text_color(c.accent).child(if item.checked { "✓" } else { "" }))
                })
                .child(item.label.clone())
                .into_any_element()
        });
        let upward = self.upward;
        let menu = ident("popup-menu")
            .occlude()
            .min_w(px(180.))
            .p(px(4.))
            .bg(c.raised)
            .border_1()
            .border_color(c.border)
            .rounded(c.radius)
            .shadow_lg()
            .children(items)
            .on_mouse_down_out(move |_, window, cx| close(window, cx))
            // Unfolds from where it was opened.
            .with_animation("popup-menu-in", Animation::new(motion::QUICK).with_easing(motion::ease_out), move |d, t| {
                let shift = 6. * (1. - t);
                d.opacity(t).relative().top(px(if upward { shift } else { -shift }))
            });
        deferred(
            anchored()
                .position(self.position)
                .anchor(if self.upward { Anchor::BottomLeft } else { Anchor::TopLeft })
                .snap_to_window_with_margin(px(8.))
                .child(menu),
        )
        .with_priority(10)
    }
}

/// A dialog's frame: the window dimmed, the content in a centered card.
/// The dimming fades in and the card rises into place.
pub fn modal(key: &'static str, width: f32, content: impl IntoElement, c: &Colors) -> Ident {
    let scrim = div().absolute().inset_0().bg(black().alpha(0.5)).with_animation(
        SharedString::from(format!("{key}-scrim-in")),
        Animation::new(motion::BASE).with_easing(motion::ease_out),
        |d, t| d.opacity(t),
    );
    ident(key)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p(px(20.))
        .child(scrim)
        .child(
            ident(SharedString::from(format!("{key}-card")))
                .w(px(width))
                .max_w_full()
                .max_h_full()
                .flex()
                .flex_col()
                .bg(c.surface)
                .border_1()
                .border_color(c.border)
                .rounded(c.radius)
                .shadow_lg()
                .text_color(c.foreground)
                .child(content)
                .with_animation(
                    SharedString::from(format!("{key}-card-in")),
                    Animation::new(motion::BASE).with_easing(motion::ease_out),
                    |d, t| d.opacity(t).relative().top(px(16. * (1. - t))),
                ),
        )
}

/// A section heading in a dialog.
pub fn heading(text: impl Into<SharedString>, c: &Colors) -> impl IntoElement {
    div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(c.foreground).child(text.into())
}

pub fn muted(text: impl Into<SharedString>, size: f32, c: &Colors) -> Div {
    div().text_size(px(size)).text_color(c.muted).child(text.into())
}

/// A raised surface tinted towards the accent (your messages).
pub fn accent_surface(c: &Colors) -> Hsla {
    tint(c.raised, c.accent, 0.14)
}
