//! The real views in a headless window, driven by clicks and keys.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext as _, Bounds, Entity, Point, TestAppContext, Window, WindowBounds, WindowOptions, px, size};
use serde_json::json;

use super::root::RootView;
use super::state::{AppEvent, AppState, Bot, Provider, Role};
use crate::backend::store::{Config, Server};

pub struct Harness {
    pub window: gpui_kit::AnyWindowHandle,
    pub root: Entity<RootView>,
    pub state: Entity<AppState>,
    _dir: tempfile::TempDir,
}

pub fn open(cx: &mut TestAppContext) -> Harness {
    let config = Config {
        servers: vec![Server { id: "s1".into(), name: "Test server".into(), url: "https://hermes.invalid".into(), provider: String::new(), username: String::new() }],
        active_server: Some("s1".into()),
        ..Default::default()
    };
    open_with(cx, config, false)
}

/// `live` connects for real instead of pretending to be connected.
pub fn open_with(cx: &mut TestAppContext, config: Config, live: bool) -> Harness {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::theme::init(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.json");
    let (window, root) = cx.update(|cx| {
        let (window, root) = gpui_kit::open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1400.), px(900.)) })), ..Default::default() },
            cx,
            |window, cx| {
                let state = cx.new(|cx| {
                    let mut s = AppState::with_config(config, path, cx);
                    if live {
                        s.reconnect(cx);
                        return s;
                    }
                    s.connected = true;
                    s.active_bot = "default".into();
                    s.bots = vec![Bot { name: "default".into(), display_name: "Hermes".into(), model: "m1".into(), ..Default::default() }];
                    s
                });
                cx.new(|cx| RootView::with_state(state, window, cx))
            },
        )
        .unwrap();
        (window, root)
    });
    let state = root.read_with(cx, |r, _| r.state.clone());
    Harness { window, root, state, _dir: dir }
}

impl Harness {
    /// Runs `f` against the window after a fresh frame.
    pub fn ui<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .unwrap()
    }

    pub fn focus_composer(&self, cx: &mut TestAppContext) {
        let composer = self.root.read_with(cx, |r, cx| r.chat.read(cx).composer.clone());
        self.ui(cx, |window, cx| composer.update(cx, |c, cx| c.focus(window, cx)));
    }

    pub fn composer_text(&self, cx: &mut TestAppContext) -> String {
        self.root.read_with(cx, |r, cx| r.chat.read(cx).composer.read(cx).input.read(cx).value().to_string())
    }
}

fn catalog() -> crate::backend::slash::Catalog {
    crate::backend::slash::Catalog::from_json(&json!({
        "categories": [{ "name": "Info", "pairs": [["/help", "Show the commands you can use here"], ["/heartbeat", "Set a recurring prompt"]] }],
        "pairs": [],
        "commands": { "/help": {}, "/heartbeat": {} },
    }))
}

#[gpui_kit::test]
fn the_composer_completes_and_runs_commands(cx: &mut TestAppContext) {
    let h = open(cx);
    h.state.update(cx, |s, _| {
        s.catalogs.insert("default".into(), catalog());
    });
    h.focus_composer(cx);
    h.ui(cx, |w, cx| w.input("/he", cx));
    h.ui(cx, |w, _| {
        assert!(w.try_find("slash-menu").is_some(), "the command menu opens on /");
        assert!(w.try_find("slash-1").is_some());
    });
    // Down picks the second row, Tab puts it in the box.
    h.ui(cx, |w, cx| {
        w.press("down", cx);
        w.press("up", cx);
        w.press("tab", cx);
    });
    assert_eq!(h.composer_text(cx).trim(), "/help");
    // Enter runs the completed command; /help answers locally.
    h.ui(cx, |w, cx| w.press("enter", cx));
    h.ui(cx, |w, _| assert!(w.try_find("slash-menu").is_none(), "the menu closes with the box empty"));
    assert_eq!(h.composer_text(cx), "");
    h.state.read_with(cx, |s, _| {
        let row = s.rows.last().expect("a command row");
        assert_eq!(row.role, Role::Command);
        assert_eq!(row.name, "/help");
        assert!(row.text.contains("/heartbeat"), "{}", row.text);
    });
    // Escape dismisses the menu without clearing the text.
    h.ui(cx, |w, cx| w.input("/h", cx));
    h.ui(cx, |w, cx| w.press("escape", cx));
    h.ui(cx, |w, _| assert!(w.try_find("slash-menu").is_none()));
    assert_eq!(h.composer_text(cx), "/h");
}

