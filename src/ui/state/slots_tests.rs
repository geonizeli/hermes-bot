//! Every bot's chat stays loaded: switching swaps chats, parked chats keep
//! taking their events, and bots or groups deleted on the server leave.

use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::{Value, json};

use super::{AppState, Bot, Role};
use crate::backend::gateway::Incoming;
use crate::backend::groups::Room;
use crate::backend::store::Config;

fn bot(name: &str, is_default: bool) -> Bot {
    Bot { name: name.into(), display_name: name.into(), is_default, ..Default::default() }
}

/// Two bots, "a" open on live-a with one message; "b" parked on live-b.
fn state(cx: &mut TestAppContext) -> (Entity<AppState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.json");
    let state = cx.new(|cx| {
        let mut s = AppState::with_config(Config::default(), path, cx);
        s.bots = vec![bot("a", true), bot("b", false)];
        s.active_bot = "b".into();
        s.live_session = "live-b".into();
        s.on_user_message("to b", Vec::new());
        s.park_active();
        s.active_bot = "a".into();
        s.live_session = "live-a".into();
        s.on_user_message("to a", Vec::new());
        s
    });
    (state, dir)
}

fn event(state: &Entity<AppState>, cx: &mut TestAppContext, kind: &str, session: &str, payload: Value) {
    state.update(cx, |s, cx| {
        s.handle_incoming(0, Incoming::Event { kind: kind.into(), session_id: session.into(), payload }, cx)
    });
}

fn texts(state: &Entity<AppState>, cx: &mut TestAppContext) -> Vec<String> {
    state.read_with(cx, |s, _| s.rows.iter().map(|r| r.text.clone()).collect())
}

#[gpui_kit::test]
fn switching_bots_keeps_each_chat(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, cx| s.select_bot("b", cx));
    assert_eq!(texts(&state, cx), vec!["to b"]);
    assert!(state.read_with(cx, |s, _| s.live_session == "live-b" && !s.chat_loading));
    state.update(cx, |s, cx| s.select_bot("a", cx));
    assert_eq!(texts(&state, cx), vec!["to a"]);
    assert!(state.read_with(cx, |s, _| s.live_session == "live-a"));
}

#[gpui_kit::test]
fn a_parked_chat_streams_out_of_sight(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    state.update(cx, |s, _| s.mark_running("live-b", "b"));
    event(&state, cx, "message.delta", "live-b", json!({ "text": "Hel" }));
    event(&state, cx, "message.delta", "live-b", json!({ "text": "lo" }));
    // The open chat is untouched.
    assert_eq!(texts(&state, cx), vec!["to a"]);
    event(&state, cx, "message.complete", "live-b", json!({ "text": "Hello", "status": "complete" }));
    assert!(state.read_with(cx, |s, _| !s.is_busy("b") && s.bots[1].preview == "Hello"));

    state.update(cx, |s, cx| s.select_bot("b", cx));
    assert_eq!(texts(&state, cx), vec!["to b", "Hello"]);
    let roles: Vec<Role> = state.read_with(cx, |s, _| s.rows.iter().map(|r| r.role).collect());
    assert_eq!(roles, vec![Role::User, Role::Assistant]);
    assert!(state.read_with(cx, |s, _| s.rows[1].done && !s.streaming));
}

#[gpui_kit::test]
fn a_parked_error_stays_in_its_chat(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    event(&state, cx, "error", "live-b", json!({ "message": "boom" }));
    assert!(state.read_with(cx, |s, _| s.error.is_empty()));
    state.update(cx, |s, cx| s.select_bot("b", cx));
    assert_eq!(texts(&state, cx), vec!["to b", "boom"]);
}

#[gpui_kit::test]
fn a_bot_deleted_on_the_server_leaves(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    // "b" (parked) goes.
    state.update(cx, |s, cx| {
        s.bots.retain(|b| b.name != "b");
        s.sync_bots(&["a".into(), "b".into()], cx);
    });
    assert!(state.read_with(cx, |s, _| !s.parked.contains_key("b") && s.active_bot == "a"));
    assert_eq!(texts(&state, cx), vec!["to a"]);

    // The open one goes: the default bot takes over.
    state.update(cx, |s, cx| {
        s.bots = vec![bot("c", false), bot("d", true)];
        s.sync_bots(&["a".into()], cx);
    });
    assert!(state.read_with(cx, |s, _| s.active_bot == "d" && s.rows.is_empty() && s.status.contains("a was deleted")));
}

#[gpui_kit::test]
fn a_group_disbanded_on_the_server_closes(cx: &mut TestAppContext) {
    let (state, _dir) = state(cx);
    let room = |id: &str| Room::from_value(&json!({ "room_id": id, "name": format!("Room {id}") }));
    state.update(cx, |s, cx| {
        s.groups.rooms = vec![room("r1"), room("r2")];
        s.select_group("r1", cx);
    });
    // A room created a moment ago and not listed yet doesn't close.
    state.update(cx, |s, cx| {
        s.active_group = Some("new".into());
        s.apply_rooms(vec![room("r1"), room("r2")], cx);
        s.active_group = Some("r1".into());
    });
    assert!(state.read_with(cx, |s, _| s.active_group.as_deref() == Some("r1")));
    state.update(cx, |s, cx| s.apply_rooms(vec![room("r2")], cx));
    assert!(state.read_with(cx, |s, _| s.active_group.is_none() && s.groups.rooms.len() == 1));
    assert!(state.read_with(cx, |s, _| s.status.contains("Room r1 was disbanded")));
    // The bot chat behind it is still there.
    assert_eq!(texts(&state, cx), vec!["to a"]);
}
