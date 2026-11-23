//! Bot groups in the app's state: the server's rooms, the open room's log
//! and live status, and what you do in it. The rules live in
//! `backend::groups`; this keeps the UI side of them.

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::{Context, ListAlignment, ListState, px};
use serde_json::{Value, json};

use crate::backend::gateway::RpcError;
use crate::backend::groups::{self, Capabilities, Driver, Entry, Room, Speaker};
use crate::backend::runtime;

use super::{AppEvent, AppState};

/// How long a group request may take; a bot's computer can be slow to answer.
const DEADLINE: Duration = Duration::from_secs(45);

/// A message you sent whose outcome is still open. It keeps its `event_id`
/// so a Retry is recognized as the same message.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub event_id: String,
    pub text: String,
    /// Why it couldn't be confirmed; empty while the request is out.
    pub failed: String,
}

/// The open group.
pub struct GroupChat {
    pub room_id: String,
    pub room: Option<Room>,
    pub entries: Vec<Entry>,
    pub driver: Driver,
    pub list: ListState,
    /// The last log seq read; the next read starts after it.
    cursor: i64,
    epoch: i64,
    /// The first read hasn't come back yet.
    pub loading: bool,
    /// A read is out; polls don't stack.
    reading: bool,
    /// The last read failed, in words; the next poll tries again.
    pub read_error: String,
    /// A Stop that hasn't been acknowledged keeps its cancel id.
    pub stopping: Option<String>,
    /// Bumped whenever the open group changes, so stale replies are dropped.
    token: u64,
}

impl Default for GroupChat {
    fn default() -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(1000.));
        list.set_follow_mode(gpui_kit::FollowMode::Tail);
        GroupChat {
            room_id: String::new(),
            room: None,
            entries: Vec::new(),
            driver: Driver::default(),
            list,
            cursor: 0,
            epoch: 0,
            loading: false,
            reading: false,
            read_error: String::new(),
            stopping: None,
            token: 0,
        }
    }
}

impl GroupChat {
    pub(super) fn reset(&mut self, room_id: &str, room: Option<Room>) {
        self.token += 1;
        self.room_id = room_id.to_string();
        self.room = room;
        self.entries.clear();
        self.driver = Driver::default();
        self.list.reset(0);
        self.list.set_follow_mode(gpui_kit::FollowMode::Tail);
        self.cursor = 0;
        self.epoch = 0;
        self.loading = !room_id.is_empty();
        self.reading = false;
        self.read_error.clear();
        self.stopping = None;
    }

    pub fn member(&self, id: &str) -> Option<&groups::Member> {
        self.room.as_ref().and_then(|r| r.member(id))
    }
}

/// The groups the server has and what it can do with them.
#[derive(Default)]
pub struct Groups {
    /// `None` until `groups.capabilities` answers.
    pub caps: Option<Capabilities>,
    /// Why groups can't be used here; empty when they can.
    pub unavailable: String,
    pub rooms: Vec<Room>,
    /// Each room's last line, as far as this session has seen.
    pub previews: HashMap<String, String>,
    /// Sends whose outcome is open, by room.
    pub sends: HashMap<String, Outgoing>,
    /// A create whose reply was lost keeps its room id, so trying the same
    /// group again opens it instead of making a second one.
    pub(super) create_intent: Option<(String, String)>,
}

impl Groups {
    pub fn offers(&self, method: &str) -> bool {
        self.caps.as_ref().is_some_and(|c| c.usable() && c.offers(method))
    }
}

fn preview_of(entry: &Entry, chat: &GroupChat, names: &dyn Fn(&str) -> String) -> String {
    let who = match &entry.speaker {
        Speaker::You => "You".to_string(),
        Speaker::Scheduled => "Scheduled".to_string(),
        Speaker::Member(id) => names(&chat.member(id).map(|m| m.profile.clone()).unwrap_or_else(|| id.clone())),
        Speaker::Notice => return entry.text.clone(),
    };
    let words: String = entry.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
    format!("{who}: {words}")
}