#[gpui_kit::test]
fn sending_needs_a_connection(cx: &mut TestAppContext) {
    let h = open(cx);
    h.focus_composer(cx);
    h.ui(cx, |w, cx| w.input("hello", cx));
    h.ui(cx, |w, cx| w.click("send", cx));
    h.state.read_with(cx, |s, _| {
        assert_eq!(s.error, "Not connected.");
        assert!(s.rows.is_empty());
    });
    // The error bar shows and dismisses.
    h.ui(cx, |w, cx| w.click("dismiss-error", cx));
    assert_eq!(h.state.read_with(cx, |s, _| s.error.clone()), "");
}

#[gpui_kit::test]
fn the_pickers_change_the_chat_settings(cx: &mut TestAppContext) {
    let h = open(cx);
    h.state.update(cx, |s, cx| {
        s.models = vec![Provider {
            slug: "p".into(),
            name: "Provider".into(),
            models: vec!["m1".into(), "m2".into(), "other".into()],
            authenticated: true,
            capabilities: json!({ "m1": { "fast": true, "can_disable_reasoning": true } }),
        }];
        s.session_info = json!({ "model": "m1", "provider": "p", "reasoning_effort": "high" });
        cx.notify();
    });
    h.ui(cx, |w, cx| w.click("model-chip", cx));
    h.ui(cx, |w, _| assert!(w.try_find("model-picker").is_some()));
    h.ui(cx, |w, cx| w.input("m2", cx));
    h.ui(cx, |w, _| {
        assert!(w.try_find("model-0").is_some());
        assert!(w.try_find("model-1").is_none(), "the search filters models");
    });
    h.ui(cx, |w, cx| w.click("model-0", cx));
    h.ui(cx, |w, _| assert!(w.try_find("model-picker").is_none()));
    // Without a live chat the pick has nowhere to go.
    assert_eq!(h.state.read_with(cx, |s, _| s.error.clone()), "Not connected.");

    h.state.update(cx, |s, _| s.error.clear());
    h.ui(cx, |w, cx| w.click("effort-chip", cx));
    h.ui(cx, |w, _| {
        assert!(w.try_find("popup-menu").is_some());
        // none, minimal, low, medium, high, xhigh, max
        assert!(w.try_find("menu-item-6").is_some());
    });
    h.ui(cx, |w, cx| w.click("menu-item-0", cx));
    h.ui(cx, |w, _| assert!(w.try_find("popup-menu").is_none()));
    assert_eq!(h.state.read_with(cx, |s, _| s.error.clone()), "Not connected.");

    h.state.update(cx, |s, _| s.error.clear());
    h.ui(cx, |w, cx| w.click("fast-chip", cx));
    assert_eq!(h.state.read_with(cx, |s, _| s.error.clone()), "Not connected.");

    // An expensive model asks first.
    h.state.update(cx, |_, cx| cx.emit(AppEvent::ModelConfirm { message: "Pricey".into(), model: "m2".into(), provider: "p".into() }));
    h.ui(cx, |w, _| assert!(w.try_find("confirm-model").is_some()));
    h.ui(cx, |w, cx| w.click("confirm-cancel", cx));
    h.ui(cx, |w, _| assert!(w.try_find("confirm-model").is_none()));
}

