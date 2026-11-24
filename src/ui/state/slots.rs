//! Every bot's chat stays loaded. The open chat lives in `AppState`'s own
//! fields; the others wait in `parked`, each a `ChatSlot`, and keep taking
//! their sessions' events, so switching bots swaps a slot in rather than
//! reading the chat again.

use std::time::Instant;

use gpui_kit::{Context, ListAlignment, ListState, px};
use serde_json::Value;

use crate::backend::attachments;

use super::{AppEvent, AppState, Role, Row, Usage};

/// One bot's conversation, as `AppState` holds the open one.
pub(super) struct ChatSlot {
    rows: Vec<Row>,
    list: ListState,
    assistant_row: Option<usize>,
    todo_row: Option<usize>,
    last_user_index: Option<usize>,
    editing: Option<u64>,
    thinking_since: Option<Instant>,
    live_session: String,
    active_session: String,
    streaming: bool,
    chat_loading: bool,
    session_info: Value,
    usage: Option<Usage>,
    pending: Vec<attachments::Attachment>,
    /// The session may have moved where this app didn't see it (another
    /// client, a cron run); it's checked when the chat is opened.
    pub(super) stale: bool,
}

impl ChatSlot {
    pub(super) fn new() -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(1000.));
        list.set_follow_mode(gpui_kit::FollowMode::Tail);
        ChatSlot {
            rows: Vec::new(),
            list,
            assistant_row: None,
            todo_row: None,
            last_user_index: None,
            editing: None,
            thinking_since: None,
            live_session: String::new(),
            active_session: String::new(),
            streaming: false,
            chat_loading: false,
            session_info: Value::Null,
            usage: None,
            pending: Vec::new(),
            stale: false,
        }
    }

    pub(super) fn is_settled(&self) -> bool {
        !self.streaming && !self.chat_loading
    }
}

/// What a transcript says, for telling whether a fresh read differs from
/// what's shown: the words of each message, in order.
fn words<'a>(rows: impl Iterator<Item = (Role, &'a str)>) -> Vec<(Role, String)> {
    rows.filter(|(role, _)| matches!(role, Role::User | Role::Assistant))
        .map(|(role, text)| (role, text.trim().to_string()))
        .filter(|(_, text)| !text.is_empty())
        .collect()
}

/// True when `fresh` (history rows) says something other than `shown`.
pub(super) fn differs(shown: &[Row], fresh: &[Value]) -> bool {
    let role = |v: &Value| match v.get("role").and_then(Value::as_str) {
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        _ => Role::Info,
    };
    let fresh = words(fresh.iter().map(|v| (role(v), v.get("text").and_then(Value::as_str).unwrap_or_default())));
    words(shown.iter().map(|r| (r.role, r.text.as_str()))) != fresh
}

impl AppState {
    /// Trades the open chat for `slot`.
    fn swap_chat(&mut self, slot: &mut ChatSlot) {
        use std::mem::swap;
        swap(&mut self.rows, &mut slot.rows);
        swap(&mut self.list, &mut slot.list);
        swap(&mut self.assistant_row, &mut slot.assistant_row);
        swap(&mut self.todo_row, &mut slot.todo_row);
        swap(&mut self.last_user_index, &mut slot.last_user_index);
        swap(&mut self.editing, &mut slot.editing);
        swap(&mut self.thinking_since, &mut slot.thinking_since);
        swap(&mut self.live_session, &mut slot.live_session);
        swap(&mut self.active_session, &mut slot.active_session);
        swap(&mut self.streaming, &mut slot.streaming);
        swap(&mut self.chat_loading, &mut slot.chat_loading);
        swap(&mut self.session_info, &mut slot.session_info);
        swap(&mut self.usage, &mut slot.usage);
        swap(&mut self.pending, &mut slot.pending);
    }

    /// Puts the open chat away under its bot, leaving empty fields.
    pub(super) fn park_active(&mut self) {
        if self.active_bot.is_empty() {
            return;
        }
        let mut slot = ChatSlot::new();
        self.swap_chat(&mut slot);
        self.parked.insert(self.active_bot.clone(), slot);
    }

    /// Brings `profile`'s parked chat out; false when it has none.
    pub(super) fn unpark(&mut self, profile: &str) -> Option<bool> {
        let mut slot = self.parked.remove(profile)?;
        let stale = slot.stale;
        self.swap_chat(&mut slot);
        Some(stale)
    }

