//! hermes-bot: chat with Hermes bots on remote Hermes servers.

mod backend;
mod ui;

use gpui_kit::*;

actions!(hermes_bot, [Quit, CloseWindow, NewWindow]);

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

fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(None, size(px(1200.), px(820.)), cx)),
        window_min_size: Some(size(px(720.), px(480.))),
        titlebar: Some(TitlebarOptions { title: Some("Hermes Bot".into()), ..Default::default() }),
        // Matches the window to hermes-bot.desktop, so Hyprland (Wayland
        // app_id) and the launcher pick up the installed icon.
        app_id: Some("hermes-bot".into()),
        window_decorations: Some(decorations()),
        ..Default::default()
    }
}

fn open_main_window(cx: &mut App) -> bool {
    match gpui_kit::open_window(window_options(cx), cx, |window, cx| {
        cx.new(|cx| ui::root::RootView::new(window, cx))
    }) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("hermes-bot: couldn't open the window: {e}");
            false
        }
    }
}

/// Quit, close-window, and new-window shortcuts. gpui-kit leaves these to the
/// app so it can confirm before quitting; we quit and close straight away.
fn bind_window_shortcuts(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &CloseWindow, cx| {
        // Key dispatch already holds the active window; nested update would
        // fail with "window not found". Close after this update finishes.
        let Some(window) = cx.active_window() else {
            return;
        };
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        });
    });
    cx.on_action(|_: &NewWindow, cx| {
        cx.defer(|cx| {
            let _ = open_main_window(cx);
        });
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-n", NewWindow, None),
        KeyBinding::new("alt-f4", CloseWindow, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-n", NewWindow, None),
    ]);
    // macOS menu bar: Quit / Close / New pick up the keybindings above.
    cx.set_menus([
        Menu::new("Hermes Bot").items([MenuItem::action("Quit Hermes Bot", Quit)]),
        Menu::new("File").items([MenuItem::action("New Window", NewWindow)]),
        Menu::new("Window").items([MenuItem::action("Close Window", CloseWindow)]),
    ]);
}

fn main() {
    let app = gpui_kit::application();
    // On macOS the red traffic light closes the window only; GPUI's default
    // QuitMode keeps the app in the Dock. Reopen restores the window.
    #[cfg(target_os = "macos")]
    app.on_reopen(|cx| {
        if cx.windows().is_empty() && !open_main_window(cx) {
            cx.quit();
        }
    });
    app.run(|cx| {
        gpui_kit::init(cx);
        ui::theme::init(cx);
        bind_window_shortcuts(cx);
        if !open_main_window(cx) {
            cx.quit();
        }
    });
}