fn with_transcript(h: &Harness, cx: &mut TestAppContext) -> (u64, u64) {
    h.state.update(cx, |s, cx| {
        s.load_transcript(vec![
            json!({ "role": "user", "text": "first question", "rowId": 1, "timestamp": 1.0 }),
            json!({ "role": "assistant", "text": "An **answer** with `code`\n\n```rust\nfn main() {}\n```", "reasoning": "hmm" }),
            json!({ "role": "tool", "name": "terminal", "args": "{\"command\": \"ls\"}", "result": "a.txt" }),
        ]);
        cx.notify();
    });
    h.state.read_with(cx, |s, _| (s.rows[0].id, s.rows[1].id))
}

#[gpui_kit::test]
fn messages_can_be_edited_regenerated_and_expanded(cx: &mut TestAppContext) {
    let h = open(cx);
    let (user, answer) = with_transcript(&h, cx);
    let tool = h.state.read_with(cx, |s, _| s.rows[2].id);

    h.ui(cx, |w, cx| w.click(super::widgets::id("edit", user), cx));
    let editing = h.root.read_with(cx, |r, cx| r.chat.read(cx).composer.read(cx).editing);
    assert!(editing);
    assert_eq!(h.composer_text(cx), "first question");
    h.ui(cx, |w, _| assert!(w.try_find("cancel-edit").is_some()));
    h.ui(cx, |w, cx| w.click("cancel-edit", cx));
    assert!(!h.root.read_with(cx, |r, cx| r.chat.read(cx).composer.read(cx).editing));
    assert_eq!(h.composer_text(cx), "");

    // Regenerate resends the last question (here: nowhere to send it).
    h.ui(cx, |w, cx| w.click(super::widgets::id("regenerate", answer), cx));
    h.state.read_with(cx, |s, _| assert_eq!(s.rows.len(), 3));

    h.ui(cx, |w, cx| w.click(super::widgets::id("reasoning-toggle", answer), cx));
    assert!(h.state.read_with(cx, |s, _| s.rows[1].show_reasoning));
    h.ui(cx, |w, cx| w.click(super::widgets::id("tool-chip", tool), cx));
    assert!(h.state.read_with(cx, |s, _| s.rows[2].expanded));
    h.ui(cx, |w, _| assert!(w.try_find(super::widgets::id("tool-body", tool)).is_some()));

    // Copy puts the answer on the clipboard.
    h.ui(cx, |w, cx| w.click(super::widgets::id("copy-answer", answer), cx));
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert!(copied.is_some_and(|t| t.contains("**answer**")));
}

#[gpui_kit::test]
fn attachments_show_as_chips_until_removed(cx: &mut TestAppContext) {
    let h = open(cx);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "hello").unwrap();
    h.state.update(cx, |s, cx| s.add_files(&[file], cx));
    let id = h.state.read_with(cx, |s, _| s.pending[0].id);
    h.ui(cx, |w, cx| w.click(super::widgets::id("remove-attachment", id), cx));
    assert!(h.state.read_with(cx, |s, _| s.pending.is_empty()));
}

#[gpui_kit::test]
fn the_bot_s_questions_get_answers(cx: &mut TestAppContext) {
    let h = open(cx);
    let ask = |h: &Harness, cx: &mut TestAppContext, method: &str, params: serde_json::Value| {
        let method = method.to_string();
        h.state.update(cx, |_, cx| cx.emit(AppEvent::ServerRequest { id: json!(1), method, params }));
    };
    let open_now = |h: &Harness, cx: &mut TestAppContext| h.root.read_with(cx, |r, cx| r.request.read(cx).open);

    ask(&h, cx, "approval", json!({ "command": "rm -rf build", "description": "Clean up" }));
    assert!(open_now(&h, cx));
    h.ui(cx, |w, _| {
        assert!(w.try_find("approve-deny").is_some());
    });
    h.ui(cx, |w, cx| w.click("approve-once", cx));
    assert!(!open_now(&h, cx));

    ask(&h, cx, "clarify", json!({ "questions": [
        { "qid": "q1", "question": "Which?", "choices": ["a", "b"] },
        { "qid": "q2", "question": "Why?" },
    ]}));
    h.ui(cx, |w, cx| w.click("choice-0-1", cx));
    h.ui(cx, |w, cx| w.click("clarify-answer", cx));
    assert!(!open_now(&h, cx));

    ask(&h, cx, "sudo", json!({ "prompt": "Password for sudo" }));
    h.ui(cx, |w, cx| w.input("secret", cx));
    h.ui(cx, |w, cx| w.press("enter", cx));
    assert!(!open_now(&h, cx));

    // Requests only the official desktop understands are declined at once.
    ask(&h, cx, "desktop.only", json!({}));
    assert!(!open_now(&h, cx));
}