    /// Runs `f` on `profile`'s chat as if it were open: the open one, or a
    /// parked one swapped in for the call. `None` when the bot has no chat.
    pub(super) fn with_bot_chat<R>(
        &mut self,
        profile: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> Option<R> {
        if self.active_bot == profile {
            return Some(f(self, cx));
        }
        let mut slot = self.parked.remove(profile)?;
        self.swap_chat(&mut slot);
        let open = std::mem::replace(&mut self.active_bot, profile.to_string());
        let was_backstage = std::mem::replace(&mut self.backstage, true);
        let result = f(self, cx);
        self.backstage = was_backstage;
        self.active_bot = open;
        self.swap_chat(&mut slot);
        self.parked.insert(profile.to_string(), slot);
        Some(result)
    }

    /// The bot whose chat runs on live session `live`, open or parked.
    pub(super) fn bot_for_live(&self, live: &str) -> Option<String> {
        if live.is_empty() {
            return None;
        }
        if self.live_session == live {
            return Some(self.active_bot.clone());
        }
        self.parked.iter().find(|(_, s)| s.live_session == live).map(|(bot, _)| bot.clone())
    }

    /// `with_bot_chat` for the chat on live session `live`.
    pub(super) fn with_live_chat<R>(
        &mut self,
        live: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> Option<R> {
        let bot = self.bot_for_live(live)?;
        self.with_bot_chat(&bot, cx, f)
    }

    /// Loads every bot's chat that isn't loaded yet, out of sight.
    pub(super) fn preload_chats(&mut self, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .bots
            .iter()
            .map(|b| b.name.clone())
            .filter(|n| *n != self.active_bot && !self.parked.contains_key(n))
            .collect();
        for name in names {
            let mut slot = ChatSlot::new();
            slot.chat_loading = true;
            self.parked.insert(name.clone(), slot);
            self.spawn_open(name, cx);
        }
    }

    /// Something changed in the sessions' stores: parked chats that aren't
    /// busy check themselves when next opened.
    pub(super) fn mark_parked_stale(&mut self) {
        for slot in self.parked.values_mut().filter(|s| s.is_settled()) {
            slot.stale = true;
        }
    }

    /// Re-reads the open chat's history and swaps it in only when it says
    /// something new, without clearing what's shown first.
    pub(super) fn recheck_chat(&mut self, _cx: &mut Context<Self>) {
        let (profile, stored) = (self.active_bot.clone(), self.active_session.clone());
        if stored.is_empty() || !self.chat_is_settled() {
            return;
        }
        let generation = self.generation;
        let queue = self.queue.clone();
        crate::backend::runtime().spawn(async move {
            let Some(session) = crate::backend::session::current() else { return };
            let Ok(fresh) = crate::backend::history::load(&session, &stored, &profile).await else { return };
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.with_bot_chat(&profile, cx, |app, cx| {
                    if app.active_session != stored || !app.chat_is_settled() || !differs(&app.rows, &fresh) {
                        return;
                    }
                    app.load_transcript(fresh);
                    if !app.backstage {
                        cx.emit(AppEvent::TranscriptLoaded);
                    }
                    cx.notify();
                });
            });
        });
    }

    fn chat_is_settled(&self) -> bool {
        !self.streaming && !self.chat_loading && self.editing.is_none()
    }

    /// Forgets bots the server no longer has; the open one gives way to
    /// another bot. Bots that appeared are loaded.
    pub(super) fn sync_bots(&mut self, before: &[String], cx: &mut Context<Self>) {
        let now: Vec<String> = self.bots.iter().map(|b| b.name.clone()).collect();
        let gone: Vec<String> = before.iter().filter(|n| !now.contains(n)).cloned().collect();
        for name in &gone {
            self.parked.remove(name);
            let key = self.queue_key(name);
            self.queues.remove(&key);
            self.catalogs.remove(name);
            self.running.retain(|_, bot| bot != name);
        }
        if gone.contains(&self.active_bot) {
            let removed = std::mem::take(&mut self.active_bot);
            self.status = format!("{removed} was deleted on the server.");
            if let Some(server_id) = self.config.active_server.clone() {
                if self.config.last_bot.get(&server_id) == Some(&removed) {
                    self.config.last_bot.remove(&server_id);
                    self.save_config();
                }
            }
            let pick = self
                .bots
                .iter()
                .find(|b| b.is_default)
                .or(self.bots.first())
                .map(|b| b.name.clone());
            // The deleted bot's chat is dropped, not parked.
            let mut dropped = ChatSlot::new();
            self.swap_chat(&mut dropped);
            match pick {
                Some(name) => self.show_bot(&name, cx),
                None => {
                    self.refresh_bot_extras(cx);
                    cx.emit(AppEvent::TranscriptLoaded);
                }
            }
        }
        if !gone.is_empty() || now.len() != before.len() {
            self.preload_chats(cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_fresh_read_with_the_same_words_is_no_change() {
        let mut user = Row::new(1, Role::User);
        user.text = "hi".into();
        let mut answer = Row::new(2, Role::Assistant);
        answer.text = "hello ".into();
        let tool = Row::new(3, Role::Tool);
        let shown = vec![user, tool, answer];
        let same = vec![
            json!({ "role": "user", "text": "hi" }),
            json!({ "role": "tool", "name": "x" }),
            json!({ "role": "assistant", "text": "hello" }),
        ];
        assert!(!differs(&shown, &same));
        let more = [same.clone(), vec![json!({ "role": "user", "text": "again" })]].concat();
        assert!(differs(&shown, &more));
    }
}
