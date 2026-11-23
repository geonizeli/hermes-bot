//! Queued messages against a fake gateway that records what it's asked.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::{AppState, Bot, MessageQueue, QueuedMessage};
use crate::backend::gateway::{Gateway, Incoming};
use crate::backend::runtime;
use crate::backend::store::Config;

pub(super) type Requests = Arc<Mutex<Vec<Value>>>;

/// A gateway that answers every request with `{status: "streaming"}` and
/// keeps each request for the test to read.
fn fake_gateway() -> (Arc<Gateway>, Requests) {
    fake_gateway_with(|_| json!({ "status": "streaming" }))
}

/// A gateway that answers each request with `reply(request)`.
pub(super) fn fake_gateway_with(reply: fn(&Value) -> Value) -> (Arc<Gateway>, Requests) {
    let requests: Requests = Arc::default();
    let seen = requests.clone();
    let gateway = runtime().block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let frame: Value = serde_json::from_str(&text).unwrap();
                let reply = json!({ "jsonrpc": "2.0", "id": frame["id"], "result": reply(&frame) });
                seen.lock().unwrap().push(frame);
                let _ = ws.send(Message::text(reply.to_string())).await;
            }
        });
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Incoming>();
        std::mem::forget(rx);
        Gateway::connect_url(&format!("ws://127.0.0.1:{port}/api/ws"), tx).await.unwrap()
    });
    (gateway, requests)
}

fn state(cx: &mut TestAppContext) -> (Entity<AppState>, Requests, tempfile::TempDir) {
    state_with(cx, fake_gateway())
}

pub(super) fn state_with(cx: &mut TestAppContext, (gateway, requests): (Arc<Gateway>, Requests)) -> (Entity<AppState>, Requests, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("servers.json");
    let state = cx.new(|cx| {
        let mut s = AppState::with_config(Config::default(), path, cx);
        s.active_bot = "default".into();
        s.bots = vec![
            Bot { name: "default".into(), display_name: "Hermes".into(), ..Default::default() },
            Bot { name: "work".into(), display_name: "Work".into(), ..Default::default() },
        ];
        s.live_session = "live-1".into();
        s.connected = true;
        s.gateway = Some(gateway);
        s
    });
    (state, requests, dir)
}

/// Starts a turn on the open chat, as sending a message does.
fn answering(state: &Entity<AppState>, cx: &mut TestAppContext) {
    state.update(cx, |s, _| {
        s.on_user_message("hi", Vec::new());
        s.streaming = true;
        s.mark_running("live-1", "default");
    });
}

fn complete(state: &Entity<AppState>, cx: &mut TestAppContext, session: &str, status: &str) {
    let payload = json!({ "text": "Done.", "status": status });
    state.update(cx, |s, cx| {
        s.handle_incoming(0, Incoming::Event { kind: "message.complete".into(), session_id: session.into(), payload }, cx)
    });
}