#[gpui_kit::test]
fn servers_and_profiles_open_from_the_sidebar(cx: &mut TestAppContext) {
    let h = open(cx);
    h.ui(cx, |w, cx| w.click("server-switcher", cx));
    h.ui(cx, |w, _| assert!(w.try_find("popup-menu").is_some()));
    // Test server, separator, Reconnect? (connected: no), Manage servers…
    h.ui(cx, |w, cx| w.click("menu-item-2", cx));
    assert!(h.root.read_with(cx, |r, cx| r.setup.read(cx).open));
    h.ui(cx, |w, _| {
        assert!(w.try_find("resign-s1").is_some());
        assert!(w.try_find("check-server").is_some());
    });
    h.ui(cx, |w, cx| w.click("setup-close", cx));
    assert!(!h.root.read_with(cx, |r, cx| r.setup.read(cx).open));

    h.ui(cx, |w, cx| w.right_click("bot-default", cx));
    h.ui(cx, |w, cx| w.click("menu-item-0", cx));
    assert!(h.root.read_with(cx, |r, cx| r.edit.read(cx).open));
    h.ui(cx, |w, cx| {
        assert!(w.try_find("edit-tab-Pet").is_some());
        w.click("edit-tab-Pet", cx);
    });
    h.ui(cx, |w, cx| w.click("edit-tab-Bot", cx));
    h.ui(cx, |w, cx| w.click("edit-save", cx));
    h.ui(cx, |w, _| assert!(w.try_find("edit-bot-dialog").is_some(), "a failed save keeps the dialog"));
    h.ui(cx, |w, cx| w.click("edit-cancel", cx));
    assert!(!h.root.read_with(cx, |r, cx| r.edit.read(cx).open));

    // The header's identity opens the same editor.
    h.ui(cx, |w, cx| w.click("bot-identity", cx));
    assert!(h.root.read_with(cx, |r, cx| r.edit.read(cx).open));
}

#[gpui_kit::test]
fn the_side_panel_shows_jobs_and_the_screen(cx: &mut TestAppContext) {
    let h = open(cx);
    h.state.update(cx, |s, cx| {
        s.jobs = crate::backend::cron::jobs_for_bot(
            &json!({ "scoped": "default", "jobs": [{ "job_id": "j1", "name": "Daily brief", "schedule": "0 9 * * *", "deliver": "chat" }] }),
            "default",
        );
        cx.notify();
    });
    h.ui(cx, |w, _| assert!(w.try_find("job-j1").is_some(), "the jobs panel is open by default"));
    h.ui(cx, |w, cx| w.click("job-j1", cx));
    let expanded = h.root.read_with(cx, |r, cx| r.chat.read(cx).jobs_expanded.read(cx).contains("j1"));
    assert!(expanded);

    h.ui(cx, |w, cx| w.click("toggle-screen", cx));
    h.state.update(cx, |s, cx| {
        s.screen = json!({ "running": true, "lease": { "holder": "agent" } });
        cx.notify();
    });
    h.ui(cx, |w, _| {
        assert!(w.try_find("screen-holder").is_some());
        assert!(w.try_find("screen-stop").is_some());
    });
    // No session to watch through: the view says so and offers to reconnect.
    let screen = h.root.read_with(cx, |r, cx| r.chat.read(cx).screen.clone());
    assert_eq!(screen.read_with(cx, |v, _| v.message.clone()), "Not connected.");
    h.ui(cx, |w, _| assert!(w.try_find("screen-reconnect").is_some()));
    h.ui(cx, |w, cx| w.click("screen-expand", cx));
    h.ui(cx, |w, _| assert!(w.try_find("expanded-screen").is_some()));
    h.ui(cx, |w, cx| w.click("collapse-screen", cx));
    h.ui(cx, |w, _| assert!(w.try_find("expanded-screen").is_none()));

    h.ui(cx, |w, cx| w.click("toggle-screen", cx));
    h.ui(cx, |w, _| assert!(w.try_find("side-panel").is_none(), "the same toggle closes it"));
}

