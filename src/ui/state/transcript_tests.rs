//! The conversation model driven by gateway events, as a live turn sends them.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::{Value, json};

use super::{AppEvent, AppState, Bot, Role};
use crate::backend::gateway::Incoming;
use crate::backend::store::Config;

fn state(cx: &mut TestAppContext) -> (Entity<AppState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.json");
    let state = cx.new(|cx| {
        let mut s = AppState::with_config(Config::default(), path, cx);
        s.active_bot = "default".into();
        s.bots = vec![Bot { name: "default".into(), display_name: "Hermes".into(), ..Default::default() }];
        s.live_session = "live-1".into();
        s
    });
    (state, dir)
}

fn event(state: &Entity<AppState>, cx: &mut TestAppContext, kind: &str, session: &str, payload: Value) {
    state.update(cx, |s, cx| {
        s.handle_incoming(0, Incoming::Event { kind: kind.into(), session_id: session.into(), payload }, cx)
    });
}

fn roles(state: &Entity<AppState>, cx: &mut TestAppContext) -> Vec<Role> {
    state.read_with(cx, |s, _| s.rows.iter().map(|r| r.role).collect())
}

#[gpui_kit::test]
fn streams_a_turn_into_rows(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, _| {
        s.on_user_message("hi", Vec::new());
        s.streaming = true;
        s.mark_running("live-1", "default");
    });
    assert!(state.read_with(cx, |s, _| s.waiting_for_answer() && s.is_busy("default")));

    event(&state, cx, "message.start", "live-1", json!({}));
    event(&state, cx, "reasoning.delta", "live-1", json!({ "text": "let me " }));
    event(&state, cx, "reasoning.delta", "live-1", json!({ "text": "think" }));
    event(&state, cx, "message.delta", "live-1", json!({ "text": "Hel" }));
    event(&state, cx, "message.delta", "live-1", json!({ "text": "lo" }));
    event(&state, cx, "tool.start", "live-1", json!({ "tool_id": "t1", "name": "terminal", "args": { "command": "ls" } }));
    assert!(state.read_with(cx, |s, _| !s.rows[2].done));
    event(&state, cx, "tool.complete", "live-1", json!({ "tool_id": "t1", "name": "terminal", "result": { "output": "a.txt" }, "duration_s": 1.25 }));
    event(&state, cx, "message.delta", "live-1", json!({ "text": "Done" }));
    event(&state, cx, "todo.updated", "live-1", json!({ "todos": [{ "content": "ship", "status": "in_progress" }] }));
    event(&state, cx, "todo.updated", "live-1", json!({ "todos": [{ "content": "ship", "status": "completed" }] }));
    event(&state, cx, "session.usage", "live-1", json!({ "context_tokens": 5000, "context_length": 10000 }));
    event(&state, cx, "message.complete", "live-1", json!({ "text": "Done.", "status": "complete" }));

    assert_eq!(roles(&state, cx), vec![Role::User, Role::Assistant, Role::Tool, Role::Assistant, Role::Todos]);
    state.read_with(cx, |s, _| {
        assert_eq!(s.rows[1].text, "Hello");
        assert_eq!(s.rows[1].reasoning, "let me think");
        assert!(s.rows[1].done);
        assert_eq!(s.rows[2].result, "a.txt");
        assert!(s.rows[2].args.contains("\"command\": \"ls\""));
        assert_eq!(s.rows[2].duration, 1.25);
        assert!(s.rows[2].done);
        // The final text replaces what streamed.
        assert_eq!(s.rows[3].text, "Done.");
        assert_eq!(s.rows[4].todos.len(), 1);
        assert_eq!(s.rows[4].todos[0].status, "completed");
        assert!(!s.streaming);
        assert!(!s.is_busy("default"));
        assert_eq!(s.usage.as_ref().map(|u| u.percent.round()), Some(50.0));
        assert_eq!(s.bots[0].preview, "Done.");
        assert_eq!(s.list.item_count(), s.rows.len());
    });
}

#[gpui_kit::test]
fn ends_turns_with_errors_and_stops(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, _| s.on_user_message("go", Vec::new()));
    event(&state, cx, "tool.start", "live-1", json!({ "tool_id": "t1", "name": "web_search" }));
    event(&state, cx, "message.complete", "live-1", json!({ "status": "error", "error": "rate limited" }));
    event(&state, cx, "message.complete", "live-1", json!({ "status": "interrupted" }));
    assert_eq!(roles(&state, cx), vec![Role::User, Role::Tool, Role::Error, Role::Info]);
    state.read_with(cx, |s, _| {
        // A tool left running when the turn ends is closed.
        assert!(s.rows[1].done);
        assert_eq!(s.rows[2].text, "rate limited");
        assert_eq!(s.rows[3].text, "Stopped");
    });
}

#[gpui_kit::test]
fn finishes_background_turns_in_the_sidebar(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, _| {
        s.bots.push(Bot { name: "work".into(), display_name: "Work".into(), ..Default::default() });
        s.mark_running("live-work", "work");
    });
    assert!(state.read_with(cx, |s, _| s.is_busy("work")));
    // Another chat's events don't touch the open one.
    event(&state, cx, "message.delta", "live-work", json!({ "text": "x" }));
    event(&state, cx, "message.complete", "live-work", json!({ "text": "Report ready" }));
    state.read_with(cx, |s, _| {
        assert!(s.rows.is_empty());
        assert!(!s.is_busy("work"));
        assert_eq!(s.bots[1].preview, "Report ready");
    });
}

