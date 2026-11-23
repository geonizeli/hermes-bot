//! Bot groups: group chats the Hermes gateway owns and runs (`groups.*`).
//!
//! The gateway keeps each room's log and runs its turns whether or not any
//! app is open. A client lists rooms, reads a room's state and log, and
//! posts to it. The rules follow the official desktop's group contract
//! (hermes-agent `apps/desktop/docs/canonical-groups.md`):
//! - a control is offered only when `groups.capabilities` lists its method;
//! - nothing ever falls back to running a group on the client;
//! - a Send's words come back for editing only on the gateway's
//!   `admitted: false` proof; any other failure keeps the same `event_id`
//!   for Retry, which the gateway recognizes, so nothing posts twice.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::gateway::RpcError;

/// Bots a group may hold.
pub const MIN_MEMBERS: usize = 2;
pub const MAX_MEMBERS: usize = 6;

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// What the gateway's group surface offers (`groups.capabilities`).
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    /// The gateway runs its own group driver, so groups work with no app open.
    pub driver: bool,
    pub methods: HashSet<String>,
    /// Profiles that can sit in a group here.
    pub seatable: Vec<String>,
}

impl Capabilities {
    pub fn from_reply(v: &Value) -> Capabilities {
        let list = |key: &str| -> Vec<String> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default()
        };
        Capabilities {
            driver: v.get("driver").and_then(Value::as_bool).unwrap_or(false),
            methods: list("methods").into_iter().collect(),
            seatable: list("seatable_profiles"),
        }
    }

    /// Gateway group chats work here: the gateway runs them itself.
    pub fn usable(&self) -> bool {
        self.driver
    }

    pub fn offers(&self, method: &str) -> bool {
        self.methods.contains(method)
    }
}

/// Why the gateway can't run groups, in words.
pub fn unavailable_reason(caps: Option<&Capabilities>, error: Option<&RpcError>) -> String {
    match (caps, error) {
        (_, Some(e)) if e.code == -32601 => "This server's Hermes is too old for bot groups. Update Hermes there and reconnect.".into(),
        (_, Some(_)) => "Couldn't reach the server's group chats.".into(),
        (Some(c), None) if !c.driver => "The server's group driver isn't running. Restart or update Hermes there.".into(),
        _ => String::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub member_id: String,
    pub profile: String,
    pub handle: String,
    pub display_name: String,
}

impl Member {
    fn from_value(v: &Value) -> Member {
        Member {
            member_id: text(v, "member_id"),
            profile: Some(text(v, "profile"))
                .filter(|p| !p.is_empty())
                .or_else(|| v.pointer("/target/profile").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default(),
            handle: text(v, "handle"),
            display_name: text(v, "display_name"),
        }
    }
}

/// A group as the sidebar lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Room {
    pub room_id: String,
    pub name: String,
    pub members: Vec<Member>,
    pub authority_epoch: i64,
    pub latest_seq: i64,
    pub updated_at: f64,
}

impl Room {
    pub fn from_value(v: &Value) -> Room {
        Room {
            room_id: text(v, "room_id"),
            name: text(v, "name"),
            members: v.get("members").and_then(Value::as_array).map(|m| m.iter().map(Member::from_value).collect()).unwrap_or_default(),
            authority_epoch: v.get("authority_epoch").and_then(Value::as_i64).unwrap_or(0),
            latest_seq: v.get("latest_seq").and_then(Value::as_i64).unwrap_or(0),
            updated_at: v.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0),
        }
    }

    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.member_id == id)
    }
}

/// `groups.list`: the rooms, most recently active first.
pub fn rooms_from_list(v: &Value) -> Vec<Room> {
    let mut rooms: Vec<Room> = v
        .get("rooms")
        .and_then(Value::as_array)
        .map(|r| r.iter().map(Room::from_value).filter(|r| !r.room_id.is_empty()).collect())
        .unwrap_or_default();
    rooms.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    rooms
}

/// One member's live work (`driver_status.member_activity`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    pub member_id: String,
    /// Waiting for its turn rather than answering.
    pub queued: bool,
    /// The tool it runs, when it reports one.
    pub tool: String,
    /// Seconds since the epoch; 0 when unknown.
    pub started_at: f64,
}

/// What the room's driver reports (`groups.state` → `driver_status`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Driver {
    /// No driver status at all: the gateway's group driver is down.
    pub missing: bool,
    pub working: bool,
    pub blocked: bool,
    pub activity: Vec<Activity>,
    pub pending_actions: usize,
    pub last_error: String,
}