/// Waits (in real time) for `done`, letting network results land.
fn wait_for(h: &Harness, cx: &mut TestAppContext, seconds: u64, done: impl Fn(&AppState) -> bool) -> bool {
    for _ in 0..seconds * 10 {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        if h.state.read_with(cx, |s, _| done(s)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// Sends a real message to the `default` bot through the composer and waits
/// for the streamed answer. Run with
/// `HERMES_BOT_UI_LIVE=1 cargo test ui::tests::live -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn live_send_streams_an_answer(cx: &mut TestAppContext) {
    if std::env::var("HERMES_BOT_UI_LIVE").is_err() {
        return;
    }
    let mut config = crate::backend::store::load(&crate::backend::store::config_path()).unwrap();
    let server = config.active_server.clone().expect("a saved server");
    config.last_bot.insert(server, "default".into());
    let h = open_with(cx, config, true);
    assert!(
        wait_for(&h, cx, 60, |s| s.connected && !s.chat_loading && s.active_bot == "default"),
        "connect: {}",
        h.state.read_with(cx, |s, _| format!("{} / {}", s.status, s.error))
    );
    let before = h.state.read_with(cx, |s, _| s.rows.len());
    println!("opened the default bot's chat with {before} rows");
    h.ui(cx, |w, _| assert!(w.try_find("send").is_some()));

    h.focus_composer(cx);
    h.ui(cx, |w, cx| w.input("Reply with just the word pong.", cx));
    h.ui(cx, |w, cx| w.press("enter", cx));
    assert!(h.state.read_with(cx, |s, _| s.streaming), "sending starts a turn: {}", h.state.read_with(cx, |s, _| s.error.clone()));

    let finished = wait_for(&h, cx, 180, |s| {
        let answer = s.rows.iter().skip(before).rev().find(|r| r.role == Role::Assistant);
        !s.streaming && answer.is_some_and(|a| a.done && !a.text.is_empty())
    });
    let (rows, error) = h.state.read_with(cx, |s, _| {
        (s.rows.iter().skip(before).map(|r| format!("{:?}: {}", r.role, r.text)).collect::<Vec<_>>(), s.error.clone())
    });
    println!("new rows: {rows:#?}");
    assert!(finished, "the answer finishes (error: {error})");
    let answer = rows.iter().rev().find(|r| r.starts_with("Assistant")).unwrap();
    assert!(answer.to_lowercase().contains("pong"), "{answer}");
    assert!(rows.iter().any(|r| r.starts_with("User: Reply with just the word pong.")), "the sent message shows as a bubble");
    // GPUI's test scheduler rejects wake-ups from the network thread, which
    // the real app relies on; close the socket before anything else arrives.
    h.state.update(cx, |s, _| s.disconnect());
}

/// Watches the `default` bot's screen when it's already running (never
/// starts it or takes control). Run with
/// `HERMES_BOT_UI_LIVE=1 cargo test ui::tests::live_screen -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn live_screen_shows_frames(cx: &mut TestAppContext) {
    if std::env::var("HERMES_BOT_UI_LIVE").is_err() {
        return;
    }
    let mut config = crate::backend::store::load(&crate::backend::store::config_path()).unwrap();
    let server = config.active_server.clone().expect("a saved server");
    config.last_bot.insert(server, "default".into());
    config.ui.side_panel_open = false;
    let h = open_with(cx, config, true);
    assert!(wait_for(&h, cx, 60, |s| s.connected && !s.chat_loading && s.active_bot == "default"));
    assert!(wait_for(&h, cx, 30, |s| !s.screen.is_null() && !s.screen_busy), "the screen status arrives");
    let status = h.state.read_with(cx, |s, _| s.screen.clone());
    println!("screen status: {status}");
    let was_running = status.get("running").and_then(|v| v.as_bool()) == Some(true);
    h.ui(cx, |w, cx| w.click("toggle-screen", cx));
    if !was_running {
        // HERMES_BOT_SCREEN_START=1 starts it for the test and stops it after.
        if std::env::var("HERMES_BOT_SCREEN_START").is_err() {
            println!("the default bot's screen is off; not starting it");
            h.state.update(cx, |s, _| s.disconnect());
            return;
        }
        h.ui(cx, |w, cx| w.click("screen-start", cx));
        assert!(
            wait_for(&h, cx, 90, |s| s.screen.get("running").and_then(|v| v.as_bool()) == Some(true)),
            "the screen starts: {}",
            h.state.read_with(cx, |s, _| format!("{} {}", s.screen, s.error))
        );
    }
    let screen = h.root.read_with(cx, |r, cx| r.chat.read(cx).screen.clone());
    let mut live = false;
    for _ in 0..300 {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        if screen.read_with(cx, |v, _| v.state == super::screen::ScreenState::Live && v.image.is_some()) {
            live = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let (w, hgt, message) = screen.read_with(cx, |v, _| (v.frame_width, v.frame_height, v.message.clone()));
    println!("screen {w}x{hgt} live={live} message={message:?}");
    assert!(live, "frames arrive: {message}");
    assert!(!screen.read_with(cx, |v, _| v.in_control), "watching starts without control");
    // Take control and hand it back (the lease round-trip).
    let in_control = |cx: &mut TestAppContext| screen.read_with(cx, |v, _| v.in_control);
    h.ui(cx, |w, cx| w.click("screen-control", cx));
    let took = (0..100).any(|_| {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(100));
        in_control(cx)
    });
    println!("took control: {took} ({})", screen.read_with(cx, |v, _| v.message.clone()));
    assert!(took);
    h.ui(cx, |w, cx| w.click("screen-control", cx));
    let gave = (0..100).any(|_| {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(100));
        !in_control(cx)
    });
    println!("gave it back: {gave}");
    assert!(gave);
    if !was_running {
        h.ui(cx, |w, cx| w.click("screen-stop", cx));
        let stopped = wait_for(&h, cx, 60, |s| !s.screen_busy && s.screen.get("running").and_then(|v| v.as_bool()) == Some(false));
        println!("stopped again: {stopped}");
        assert!(stopped);
    }
    h.ui(cx, |w, cx| w.click("toggle-screen", cx));
    h.state.update(cx, |s, _| s.disconnect());
}

#[gpui_kit::test]
fn loading_shows_placeholders_where_content_will_be(cx: &mut TestAppContext) {
    let h = open(cx);
    h.state.update(cx, |s, cx| {
        s.chat_loading = true;
        s.bots.clear();
        cx.notify();
    });
    h.ui(cx, |w, _| {
        assert!(w.try_find("chat-loading").is_some(), "the chat shows its loading shape");
    });

    h.state.update(cx, |s, cx| {
        s.chat_loading = false;
        s.bots = vec![Bot { name: "default".into(), display_name: "Hermes".into(), ..Default::default() }];
        cx.notify();
    });
    h.ui(cx, |w, _| {
        assert!(w.try_find("chat-loading").is_none(), "the placeholders go once the chat opens");
        assert!(w.try_find(super::widgets::id("bot", "default")).is_some());
    });
}

#[gpui_kit::test]
fn the_theme_switches_and_is_remembered(cx: &mut TestAppContext) {
    use crate::backend::theme::ThemeChoice;
    let h = open(cx);
    let dark = |cx: &mut TestAppContext| cx.read(|cx| super::theme::colors(cx).is_dark());

    h.state.update(cx, |s, cx| s.set_theme(ThemeChoice::Light, cx));
    assert!(!dark(cx));
    assert_eq!(h.state.read_with(cx, |s, _| s.config.ui.theme.clone()), Some("light".into()));

    // "Match system" follows the desktop's preference as it changes.
    h.state.update(cx, |s, cx| s.set_theme(ThemeChoice::System, cx));
    cx.update(|cx| super::theme::set_appearance(gpui_kit::WindowAppearance::Dark, cx));
    assert!(dark(cx));
    cx.update(|cx| super::theme::set_appearance(gpui_kit::WindowAppearance::Light, cx));
    assert!(!dark(cx));

    // /skin switches too, and says what it did.
    h.focus_composer(cx);
    h.ui(cx, |w, cx| w.input("/skin dark", cx));
    h.ui(cx, |w, cx| w.press("enter", cx));
    assert!(dark(cx));
    h.state.read_with(cx, |s, _| {
        let row = s.rows.last().expect("a command row");
        assert_eq!(row.name, "/skin");
        assert_eq!(row.text, "Theme: Dark.");
        assert_eq!(s.config.ui.theme.as_deref(), Some("dark"));
    });
}

#[gpui_kit::test]
fn the_window_draws_its_frame_only_where_the_compositor_does_not(cx: &mut TestAppContext) {
    let h = open(cx);
    h.ui(cx, |w, _| {
        assert!(w.try_find("title-bar").is_none(), "a compositor-drawn frame needs no title bar of ours");
        assert!(w.try_find("root").is_some());
    });

    // GNOME on Wayland leaves the frame to the app.
    cx.simulate_window_decorations_change(h.window, gpui_kit::Decorations::Client { tiling: Default::default() });
    h.ui(cx, |w, _| {
        assert!(w.try_find("title-bar").is_some(), "the window draws its own title bar");
        assert!(w.try_find("close").is_some(), "with a close button");
        assert!(w.try_find("root").is_some(), "the chat is still there");
    });

    cx.simulate_window_decorations_change(h.window, gpui_kit::Decorations::Server);
    h.ui(cx, |w, _| assert!(w.try_find("title-bar").is_none()));
}

#[gpui_kit::test]
fn the_queue_button_and_panel(cx: &mut TestAppContext) {
    let h = open(cx);
    h.ui(cx, |w, _| assert!(w.try_find("queue").is_none(), "nothing to queue behind while idle"));
    h.state.update(cx, |s, cx| {
        s.streaming = true;
        cx.notify();
    });
    h.focus_composer(cx);
    h.ui(cx, |w, cx| w.input("run the tests after this", cx));
    // Enter would steer; the Queue button sends after the answer instead.
    h.ui(cx, |w, cx| w.click("queue", cx));
    assert_eq!(h.composer_text(cx), "");
    let id = h.state.read_with(cx, |s, _| {
        let queue = s.queued().expect("a queued message");
        assert_eq!(queue.items[0].text, "run the tests after this");
        queue.items[0].id
    });
    h.ui(cx, |w, _| {
        assert!(w.try_find("queue-panel").is_some());
        assert!(w.try_find(super::widgets::id("queued-steer", id)).is_some(), "plain text can steer now");
    });

    // Edit takes it back into the message box.
    h.ui(cx, |w, cx| w.click(super::widgets::id("queued-edit", id), cx));
    assert_eq!(h.composer_text(cx), "run the tests after this");
    h.ui(cx, |w, _| assert!(w.try_find("queue-panel").is_none()));

    // Queue it again, then remove it.
    h.ui(cx, |w, cx| w.click("queue", cx));
    let id = h.state.read_with(cx, |s, _| s.queued().unwrap().items[0].id);
    h.ui(cx, |w, cx| w.click(super::widgets::id("queued-remove", id), cx));
    assert!(h.state.read_with(cx, |s, _| s.queued().is_none()));
}