impl AppState {
    /// Runs one group request and hands the whole outcome to `done`.
    fn group_call(
        &mut self,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<Value, RpcError>, &mut Context<Self>) + Send + 'static,
    ) {
        let Some(gateway) = self.gateway.clone() else {
            done(self, Err(RpcError { message: "Not connected.".into(), ..Default::default() }), cx);
            return;
        };
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = gateway.request_detailed(method, params, DEADLINE).await;
            queue.queue(move |app, cx| {
                if app.generation == generation {
                    done(app, result, cx);
                    cx.notify();
                }
            });
        });
    }

    /// A bot's name as the sidebar shows it.
    pub fn bot_label(&self, profile: &str) -> String {
        self.bots.iter().find(|b| b.name == profile).map(|b| b.display_name.clone()).unwrap_or_else(|| profile.to_string())
    }

    /// A member's name: what the room calls it, else its bot's name.
    pub fn member_label(&self, member: &groups::Member) -> String {
        if !member.display_name.is_empty() {
            return member.display_name.clone();
        }
        let profile = if member.profile.is_empty() { &member.member_id } else { &member.profile };
        self.bot_label(profile)
    }

    /// Reads what the server's group surface offers, then its rooms.
    pub(super) fn load_groups(&mut self, cx: &mut Context<Self>) {
        self.group_call("groups.capabilities", json!({}), cx, |app, result, cx| match result {
            Ok(reply) => {
                let caps = Capabilities::from_reply(&reply);
                app.groups.unavailable = groups::unavailable_reason(Some(&caps), None);
                let usable = caps.usable();
                app.groups.caps = Some(caps);
                if usable {
                    app.refresh_groups(cx);
                }
            }
            Err(e) => {
                app.groups.unavailable = groups::unavailable_reason(None, Some(&e));
                app.groups.caps = Some(Capabilities::default());
            }
        });
    }

    /// Re-reads the room list; a failed read keeps the last good one.
    pub fn refresh_groups(&mut self, cx: &mut Context<Self>) {
        if !self.groups.offers("groups.list") {
            return;
        }
        self.group_call("groups.list", json!({}), cx, |app, result, _| {
            if let Ok(reply) = result {
                app.groups.rooms = groups::rooms_from_list(&reply);
                // The open room's name and members follow the list.
                if let Some(room) = app.groups.rooms.iter().find(|r| r.room_id == app.group.room_id) {
                    app.group.room = Some(room.clone());
                }
            }
        });
    }

    /// A group's name, from the list (or the open group).
    pub fn room_name(&self, room_id: &str) -> String {
        self.groups
            .rooms
            .iter()
            .find(|r| r.room_id == room_id)
            .or(self.group.room.as_ref().filter(|r| r.room_id == room_id))
            .map(|r| r.name.clone())
            .unwrap_or_default()
    }

    pub fn active_room(&self) -> Option<&Room> {
        self.active_group.as_ref().and(self.group.room.as_ref())
    }

    /// Opens a group in place of the bot chat.
    pub fn select_group(&mut self, room_id: &str, cx: &mut Context<Self>) {
        if self.active_group.as_deref() == Some(room_id) {
            return;
        }
        let room = self.groups.rooms.iter().find(|r| r.room_id == room_id).cloned();
        self.active_group = Some(room_id.to_string());
        self.group.reset(room_id, room);
        self.poll_group(cx);
        cx.emit(AppEvent::GroupOpened);
        cx.notify();
    }

    /// Back to the bots: closes the open group.
    pub(super) fn leave_group(&mut self) {
        self.active_group = None;
        self.group.reset("", None);
    }

    /// Reads the open group's state and the log after what's shown. The
    /// group view calls this every two seconds while the group is visible.
    pub fn poll_group(&mut self, cx: &mut Context<Self>) {
        let Some(room_id) = self.active_group.clone() else { return };
        let Some(gateway) = self.gateway.clone() else { return };
        if self.group.reading {
            return;
        }
        self.group.reading = true;
        cx.notify();
        let (token, cursor, generation) = (self.group.token, self.group.cursor, self.generation);
        let limit = self.groups.caps.as_ref().map_or(200, |_| 500);
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let state = gateway.request_detailed("groups.state", json!({ "room_id": room_id }), DEADLINE).await;
            let mut pages = Vec::new();
            let mut since = cursor;
            let log = loop {
                let mut params = json!({ "room_id": room_id, "limit": limit });
                if since > 0 {
                    params["since_seq"] = json!(since);
                }
                match gateway.request_detailed("groups.log", params, DEADLINE).await {
                    Ok(reply) => {
                        let page = groups::log_page(&reply);
                        let more = page.has_more && page.cursor > since;
                        since = page.cursor.max(since);
                        pages.push(page);
                        if !more {
                            break Ok(());
                        }
                    }
                    Err(e) => break Err(e),
                }
            };
            queue.queue(move |app, cx| {
                if app.generation != generation || app.group.token != token {
                    return;
                }
                app.group.reading = false;
                app.apply_group_read(state, pages, log, cx);
                cx.notify();
            });
        });
    }

    fn apply_group_read(
        &mut self,
        state: Result<Value, RpcError>,
        pages: Vec<groups::LogPage>,
        log: Result<(), RpcError>,
        cx: &mut Context<Self>,
    ) {
        let mut errors = Vec::new();
        match state {
            Ok(state) => {
                if let Some(room) = state.get("room").filter(|r| r.is_object()) {
                    self.group.room = Some(Room::from_value(room));
                }
                self.group.driver = Driver::from_state(&state);
            }
            Err(e) => errors.push(e),
        }
        for page in pages {
            // A new authority epoch replaces the log: read it from the start.
            if page.epoch != 0 && self.group.epoch != 0 && page.epoch != self.group.epoch {
                self.group.epoch = page.epoch;
                self.group.cursor = 0;
                self.group.entries.clear();
                self.group.list.reset(0);
                self.poll_group_soon(cx);
                return;
            }
            if page.epoch != 0 {
                self.group.epoch = page.epoch;
            }
            let fresh: Vec<Entry> = groups::entries_from_events(&page.events)
                .into_iter()
                .filter(|e| self.group.entries.last().is_none_or(|last| e.seq > last.seq))
                .collect();
            self.group.cursor = self.group.cursor.max(page.cursor);
            if !fresh.is_empty() {
                let at = self.group.entries.len();
                let count = fresh.len();
                self.group.entries.extend(fresh);
                self.group.list.splice(at..at, count);
            }
        }
        if let Err(e) = log {
            errors.push(e);
        }
        self.group.loading = false;
        self.group.read_error = match errors.first() {
            None => String::new(),
            Some(e) if e.message.contains("in time") => {
                "The server may be waiting for a bot's computer. Still trying…".into()
            }
            Some(_) => "Couldn't read the group. Still trying…".into(),
        };
        if let Some(last) = self.group.entries.last() {
            let names = |p: &str| self.bot_label(p);
            let preview = preview_of(last, &self.group, &names);
            self.groups.previews.insert(self.group.room_id.clone(), preview);
        }
    }

    fn poll_group_soon(&mut self, cx: &mut Context<Self>) {
        self.group.reading = false;
        self.poll_group(cx);
    }

    /// The open group's send whose outcome is open.
    pub fn group_send(&self) -> Option<&Outgoing> {
        self.active_group.as_ref().and_then(|id| self.groups.sends.get(id))
    }

    /// Posts `text` to the open group.
    pub fn send_to_group(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(room_id) = self.active_group.clone() else { return };
        if text.trim().is_empty() || self.groups.sends.contains_key(&room_id) || !self.groups.offers("groups.send") {
            return;
        }
        self.error.clear();
        let out = Outgoing { event_id: groups::new_id(), text: text.to_string(), failed: String::new() };
        self.groups.sends.insert(room_id.clone(), out);
        self.group.list.set_follow_mode(gpui_kit::FollowMode::Tail);
        self.dispatch_group_send(&room_id, cx);
        cx.notify();
    }

    /// Sends the open group's unconfirmed message again, as the same event.
    pub fn resend_to_group(&mut self, cx: &mut Context<Self>) {
        let Some(room_id) = self.active_group.clone() else { return };
        if let Some(out) = self.groups.sends.get_mut(&room_id) {
            out.failed.clear();
            self.dispatch_group_send(&room_id, cx);
        }
        cx.notify();
    }

    fn dispatch_group_send(&mut self, room_id: &str, cx: &mut Context<Self>) {
        let Some(out) = self.groups.sends.get(room_id).cloned() else { return };
        let room = room_id.to_string();
        let params = groups::send_params(room_id, &out.event_id, &out.text);
        self.group_call("groups.send", params, cx, move |app, result, cx| {
            // Only this exact send's outcome settles it.
            if app.groups.sends.get(&room).is_none_or(|o| o.event_id != out.event_id) {
                return;
            }
            match result {
                Ok(reply) if groups::send_delivered(&reply, &out.event_id) => {
                    app.groups.sends.remove(&room);
                    if app.active_group.as_deref() == Some(room.as_str()) {
                        app.poll_group_soon(cx);
                    }
                }
                Ok(_) => {
                    if let Some(o) = app.groups.sends.get_mut(&room) {
                        o.failed = "The server didn't confirm this message.".into();
                    }
                }
                Err(e) if groups::admitted_nothing(&e) => {
                    // Proven unposted: the words go back to the box.
                    app.groups.sends.remove(&room);
                    app.error = format!("The group didn't take the message: {}", e.message);
                    if app.active_group.as_deref() == Some(room.as_str()) {
                        cx.emit(AppEvent::GroupDraftBack(out.text.clone()));
                    }
                }
                Err(e) => {
                    if let Some(o) = app.groups.sends.get_mut(&room) {
                        o.failed = e.message;
                    }
                }
            }
        });
    }

    /// Asks the open group to stop its work.
    pub fn stop_group(&mut self, cx: &mut Context<Self>) {
        let Some(room_id) = self.active_group.clone() else { return };
        if !self.groups.offers("groups.stop") {
            return;
        }
        let cancel_id = self.group.stopping.get_or_insert_with(groups::new_id).clone();
        let token = self.group.token;
        self.group_call("groups.stop", json!({ "room_id": room_id, "cancel_id": cancel_id }), cx, move |app, result, cx| {
            if app.group.token != token {
                return;
            }
            match result {
                Ok(_) => {
                    app.group.stopping = None;
                    app.poll_group_soon(cx);
                }
                Err(e) => app.error = format!("Couldn't stop the group: {}", e.message),
            }
        });
        cx.notify();
    }

    /// Renames a group.
    pub fn rename_group(&mut self, room_id: &str, name: &str, cx: &mut Context<Self>) {
        let room_id = room_id.to_string();
        let name = name.trim().to_string();
        if name.is_empty() || !self.groups.offers("groups.rename") {
            return;
        }
        let params = json!({ "room_id": room_id, "event_id": groups::new_id(), "name": name });
        self.group_call("groups.rename", params, cx, move |app, result, cx| match result {
            Ok(reply) => {
                let renamed = reply.get("room").filter(|r| r.is_object()).map(Room::from_value);
                for room in app.groups.rooms.iter_mut().filter(|r| r.room_id == room_id) {
                    room.name = renamed.as_ref().map_or(name.clone(), |r| r.name.clone());
                }
                if app.group.room_id == room_id {
                    if let Some(room) = app.group.room.as_mut() {
                        room.name = renamed.as_ref().map_or(name.clone(), |r| r.name.clone());
                    }
                }
                cx.emit(AppEvent::GroupActionDone(Ok(())));
            }
            Err(e) => cx.emit(AppEvent::GroupActionDone(Err(format!("Couldn't rename the group: {}", e.message)))),
        });
    }

    /// Ends a group for good. Only a tombstone for this room confirms it.
    pub fn disband_group(&mut self, room_id: &str, cx: &mut Context<Self>) {
        let room_id = room_id.to_string();
        if !self.groups.offers("groups.disband") {
            return;
        }
        let params = json!({ "room_id": room_id, "cancel_id": groups::new_id() });
        self.group_call("groups.disband", params, cx, move |app, result, cx| match result {
            Ok(reply) if groups::disband_confirmed(&reply, &room_id) => {
                app.groups.rooms.retain(|r| r.room_id != room_id);
                app.groups.previews.remove(&room_id);
                app.groups.sends.remove(&room_id);
                if app.active_group.as_deref() == Some(room_id.as_str()) {
                    app.leave_group();
                    app.open_bot_chat(cx);
                }
                cx.emit(AppEvent::GroupActionDone(Ok(())));
            }
            Ok(_) => cx.emit(AppEvent::GroupActionDone(Err(
                "The server couldn't confirm the group ended. A bot's computer may be asleep or offline; try again once it's back.".into(),
            ))),
            Err(e) => cx.emit(AppEvent::GroupActionDone(Err(groups::disband_refusal(&e)))),
        });
    }

    /// Bots that can sit in a new group: on the roster and seatable here.
    pub fn seatable_bots(&self) -> Vec<super::Bot> {
        let seatable = self.groups.caps.as_ref().map(|c| c.seatable.clone()).unwrap_or_default();
        self.bots.iter().filter(|b| seatable.is_empty() || seatable.contains(&b.name)).cloned().collect()
    }

    /// Makes a group of these bots and opens it.
    pub fn create_group(&mut self, name: &str, profiles: Vec<String>, cx: &mut Context<Self>) {
        if let Some(problem) = groups::roster_problem(&profiles) {
            cx.emit(AppEvent::GroupActionDone(Err(problem)));
            return;
        }
        if !self.groups.offers("groups.create") {
            cx.emit(AppEvent::GroupActionDone(Err("This server can't make bot groups.".into())));
            return;
        }
        let labels: Vec<String> = profiles.iter().map(|p| self.bot_label(p)).collect();
        let name = Some(name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| groups::default_name(&labels));
        // The same name and bots after an uncertain reply reuse the room id.
        let key = format!("{name}\u{0}{}", profiles.join("\u{0}"));
        let room_id = match &self.groups.create_intent {
            Some((k, id)) if *k == key => id.clone(),
            _ => groups::new_id(),
        };
        self.groups.create_intent = Some((key, room_id.clone()));
        let params = groups::create_params(&room_id, &name, &profiles);
        self.group_call("groups.create", params, cx, move |app, result, cx| match result {
            Ok(reply) => {
                app.groups.create_intent = None;
                let room = reply.get("room").filter(|r| r.is_object()).map(Room::from_value).filter(|r| !r.room_id.is_empty());
                let room = room.unwrap_or_else(|| Room { room_id: room_id.clone(), name: name.clone(), ..Default::default() });
                if !app.groups.rooms.iter().any(|r| r.room_id == room.room_id) {
                    app.groups.rooms.insert(0, room.clone());
                }
                app.select_group(&room.room_id, cx);
                app.refresh_groups(cx);
                cx.emit(AppEvent::GroupActionDone(Ok(())));
            }
            Err(e) => {
                // A refusal ends the intent; a lost reply keeps it for Retry.
                if e.code != 0 {
                    app.groups.create_intent = None;
                }
                let retry = if e.code == 0 { " Trying again is safe." } else { "" };
                cx.emit(AppEvent::GroupActionDone(Err(format!("Couldn't create the group: {}.{retry}", e.message.trim_end_matches('.')))));
            }
        });
    }
}

#[cfg(test)]
impl AppState {
    /// Takes a `groups.state` and `groups.log` reply as if a poll read them.
    pub fn apply_group_replies(&mut self, state: Value, log: Value, cx: &mut Context<Self>) {
        self.apply_group_read(Ok(state), vec![groups::log_page(&log)], Ok(()), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_name_the_speaker() {
        let mut chat = GroupChat::default();
        chat.room = Some(Room {
            room_id: "r".into(),
            members: vec![groups::Member { member_id: "w".into(), profile: "work".into(), ..Default::default() }],
            ..Default::default()
        });
        let names = |p: &str| if p == "work" { "Work".to_string() } else { p.to_string() };
        let said = Entry { seq: 1, speaker: Speaker::Member("w".into()), member_id: "w".into(), text: "all\n done".into(), at: 0.0 };
        assert_eq!(preview_of(&said, &chat, &names), "Work: all done");
        let you = Entry { speaker: Speaker::You, text: "hi".into(), ..said.clone() };
        assert_eq!(preview_of(&you, &chat, &names), "You: hi");
    }
}
