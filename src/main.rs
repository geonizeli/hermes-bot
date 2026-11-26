//! hermes-bot: chat with Hermes bots on remote Hermes servers.

mod backend;
mod ui;

use gpui_kit::*;

actions!(hermes_bot, [Quit, CloseWindow]);

/// The compositor draws the frame where it can; GPUI falls back to drawing
/// it in the window where it can't (GNOME on Wayland).
/// `HERMES_BOT_DECORATIONS=client` asks for the window's own title bar; most
/// compositors honour that, though Hyprland keeps its own frame.
fn decorations() -> WindowDecorations {
    match std::env::var("HERMES_BOT_DECORATIONS").as_deref() {
        Ok("client") => WindowDecorations::Client,
        _ => WindowDecorations::Server,
    }
}

/// Quit and close-window shortcuts. gpui-kit leaves these to the app so it
/// can confirm before quitting; we quit and close straight away.
fn bind_window_shortcuts(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("alt-f4", CloseWindow, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
    // macOS menu bar: Quit / Close pick up the keybindings above.
    cx.set_menus([
        Menu::new("Hermes Bot").items([MenuItem::action("Quit Hermes Bot", Quit)]),
        Menu::new("Window").items([MenuItem::action("Close Window", CloseWindow)]),
    ]);
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        ui::theme::init(cx);
        bind_window_shortcuts(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(None, size(px(1200.), px(820.)), cx)),
            window_min_size: Some(size(px(720.), px(480.))),
            titlebar: Some(TitlebarOptions { title: Some("Hermes Bot".into()), ..Default::default() }),
            // Matches the window to hermes-bot.desktop, so Hyprland (Wayland
            // app_id) and the launcher pick up the installed icon.
            app_id: Some("hermes-bot".into()),
            window_decorations: Some(decorations()),
            ..Default::default()
        };
        if let Err(e) = gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| ui::root::RootView::new(window, cx))) {
            eprintln!("hermes-bot: couldn't open the window: {e}");
            cx.quit();
        }
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