/// The requests named `method`, once at least `count` have arrived.
pub(super) fn wait_for(requests: &Requests, method: &str, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found: Vec<Value> =
            requests.lock().unwrap().iter().filter(|r| r["method"] == method).map(|r| r["params"].clone()).collect();
        if found.len() >= count || Instant::now() > deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Nothing named `method` arrives within a moment.
fn none_of(requests: &Requests, method: &str) -> bool {
    std::thread::sleep(Duration::from_millis(150));
    !requests.lock().unwrap().iter().any(|r| r["method"] == method)
}

fn queued_texts(state: &Entity<AppState>, cx: &mut TestAppContext) -> Vec<String> {
    state.read_with(cx, |s, _| s.queued().map(|q| q.items.iter().map(|m| m.text.clone()).collect()).unwrap_or_default())
}

#[gpui_kit::test]
fn queued_messages_go_out_in_order_as_answers_finish(cx: &mut TestAppContext) {
    let (state, requests, _dir) = state(cx);
    answering(&state, cx);
    state.update(cx, |s, cx| {
        s.queue_message("first", cx);
        s.queue_message("second", cx);
    });
    assert_eq!(queued_texts(&state, cx), ["first", "second"]);
    assert!(none_of(&requests, "prompt.submit"), "nothing goes out while the bot answers");

    complete(&state, cx, "live-1", "complete");
    let sent = wait_for(&requests, "prompt.submit", 1);
    assert_eq!(sent[0]["text"], "first");
    assert_eq!(sent[0]["queued"], true, "a queued message never becomes a correction");
    state.read_with(cx, |s, _| {
        assert!(s.streaming);
        assert_eq!(s.rows.last().unwrap().text, "first", "it shows as your message");
    });
    assert_eq!(queued_texts(&state, cx), ["second"]);

    complete(&state, cx, "live-1", "complete");
    let sent = wait_for(&requests, "prompt.submit", 2);
    assert_eq!(sent[1]["text"], "second");
    assert!(queued_texts(&state, cx).is_empty());
}

#[gpui_kit::test]
fn stopping_parks_the_queue_until_resumed(cx: &mut TestAppContext) {
    let (state, requests, _dir) = state(cx);
    answering(&state, cx);
    state.update(cx, |s, cx| s.queue_message("later", cx));
    complete(&state, cx, "live-1", "interrupted");
    assert!(state.read_with(cx, |s, _| s.queued().unwrap().parked));
    assert!(none_of(&requests, "prompt.submit"), "a stopped turn holds the queue");

    state.update(cx, |s, cx| s.resume_queue(cx));
    let sent = wait_for(&requests, "prompt.submit", 1);
    assert_eq!(sent[0]["text"], "later");
    assert!(queued_texts(&state, cx).is_empty());
}

#[gpui_kit::test]
fn a_bot_in_the_background_sends_its_queue_too(cx: &mut TestAppContext) {
    let (state, requests, _dir) = state(cx);
    state.update(cx, |s, _| {
        let key = s.queue_key("work");
        let message = QueuedMessage { id: 7, text: "check the deploy".into(), attachments: Vec::new() };
        s.queues.insert(key, MessageQueue { items: vec![message], parked: false });
        s.mark_running("live-work", "work");
    });
    complete(&state, cx, "live-work", "complete");
    let sent = wait_for(&requests, "prompt.submit", 1);
    assert_eq!(sent[0]["session_id"], "live-work");
    assert_eq!(sent[0]["profile"], "work");
    assert_eq!(sent[0]["queued"], true);
    state.read_with(cx, |s, _| {
        assert!(s.is_busy("work"), "the bot is answering its queued message");
        assert!(s.rows.is_empty(), "the open chat is untouched");
    });
}

#[gpui_kit::test]
fn a_queued_message_can_steer_or_go_now(cx: &mut TestAppContext) {
    let (state, requests, _dir) = state(cx);
    answering(&state, cx);
    state.update(cx, |s, cx| s.queue_message("also check the logs", cx));
    let id = state.read_with(cx, |s, _| s.queued().unwrap().items[0].id);
    state.update(cx, |s, cx| s.steer_queued(id, cx));
    let steered = wait_for(&requests, "session.steer", 1);
    assert_eq!(steered[0]["text"], "also check the logs");
    assert!(queued_texts(&state, cx).is_empty());

    // Idle, a queued message can be sent at once.
    state.update(cx, |s, cx| {
        s.queue_message("one more", cx);
        s.streaming = false;
    });
    let id = state.read_with(cx, |s, _| s.queued().unwrap().items[0].id);
    state.update(cx, |s, cx| s.send_queued_now(id, cx));
    let sent = wait_for(&requests, "prompt.submit", 1);
    assert_eq!(sent[0]["text"], "one more");
}

#[gpui_kit::test]
fn queueing_while_idle_just_sends(cx: &mut TestAppContext) {
    let (state, requests, _dir) = state(cx);
    state.update(cx, |s, cx| s.queue_message("hello", cx));
    let sent = wait_for(&requests, "prompt.submit", 1);
    assert_eq!(sent[0]["text"], "hello");
    assert!(sent[0].get("queued").is_none(), "an ordinary send");
    assert!(queued_texts(&state, cx).is_empty());
}

#[gpui_kit::test]
fn editing_takes_a_message_back_with_its_attachments(cx: &mut TestAppContext) {
    let (state, _requests, dir) = state(cx);
    answering(&state, cx);
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "hello").unwrap();
    state.update(cx, |s, cx| {
        s.add_files(&[file], cx);
        s.queue_message("see attached", cx);
    });
    let id = state.read_with(cx, |s, _| {
        assert!(s.pending.is_empty(), "the attachment went with the queued message");
        s.queued().unwrap().items[0].id
    });
    let text = state.update(cx, |s, cx| s.take_queued(id, cx));
    assert_eq!(text.as_deref(), Some("see attached"));
    state.read_with(cx, |s, _| {
        assert_eq!(s.pending.len(), 1, "its attachment is waiting again");
        assert!(s.queued().is_none());
    });
}