#[gpui_kit::test]
fn shows_commands_and_drops_silent_ones(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, _| {
        s.on_command_started("/usage");
        s.on_command_output("/usage", "12k tokens");
        s.on_command_started("/heartbeat");
        s.on_command_output("/heartbeat", "");
        s.on_steered("look at b", "queued");
    });
    assert_eq!(roles(&state, cx), vec![Role::Command, Role::Info]);
    state.read_with(cx, |s, _| {
        assert_eq!(s.rows[0].text, "12k tokens");
        assert!(s.rows[0].done);
        assert_eq!(s.rows[1].text, "↪ look at b");
        assert_eq!(s.list.item_count(), 2);
    });
}

#[gpui_kit::test]
fn edit_drops_the_last_message_and_what_followed(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, _| {
        s.load_transcript(vec![
            json!({ "role": "user", "text": "first", "rowId": 1 }),
            json!({ "role": "assistant", "text": "one" }),
            json!({ "role": "user", "text": "second", "rowId": 3, "attachments": "[{\"kind\":\"image\",\"name\":\"a.png\",\"path\":\"/srv/a.png\"}]" }),
            json!({ "role": "assistant", "text": "two" }),
        ]);
    });
    state.read_with(cx, |s, _| {
        assert_eq!(s.last_user_index, Some(2));
        assert_eq!(s.rows[2].attachments[0].path, "/srv/a.png");
        assert!(s.rows[2].attachments[0].is_image());
    });
    state.update(cx, |s, _| s.truncate_rows(2));
    state.read_with(cx, |s, _| {
        assert_eq!(s.rows.len(), 2);
        assert_eq!(s.last_user_index, Some(0));
        assert_eq!(s.list.item_count(), 2);
    });
    // Without a stored id there's nothing to truncate on the server.
    state.update(cx, |s, cx| {
        s.rows[0].row_id = -1;
        s.resend_last("again", cx);
    });
    state.read_with(cx, |s, _| {
        assert!(s.error.contains("can't be changed yet"));
        assert_eq!(s.rows.len(), 2);
    });
}

#[gpui_kit::test]
fn passes_server_requests_on_and_reconnects_when_closed(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = seen.clone();
    cx.update(|cx| {
        cx.subscribe(&state, move |_, event: &AppEvent, _| {
            if let AppEvent::ServerRequest { method, .. } = event {
                log.borrow_mut().push(method.clone());
            }
        })
        .detach()
    });
    state.update(cx, |s, cx| {
        s.handle_incoming(0, Incoming::ServerRequest { id: json!(7), method: "approval".into(), params: json!({}) }, cx)
    });
    assert_eq!(*seen.borrow(), vec!["approval".to_string()]);

    state.update(cx, |s, cx| {
        s.connected = true;
        s.streaming = true;
        s.handle_incoming(0, Incoming::Closed("socket closed".into()), cx);
    });
    state.read_with(cx, |s, _| {
        assert!(!s.connected && !s.streaming);
        assert_eq!(s.status, "Reconnecting…");
    });
    // Stale generations are ignored.
    state.update(cx, |s, cx| s.handle_incoming(99, Incoming::Closed("x".into()), cx));
    assert_eq!(state.read_with(cx, |s, _| s.error.clone()), "socket closed");
}

#[gpui_kit::test]
fn tracks_the_screen_from_display_events(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    event(&state, cx, "display.status", "", json!({ "profile": "default", "profile_key": "k1", "running": true }));
    event(&state, cx, "display.lease", "", json!({ "profile_key": "k1", "lease": { "holder": "human" } }));
    event(&state, cx, "display.install.log", "", json!({ "profile_key": "k1", "line": "apt install x" }));
    event(&state, cx, "display.install.done", "", json!({ "profile_key": "k1", "code": 0 }));
    event(&state, cx, "display.status", "", json!({ "profile": "work", "running": false }));
    state.read_with(cx, |s, _| {
        assert_eq!(s.screen["running"], true);
        assert_eq!(s.screen["lease"]["holder"], "human");
        assert_eq!(s.screen_log, "apt install x\nInstalled.\n");
    });
}

#[gpui_kit::test]
fn remembers_the_side_panel_in_its_own_config(cx: &mut TestAppContext) {
    let (state, dir) = state(cx);
    state.update(cx, |s, cx| s.toggle_side_panel("screen", cx));
    assert_eq!(state.read_with(cx, |s, _| s.side_panel().1.to_string()), "screen");
    state.update(cx, |s, cx| s.toggle_side_panel("screen", cx));
    let saved = crate::backend::store::load(&dir.path().join("servers.json")).unwrap();
    assert!(!saved.ui.side_panel_open);
    assert_eq!(saved.ui.side_panel_tab, "screen");
}

#[gpui_kit::test]
fn takes_pasted_images_and_files_as_attachments(cx: &mut TestAppContext) {
    let (state, dir) = state(cx);
    let png = b"\x89PNG\r\n\x1a\n0000".to_vec();
    let image = gpui_kit::ClipboardItem::new_image(&gpui_kit::Image { format: gpui_kit::ImageFormat::Png, bytes: png.clone(), id: 1 });
    assert!(state.update(cx, |s, cx| s.paste(&image, cx)));
    let text = gpui_kit::ClipboardItem::new_string("short".into());
    assert!(!state.update(cx, |s, cx| s.paste(&text, cx)));
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "hello").unwrap();
    state.update(cx, |s, cx| s.add_files(&[file], cx));
    state.read_with(cx, |s, _| {
        assert_eq!(s.pending.len(), 2);
        assert_eq!(s.pending[0].kind, crate::backend::attachments::Kind::Image);
        assert_eq!(s.pending[1].name, "notes.txt");
    });
    let first = state.read_with(cx, |s, _| s.pending[0].id);
    state.update(cx, |s, cx| s.remove_attachment(first, cx));
    assert_eq!(state.read_with(cx, |s, _| s.pending.len()), 1);
}