impl Driver {
    pub fn from_state(state: &Value) -> Driver {
        let Some(d) = state.get("driver_status").filter(|d| d.is_object()) else {
            return Driver { missing: true, ..Driver::default() };
        };
        let activity = d
            .get("member_activity")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|r| Activity {
                        member_id: text(r, "member_id"),
                        queued: text(r, "status") == "queued",
                        tool: text(r, "tool"),
                        started_at: r.get("started_at").and_then(Value::as_f64).unwrap_or(0.0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let last_error = match d.get("last_error") {
            Some(Value::String(s)) => s.clone(),
            Some(e @ Value::Object(_)) => Some(text(e, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| text(e, "code")),
            _ => String::new(),
        };
        Driver {
            missing: false,
            working: d.get("working").and_then(Value::as_bool).unwrap_or(false),
            blocked: d.get("blocked").and_then(Value::as_bool).unwrap_or(false),
            activity,
            pending_actions: d.get("pending_actions").and_then(Value::as_array).map_or(0, Vec::len),
            last_error,
        }
    }

    /// The member's live work, answering (not queued) first.
    pub fn activity_of(&self, member_id: &str) -> Option<&Activity> {
        let mut rows = self.activity.iter().filter(|a| a.member_id == member_id);
        let first = rows.clone().find(|a| !a.queued);
        first.or_else(|| rows.next())
    }

    /// Members answering right now.
    pub fn answering(&self) -> Vec<&Activity> {
        let mut seen = HashSet::new();
        self.activity.iter().filter(|a| !a.queued && seen.insert(a.member_id.clone())).collect()
    }

    pub fn busy(&self) -> bool {
        self.working || !self.activity.is_empty()
    }
}

/// Who said a log entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Speaker {
    You,
    /// A post the gateway's scheduler made.
    Scheduled,
    Member(String),
    /// Something the room reports (a failed turn, a stop…).
    Notice,
}

/// One transcript line from the room's log.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub seq: i64,
    pub speaker: Speaker,
    /// The member a notice is about.
    pub member_id: String,
    pub text: String,
    /// Seconds since the epoch.
    pub at: f64,
}

/// Words for a reason code the gateway gives.
fn reason_text(code: &str) -> String {
    match code {
        "" => String::new(),
        "member_unavailable" => "the bot didn't answer in time".into(),
        "cancelled" | "stopped" => "stopped".into(),
        other => other.replace('_', " "),
    }
}

/// The notice an event shows, when it shows one.
fn notice_for(kind: &str, payload: &Value) -> Option<&'static str> {
    Some(match kind {
        "turn.failed" => "Turn failed",
        "turn.cancelled" => "Turn cancelled",
        "turn.deferred" => "Turn deferred",
        "member.unavailable" => "Member unavailable",
        "room.stop_requested" => "Stop requested",
        "room.disbanded" => "Group disbanded",
        "authority.lost" => "This server no longer runs the group",
        "room.activity" if text(payload, "status") == "bounded" => "Group work limit reached",
        _ => return None,
    })
}

/// The log's events as transcript lines. Events that only feed live status
/// (settled turns, routing) add no line.
pub fn entries_from_events(events: &[Value]) -> Vec<Entry> {
    events
        .iter()
        .filter_map(|e| {
            let kind = text(e, "kind");
            let payload = e.get("payload").cloned().unwrap_or(Value::Null);
            let actor = e.get("actor").cloned().unwrap_or(Value::Null);
            let seq = e.get("seq").and_then(Value::as_i64).unwrap_or(0);
            let at = e.get("created_at").and_then(Value::as_f64).unwrap_or(0.0);
            let words = Some(text(&payload, "text")).filter(|t| !t.is_empty()).unwrap_or_else(|| text(&payload, "content"));
            match kind.as_str() {
                "message.user" => {
                    let id = text(&actor, "id");
                    let speaker = if id.starts_with("cron:") || id.starts_with("cron-notice:") { Speaker::Scheduled } else { Speaker::You };
                    Some(Entry { seq, speaker, member_id: String::new(), text: words, at })
                }
                "message.member" => {
                    let id = Some(text(&actor, "id")).filter(|i| !i.is_empty()).unwrap_or_else(|| text(&payload, "member_id"));
                    Some(Entry { seq, speaker: Speaker::Member(id.clone()), member_id: id, text: words, at })
                }
                _ => {
                    let notice = notice_for(&kind, &payload)?;
                    let reason = ["error", "reason", "reason_code"]
                        .iter()
                        .map(|k| match payload.get(*k) {
                            Some(Value::String(s)) => s.clone(),
                            Some(o @ Value::Object(_)) => Some(text(o, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| text(o, "code")),
                            _ => String::new(),
                        })
                        .find(|r| !r.is_empty())
                        .unwrap_or_default();
                    let detail = reason_text(&reason);
                    let line = if detail.is_empty() || kind == "room.activity" { notice.to_string() } else { format!("{notice}: {detail}") };
                    Some(Entry { seq, speaker: Speaker::Notice, member_id: text(&payload, "member_id"), text: line, at })
                }
            }
        })
        .collect()
}

/// One page of `groups.log`.
#[derive(Clone, Debug, Default)]
pub struct LogPage {
    pub events: Vec<Value>,
    /// The last seq this page covers; the next read starts after it.
    pub cursor: i64,
    pub has_more: bool,
    pub epoch: i64,
}

pub fn log_page(v: &Value) -> LogPage {
    let events = v.get("events").and_then(Value::as_array).cloned().unwrap_or_default();
    let last = events.iter().filter_map(|e| e.get("seq").and_then(Value::as_i64)).max().unwrap_or(0);
    LogPage {
        cursor: v.get("cursor").and_then(Value::as_i64).unwrap_or(last).max(last),
        has_more: v.get("has_more").and_then(Value::as_bool).unwrap_or(false),
        epoch: v.pointer("/authority/epoch").and_then(Value::as_i64).unwrap_or(0),
        events,
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// `groups.create` for these profiles: each bot sits as its own profile,
/// under its own name as the handle.
pub fn create_params(room_id: &str, name: &str, profiles: &[String]) -> Value {
    let members: Vec<Value> = profiles
        .iter()
        .map(|p| json!({ "member_id": p, "profile": p, "handle": p, "target": { "kind": "local", "profile": p } }))
        .collect();
    json!({ "room_id": room_id, "name": name, "members": members })
}

/// Why these profiles can't form a group, or `None` when they can.
pub fn roster_problem(profiles: &[String]) -> Option<String> {
    if profiles.len() < MIN_MEMBERS || profiles.len() > MAX_MEMBERS {
        return Some(format!("A group holds {MIN_MEMBERS} to {MAX_MEMBERS} bots."));
    }
    let mut seen = HashSet::new();
    for p in profiles {
        let key = p.to_ascii_lowercase();
        if key == "all" || key == "everyone" {
            return Some(format!("@{p} is reserved in groups; rename that bot to seat it."));
        }
        if !seen.insert(key) {
            return Some("Each bot can sit in a group only once.".into());
        }
    }
    None
}

/// The name a new group gets when you give none: its bots' names.
pub fn default_name(names: &[String]) -> String {
    names.join(", ")
}

/// A message you post. Each starts its own thread, named after the event.
pub fn send_params(room_id: &str, event_id: &str, words: &str) -> Value {
    json!({ "room_id": room_id, "event_id": event_id, "payload": { "text": words, "thread_id": event_id } })
}

/// The gateway took the message as this send's (not someone else's).
pub fn send_delivered(reply: &Value, event_id: &str) -> bool {
    let ours = match reply.get("client_event_id").and_then(Value::as_str) {
        Some(id) => id == event_id,
        None => reply.get("event").is_some_and(Value::is_object),
    };
    ours && reply.get("accepted").and_then(Value::as_bool) != Some(false)
}

/// The gateway proved this request posted nothing, so its words can come back.
pub fn admitted_nothing(error: &RpcError) -> bool {
    error.data.get("admitted").and_then(Value::as_bool) == Some(false)
}

/// The disband reply confirms this room ended.
pub fn disband_confirmed(reply: &Value, room_id: &str) -> bool {
    reply.pointer("/tombstone/room_id").and_then(Value::as_str) == Some(room_id)
}

/// A disband refusal, in words.
pub fn disband_refusal(error: &RpcError) -> String {
    let reason = error.data.get("reason").and_then(Value::as_str).unwrap_or_default();
    match reason {
        "member_home_unknown" => "A bot's profile is missing on the server, so its stop can't be confirmed. Restore it and try again.".into(),
        "peer_unreachable" => "Work was stopped, but a bot's computer couldn't be reached. Try again once it's back.".into(),
        r if r.contains("stop") => "The bots haven't all confirmed they stopped. Try again once they have.".into(),
        _ => format!("Couldn't disband the group: {}", error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/groups_log.json")).unwrap()
    }

    #[test]
    fn reads_capabilities() {
        let caps = Capabilities::from_reply(&json!({
            "driver": true, "persistent_process": true,
            "methods": ["groups.list", "groups.send", "groups.stop"],
            "seatable_profiles": ["default", "work"]
        }));
        assert!(caps.usable() && caps.offers("groups.stop") && !caps.offers("groups.disband"));
        assert_eq!(caps.seatable, ["default", "work"]);
        assert!(!Capabilities::from_reply(&json!({"driver": false})).usable());
        let old = RpcError { code: -32601, message: "unknown method".into(), data: Value::Null };
        assert!(unavailable_reason(None, Some(&old)).contains("too old"));
    }

    #[test]
    fn lists_rooms_newest_first() {
        let rooms = rooms_from_list(&json!({"rooms": [
            {"room_id": "a", "name": "A", "updated_at": 1.0, "members": [{"member_id": "x", "handle": "x", "target": {"kind": "local", "profile": "x"}}]},
            {"room_id": "b", "name": "B", "updated_at": 2.0, "members": []}
        ]}));
        assert_eq!(rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["B", "A"]);
        assert_eq!(rooms[1].members[0].profile, "x");
    }

    #[test]
    fn turns_the_log_into_lines() {
        let page = log_page(&log());
        assert_eq!((page.cursor, page.has_more, page.epoch), (7, false, 1));
        let entries = entries_from_events(&page.events);
        let speakers: Vec<_> = entries.iter().map(|e| e.speaker.clone()).collect();
        assert_eq!(speakers, [Speaker::You, Speaker::Member("communication".into()), Speaker::Member("finance".into())]);
        assert_eq!(entries[0].text, "Hi!");
        assert!(entries[2].text.starts_with("Hello from @finance"));
    }

    #[test]
    fn notices_say_what_happened() {
        let events = vec![
            json!({"seq": 1, "kind": "turn.deferred", "payload": {"member_id": "work", "reason": "member_unavailable"}}),
            json!({"seq": 2, "kind": "room.activity", "payload": {"status": "bounded"}}),
            json!({"seq": 3, "kind": "room.activity", "payload": {"status": "settled"}}),
            json!({"seq": 4, "kind": "message.user", "actor": {"id": "cron:daily", "kind": "user"}, "payload": {"text": "standup"}}),
        ];
        let entries = entries_from_events(&events);
        assert_eq!(entries[0].text, "Turn deferred: the bot didn't answer in time");
        assert_eq!(entries[0].member_id, "work");
        assert_eq!(entries[1].text, "Group work limit reached");
        assert_eq!(entries[2].speaker, Speaker::Scheduled);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn reads_live_work() {
        let state = json!({"driver_status": {"working": true, "pending_actions": [{}], "member_activity": [
            {"member_id": "a", "status": "queued"},
            {"member_id": "b", "status": "running", "tool": "terminal", "started_at": 10.0}
        ]}});
        let d = Driver::from_state(&state);
        assert!(d.busy() && !d.missing);
        assert_eq!(d.pending_actions, 1);
        assert_eq!(d.answering().len(), 1);
        assert_eq!(d.activity_of("b").unwrap().tool, "terminal");
        assert!(d.activity_of("a").unwrap().queued);
        assert!(Driver::from_state(&json!({})).missing);
    }

    #[test]
    fn builds_requests_by_the_contract() {
        let p = create_params("r1", "Pair", &["default".into(), "work".into()]);
        assert_eq!(p["members"][1], json!({"member_id": "work", "profile": "work", "handle": "work", "target": {"kind": "local", "profile": "work"}}));
        let s = send_params("r1", "e1", "hello");
        assert_eq!(s["payload"], json!({"text": "hello", "thread_id": "e1"}));
        assert!(roster_problem(&["a".into()]).is_some());
        assert!(roster_problem(&["a".into(), "A".into()]).is_some());
        assert!(roster_problem(&["a".into(), "everyone".into()]).is_some());
        assert!(roster_problem(&["a".into(), "b".into()]).is_none());
    }

    #[test]
    fn judges_send_outcomes() {
        assert!(send_delivered(&json!({"event": {}, "client_event_id": "e1"}), "e1"));
        assert!(!send_delivered(&json!({"event": {}, "client_event_id": "e2"}), "e1"));
        assert!(!send_delivered(&json!({"event": {}, "client_event_id": "e1", "accepted": false}), "e1"));
        let refused = RpcError { code: 4001, message: "busy".into(), data: json!({"admitted": false}) };
        assert!(admitted_nothing(&refused));
        assert!(!admitted_nothing(&RpcError { code: 4001, message: "busy".into(), data: Value::Null }));
        assert!(disband_confirmed(&json!({"tombstone": {"room_id": "r1"}}), "r1"));
        assert!(!disband_confirmed(&json!({"tombstone": {"room_id": "r2"}}), "r1"));
    }
}
