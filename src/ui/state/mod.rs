//! `AppState`: the app's state and actions, shared by every view.
//!
//! All network work runs on the tokio runtime; results are queued back onto
//! the UI thread through `Queue`. The views read the state and call its
//! methods; one-off happenings (a probed server, a question from the bot)
//! arrive as `AppEvent`s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::{Context, EventEmitter, ListAlignment, ListState, px};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::backend::auth::{self, RefreshError, ServerInfo};
use crate::backend::botchat::{self, Opened};
use crate::backend::botedit;
use crate::backend::cron;
use crate::backend::gateway::{Gateway, Incoming, UNAUTHORIZED};
use crate::backend::slash;
use crate::backend::store::{self, Config, Server, Tokens};
use crate::backend::theme::ThemeChoice;
use crate::backend::{attachments, history, media};
use crate::backend::{roster, runtime, session};

use super::queue::Queue;

mod chat;
pub mod queued;
pub mod commands;
pub mod editor;
mod edit;
pub mod groups;
mod slots;

pub use chat::{FileRef, Role, Row, Todo};
pub use queued::{MessageQueue, QueuedMessage};
pub use groups::{GroupChat, Groups};

/// A bot (Hermes profile) as the sidebar shows it.
#[derive(Clone, Debug, Default)]
pub struct Bot {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub model: String,
    pub is_default: bool,
    pub preview: String,
    /// The cached avatar file, when the bot has one.
    pub avatar: Option<PathBuf>,
    /// The avatar is a drawn face on a transparent background: no crop.
    pub avatar_face: bool,
}

/// How full the chat's context is.
#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub context_used: f64,
    pub context_max: f64,
    pub percent: f64,
    /// Negative when unknown.
    pub cost: f64,
}

/// A provider in the model picker.
#[derive(Clone, Debug, Default)]
pub struct Provider {
    pub slug: String,
    pub name: String,
    pub models: Vec<String>,
    pub authenticated: bool,
    pub capabilities: Value,
}

#[derive(Clone, Debug)]
pub enum MediaState {
    Loading,
    Ready(PathBuf),
    Failed,
}

pub enum AppEvent {
    /// The server answered (or `None` with `error` set).
    ServerProbed(Option<ServerInfo>),
    SignInFinished(bool, String),
    /// The server wants confirmation before switching to this model.
    ModelConfirm { message: String, model: String, provider: String },
    /// The agent asks something (approval, clarification).
    ServerRequest { id: Value, method: String, params: Value },
    /// A command (like /undo) handed back a draft for the message box.
    Prefill(String),
    /// `/model`: open the model picker.
    OpenModelPicker,
    /// A chat was (re)loaded; the message box takes focus.
    TranscriptLoaded,
    /// A picture for the editor (upload or generation), or an error.
    EditImageReady(Result<PathBuf, String>),
    EditGenerateAvailable(bool),
    EditPetsLoaded(Result<Vec<Value>, String>),
    /// A pet's thumbnail; `None` when it failed.
    EditPetReady(String, Option<PathBuf>),
    EditSaved(Result<(), String>),
    /// A group opened in place of the bot chat.
    GroupOpened,
    /// The group proved it didn't take a message: its words go back to the box.
    GroupDraftBack(String),
    /// Creating, renaming or disbanding a group finished.
    GroupActionDone(Result<(), String>),
}

pub struct AppState {
    pub status: String,
    pub error: String,
    pub busy: bool,
    pub connected: bool,
    pub bots: Vec<Bot>,
    pub active_bot: String,
    pub active_session: String,
    /// The open chat has a turn running.
    pub streaming: bool,
    /// True while connecting or opening a bot's chat.
    pub chat_loading: bool,
    pub jobs: Vec<Value>,
    pub jobs_loading: bool,
    pub jobs_error: String,
    /// `SessionLiveInfo` of the open Bot Chat (model, provider,
    /// reasoning_effort, fast…), Null before it exists.
    pub session_info: Value,
    pub models: Vec<Provider>,
    pub models_loading: bool,
    pub usage: Option<Usage>,
    /// Files waiting to go with the next message.
    pub pending: Vec<attachments::Attachment>,
    /// `DisplayStatus` of the active bot's screen (Null while unknown;
    /// `{"unavailable": true}` when the server has no screens).
    pub screen: Value,
    pub screen_busy: bool,
    /// Output of a running screen install.
    pub screen_log: String,
    pub config: Config,
    /// Where `config` is saved.
    config_path: PathBuf,

    // ── the open conversation (see chat.rs) ──
    pub rows: Vec<Row>,
    pub list: ListState,
    assistant_row: Option<usize>,
    todo_row: Option<usize>,
    /// The latest message you sent: it can be edited, and the answers after
    /// it regenerated.
    pub last_user_index: Option<usize>,
    /// The message being edited in place (a row id).
    pub editing: Option<u64>,
    thinking_since: Option<Instant>,
    next_row: u64,

    gateway: Option<Arc<Gateway>>,
    /// The live (gateway) id of the open conversation; events carry it.
    live_session: String,
    /// Bumped on every (re)connect so stale callbacks are ignored.
    generation: u64,
    /// Each bot's Bot Chat as the roster reports it (`canonical_session`),
    /// the fallback when a lookup answers empty.
    roster_chats: HashMap<String, String>,
    /// Each bot's `profiles.list` row, for the editor.
    roster_rows: HashMap<String, Value>,
    /// Turns in flight, live session id → bot, including ones whose chat
    /// isn't open; switching bots leaves them running.
    running: HashMap<String, String>,
    next_attachment: u64,
    /// Messages waiting for each bot's answer to end, by `queue_key`.
    queues: HashMap<String, MessageQueue>,
    /// Server path → fetched file.
    pub media: HashMap<String, MediaState>,
    /// Each bot's slash commands, from `commands.catalog`.
    pub(crate) catalogs: HashMap<String, slash::Catalog>,
    /// The server's bot groups.
    pub groups: Groups,
    /// The open group (shown instead of the bot chat), by room id.
    pub active_group: Option<String>,
    pub group: GroupChat,
    /// Every other bot's chat, kept loaded (see slots.rs).
    parked: HashMap<String, slots::ChatSlot>,
    /// The first roster read is out: polls wait for it.
    bots_loading: bool,
    /// A parked chat is swapped in for a moment: nothing should reach the
    /// views as if it were the open one.
    backstage: bool,
    queue: Queue<AppState>,
}

impl EventEmitter<AppEvent> for AppState {}

/// Shown while a sign-in is held in memory for want of a keyring.
/// How often the bots and groups are read again.
const ROSTER_POLL: std::time::Duration = std::time::Duration::from_secs(5);

const NO_KEYRING: &str = "There's no keyring to save your sign-in, so you'll need to sign in again after quitting. \
     Install and unlock a Secret Service keyring (GNOME Keyring, KWallet or KeePassXC) to stay signed in.";

/// Text of a transcript message, whichever shape the server used.
fn message_text(m: &Value) -> String {
    if let Some(text) = m.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    match m.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The chat socket's transcript (no tool results), as conversation rows;
/// used when the REST history isn't available.
pub fn transcript_for_view(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
            let mut r = history::row(role);
            r["timestamp"] = m.get("timestamp").cloned().unwrap_or(json!(0.0));
            r["rowId"] = m.get("row_id").cloned().filter(|v| v.is_i64()).unwrap_or(json!(-1));
            match role {
                "user" => {
                    let raw = message_text(m);
                    let (text, refs) = attachments::split_refs(&slash::skill_invocation_text(&raw).unwrap_or(raw));
                    if text.trim().is_empty() && refs.is_empty() {
                        return None;
                    }
                    r["text"] = json!(text);
                    r["attachments"] = json!(Value::Array(refs).to_string());
                }
                "assistant" => {
                    let text = message_text(m);
                    let reasoning = m.get("reasoning").and_then(Value::as_str).unwrap_or_default();
                    if text.trim().is_empty() && reasoning.trim().is_empty() {
                        return None;
                    }
                    r["text"] = json!(text);
                    r["reasoning"] = json!(reasoning);
                }
                "tool" => {
                    r["name"] = json!(m.get("name").and_then(Value::as_str).unwrap_or("tool"));
                    r["detail"] = json!(m.get("context").and_then(Value::as_str).unwrap_or_default());
                    r["args"] = json!(history::pretty_args(m.get("args").unwrap_or(&Value::Null)));
                }
                _ => return None,
            }
            Some(r)
        })
        .collect()
}

/// Reduces `model.options` providers to what the picker needs.
fn models_for_view(result: &Value) -> Vec<Provider> {
    let providers = result.get("providers").and_then(Value::as_array).cloned().unwrap_or_default();
    providers
        .iter()
        .filter(|p| p.get("models").and_then(Value::as_array).is_some_and(|m| !m.is_empty()))
        .map(|p| {
            let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let models = p
                .get("models")
                .and_then(Value::as_array)
                .map(|ms| {
                    ms.iter()
                        .filter_map(|m| match m {
                            Value::String(s) => Some(s.clone()),
                            other => ["id", "name"]
                                .iter()
                                .find_map(|k| other.get(*k).and_then(Value::as_str).map(str::to_string)),
                        })
                        .filter(|id| !id.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            Provider {
                slug: text("slug"),
                name: Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text("slug")),
                models,
                authenticated: p.get("authenticated").and_then(Value::as_bool).unwrap_or(true),
                capabilities: p.get("capabilities").cloned().unwrap_or(json!({})),
            }
        })
        .collect()
}

impl AppState {
    /// The app's state from the saved config, connecting to the active server.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let path = store::config_path();
        let (config, error) = match store::load(&path) {
            Ok(config) => (config, String::new()),
            Err(e) => (Config::default(), e),
        };
        let mut state = Self::with_config(config, path, cx);
        state.error = error;
        state.connect_active(cx);
        state.watch_server(cx);
        state
    }

    /// The server pushes no word when a bot is added or deleted (the official
    /// desktop polls too), and `groups.changed` only covers what its watcher
    /// sees: re-read the bots and groups every few seconds.
    fn watch_server(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(ROSTER_POLL).await;
                let Ok(()) = this.update(cx, |app, cx| {
                    if app.connected && !app.bots_loading {
                        app.refresh_bots(cx);
                        app.refresh_groups(cx);
                    }
                }) else {
                    break;
                };
            }
        })
        .detach();
    }

    /// State for `config`, saved to `config_path`, not yet connected.
    pub fn with_config(config: Config, config_path: PathBuf, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(1000.));
        list.set_follow_mode(gpui_kit::FollowMode::Tail);
        let saved_theme = config.ui.theme.as_deref().and_then(ThemeChoice::parse);
        super::theme::set_choice(saved_theme, cx);
        AppState {
            status: String::new(),
            error: String::new(),
            busy: false,
            connected: false,
            bots: Vec::new(),
            active_bot: String::new(),
            active_session: String::new(),
            streaming: false,
            chat_loading: false,
            jobs: Vec::new(),
            jobs_loading: false,
            jobs_error: String::new(),
            session_info: Value::Null,
            models: Vec::new(),
            models_loading: false,
            usage: None,
            pending: Vec::new(),
            screen: Value::Null,
            screen_busy: false,
            screen_log: String::new(),
            config,
            config_path,
            rows: Vec::new(),
            list,
            assistant_row: None,
            todo_row: None,
            last_user_index: None,
            editing: None,
            thinking_since: None,
            next_row: 0,
            gateway: None,
            live_session: String::new(),
            generation: 0,
            roster_chats: HashMap::new(),
            roster_rows: HashMap::new(),
            running: HashMap::new(),
            next_attachment: 0,
            queues: HashMap::new(),
            media: HashMap::new(),
            catalogs: HashMap::new(),
            groups: Groups::default(),
            active_group: None,
            group: GroupChat::default(),
            parked: HashMap::new(),
            bots_loading: false,
            backstage: false,
            queue: Queue::new(cx),
        }
    }

    // ── reading ──

    pub fn servers(&self) -> &[Server] {
        &self.config.servers
    }

    pub fn active_server(&self) -> Option<&Server> {
        self.config.active_server.as_ref().and_then(|id| self.config.server(id))
    }

    pub fn active_bot(&self) -> Option<&Bot> {
        self.bots.iter().find(|b| b.name == self.active_bot)
    }

    /// Bots with a turn running, open or not.
    pub fn is_busy(&self, bot: &str) -> bool {
        self.running.values().any(|b| b == bot)
    }

    pub fn side_panel(&self) -> (bool, &str) {
        (self.config.ui.side_panel_open, &self.config.ui.side_panel_tab)
    }

    /// Opens the side panel on `tab`, or closes it when it's already showing.
    pub fn toggle_side_panel(&mut self, tab: &str, cx: &mut Context<Self>) {
        let ui = &mut self.config.ui;
        if ui.side_panel_open && ui.side_panel_tab == tab {
            ui.side_panel_open = false;
        } else {
            ui.side_panel_tab = tab.to_string();
            ui.side_panel_open = true;
        }
        self.save_config();
        cx.notify();
    }

    pub fn close_side_panel(&mut self, cx: &mut Context<Self>) {
        self.config.ui.side_panel_open = false;
        self.save_config();
        cx.notify();
    }

    pub fn set_error(&mut self, error: impl Into<String>, cx: &mut Context<Self>) {
        self.error = error.into();
        cx.notify();
    }

    // ── connection ──

    /// Switches the theme and remembers it.
    pub fn set_theme(&mut self, choice: ThemeChoice, cx: &mut Context<Self>) {
        self.config.ui.theme = Some(choice.key().to_string());
        self.save_config();
        super::theme::set_choice(Some(choice), cx);
        cx.notify();
    }

    fn save_config(&mut self) {
        if let Err(e) = store::save(&self.config_path, &self.config) {
            self.error = e;
        }
    }

    fn mark_running(&mut self, live: &str, profile: &str) {
        if !live.is_empty() {
            self.running.insert(live.to_string(), profile.to_string());
        }
    }

    /// Ends the turn on `live`, open or in the background.
    fn mark_finished(&mut self, live: &str) -> Option<String> {
        self.running.remove(live)
    }

    pub(crate) fn disconnect(&mut self) {
        self.generation += 1;
        if let Some(gateway) = self.gateway.take() {
            gateway.close();
        }
        session::set(None);
        self.live_session.clear();
        self.roster_chats.clear();
        self.catalogs.clear();
        self.running.clear();
        // The live sessions die with the socket: every chat loads again.
        self.parked.clear();
        self.bots_loading = false;
        self.connected = false;
        self.streaming = false;
        self.chat_loading = false;
        self.bots.clear();
        self.jobs.clear();
        self.jobs_error.clear();
        self.screen = Value::Null;
        self.screen_log.clear();
        self.active_session.clear();
        self.load_transcript(Vec::new());
        // Unconfirmed sends outlive a reconnect; the open group reads again.
        let sends = std::mem::take(&mut self.groups.sends);
        self.groups = Groups { sends, ..Groups::default() };
        if let Some(room_id) = self.active_group.clone() {
            let room = self.group.room.clone();
            self.group.reset(&room_id, room);
        }
    }

    /// Connects to the active server with its stored tokens, refreshing them
    /// first when they're about to expire.
    fn connect_active(&mut self, cx: &mut Context<Self>) {
        self.disconnect();
        let Some(server) = self.active_server().cloned() else {
            self.status = "Add a server to start.".into();
            cx.notify();
            return;
        };
        let generation = self.generation;
        self.status = format!("Connecting to {}…", server.name);
        self.chat_loading = true;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = open_gateway(&server, queue.clone(), generation).await;
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                match result {
                    Ok((gateway, access_token)) => {
                        session::set(Some(session::Session {
                            gateway: gateway.clone(),
                            base_url: server.url.clone(),
                            access_token,
                        }));
                        app.gateway = Some(gateway);
                        app.connected = true;
                        app.error = if store::is_in_memory(&server.id) {
                            NO_KEYRING.to_string()
                        } else {
                            String::new()
                        };
                        app.status = format!("Connected to {}", server.name);
                        app.load_bots(cx);
                        app.load_groups(cx);
                    }
                    Err(e) => {
                        app.chat_loading = false;
                        app.status = format!("Not connected to {}", server.name);
                        app.error = e;
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Runs one gateway request and hands the result to `done` on the UI thread.
    fn call(
        &mut self,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Value, &mut Context<Self>) + Send + 'static,
    ) {
        let Some(gateway) = self.gateway.clone() else {
            self.set_error("Not connected.", cx);
            return;
        };
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = gateway.request(method, params).await;
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                match result {
                    Ok(value) => done(app, value, cx),
                    Err(e) => {
                        app.streaming = false;
                        app.chat_loading = false;
                        app.error = e;
                    }
                }
                cx.notify();
            });
        });
    }

    fn load_bots(&mut self, cx: &mut Context<Self>) {
        self.bots_loading = true;
        self.call("profiles.list", json!({ "include_sessions": true }), cx, |app, result, cx| {
            app.bots_loading = false;
            app.apply_roster(&result, true, cx);
            let server_id = app.config.active_server.clone().unwrap_or_default();
            let remembered = app.config.last_bot.get(&server_id).cloned();
            let pick = remembered
                .filter(|name| app.bots.iter().any(|b| b.name == *name))
                .or_else(|| app.bots.iter().find(|b| b.is_default).map(|b| b.name.clone()))
                .or_else(|| app.bots.first().map(|b| b.name.clone()))
                .unwrap_or_else(|| "default".into());
            if app.bots.is_empty() {
                app.chat_loading = false;
            }
            app.active_bot = pick;
            app.open_bot_chat(cx);
            app.preload_chats(cx);
        });
    }

    /// Re-reads the roster (names, looks) without touching the open chat;
    /// bots deleted on the server leave, new ones load.
    fn refresh_bots(&mut self, cx: &mut Context<Self>) {
        self.refresh_bots_with(false, cx);
    }

    /// `refresh_bots`, fetching every avatar again (after an edit).
    pub(super) fn refresh_bots_with(&mut self, all_avatars: bool, _cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else { return };
        let generation = self.generation;
        let queue = self.queue.clone();
        // A failed poll isn't worth an error: the next one tries again.
        runtime().spawn(async move {
            let Ok(result) = gateway.request("profiles.list", json!({ "include_sessions": true })).await else { return };
            queue.queue(move |app, cx| {
                if app.generation != generation || app.bots_loading {
                    return;
                }
                let before: Vec<String> = app.bots.iter().map(|b| b.name.clone()).collect();
                app.apply_roster(&result, all_avatars, cx);
                app.sync_bots(&before, cx);
            });
        });
    }

    /// Takes a `profiles.list` reply as the sidebar's bots, then fetches
    /// their avatars.
    /// `all_avatars` false fetches only the avatars of bots that are new or
    /// have none cached.
    fn apply_roster(&mut self, result: &Value, all_avatars: bool, cx: &mut Context<Self>) {
        let Some(profiles) = result.get("profiles").and_then(Value::as_array).cloned() else { return };
        let server_id = self.config.active_server.clone().unwrap_or_default();
        let known: Vec<String> = self.bots.iter().map(|b| b.name.clone()).collect();
        let previews: HashMap<String, String> = self.bots.iter().map(|b| (b.name.clone(), b.preview.clone())).collect();
        let mut with_avatar = Vec::new();
        self.bots = profiles
            .iter()
            .map(|p| {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("default").to_string();
                let mut avatar = None;
                let mut avatar_face = false;
                if p.get("has_avatar").and_then(Value::as_bool).unwrap_or(false) {
                    if let Some(file) = media::cached_avatar(&server_id, &name) {
                        avatar_face = botedit::is_face_copy(&file);
                        avatar = Some(file);
                    }
                    if all_avatars || avatar.is_none() || !known.contains(&name) {
                        with_avatar.push(name.clone());
                    }
                } else {
                    media::clear_avatar(&server_id, &name);
                }
                let preview = p
                    .pointer("/canonical_session/preview")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| previews.get(&name).cloned())
                    .unwrap_or_default();
                let _ = roster::meta(p);
                Bot {
                    display_name: roster::display_name(p),
                    description: p.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                    model: p.get("model").and_then(Value::as_str).unwrap_or_default().to_string(),
                    is_default: p.get("is_default").and_then(Value::as_bool).unwrap_or(false),
                    preview,
                    avatar,
                    avatar_face,
                    name,
                }
            })
            .collect();
        self.roster_chats = profiles
            .iter()
            .filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), botchat::canonical_from_roster(p)?)))
            .collect();
        self.roster_rows = profiles
            .iter()
            .filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.clone())))
            .collect();
        self.load_avatars(server_id, with_avatar);
        cx.notify();
    }

    /// Fetches each bot's avatar from the server and swaps it into the roster.
    fn load_avatars(&mut self, server_id: String, names: Vec<String>) {
        let Some(gateway) = self.gateway.clone() else { return };
        if names.is_empty() {
            return;
        }
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let mut found = Vec::new();
            for name in names {
                let Ok(reply) = gateway.request("profiles.get_asset", json!({ "asset": "avatar", "name": name })).await else {
                    continue;
                };
                let data = reply.get("data").and_then(Value::as_str).filter(|_| reply["found"] != false);
                match data.and_then(|d| media::store_avatar(&server_id, &name, d)) {
                    Some(file) => {
                        let face = botedit::is_face_copy(&file);
                        found.push((name, Some(file), face));
                    }
                    None if reply["found"] == false => {
                        media::clear_avatar(&server_id, &name);
                        found.push((name, None, false));
                    }
                    None => {}
                }
            }
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                for (name, file, face) in found {
                    for bot in app.bots.iter_mut().filter(|b| b.name == name) {
                        bot.avatar = file.clone();
                        bot.avatar_face = face;
                    }
                }
                cx.notify();
            });
        });
    }

    fn handle_incoming(&mut self, generation: u64, incoming: Incoming, cx: &mut Context<Self>) {
        if self.generation != generation {
            return;
        }
        match incoming {
            Incoming::Event { kind, session_id, payload } => {
                if kind.starts_with("display.") {
                    self.handle_display_event(&kind, &payload);
                    cx.notify();
                    return;
                }
                match kind.as_str() {
                    // Another client or a cron run wrote to a session store.
                    "sessions.changed" => self.mark_parked_stale(),
                    "groups.changed" => self.on_groups_changed(&payload, cx),
                    _ => {}
                }
                if session_id.is_empty() || session_id == self.live_session {
                    let (bot, live) = (self.active_bot.clone(), self.live_session.clone());
                    if let Some(completed) = self.chat_event(&kind, &payload, cx) {
                        self.after_turn(&bot, &live, completed, cx);
                    }
                } else if let Some(bot) = self.bot_for_live(&session_id) {
                    // A parked chat keeps up with its session.
                    let ended = self.with_bot_chat(&bot, cx, |app, cx| app.chat_event(&kind, &payload, cx)).flatten();
                    if let Some(completed) = ended {
                        self.after_turn(&bot, &session_id, completed, cx);
                    }
                } else if matches!(kind.as_str(), "message.complete" | "error") {
                    // A turn finishing in a bot whose chat isn't loaded.
                    let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                    if let Some(profile) = self.mark_finished(&session_id) {
                        let final_text = text("text");
                        if kind == "message.complete" && !final_text.trim().is_empty() {
                            self.set_bot_preview(&profile, &final_text);
                        }
                        let completed = kind == "message.complete" && matches!(text("status").as_str(), "" | "complete");
                        self.after_turn(&profile, &session_id, completed, cx);
                    }
                }
            }
            Incoming::ServerRequest { id, method, params } => {
                cx.emit(AppEvent::ServerRequest { id, method, params });
            }
            Incoming::Closed(reason) => {
                // Whatever was answering is lost; queued messages wait to be resumed.
                self.queues.values_mut().for_each(MessageQueue::park);
                self.gateway = None;
                self.running.clear();
                self.parked.clear();
                session::set(None);
                self.connected = false;
                self.streaming = false;
                if reason == UNAUTHORIZED {
                    self.status = "Signed out".into();
                    self.error = "The server ended this sign-in. Sign in again.".into();
                    cx.notify();
                    return;
                }
                self.status = "Reconnecting…".into();
                self.error = reason;
                let queue = self.queue.clone();
                runtime().spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    queue.queue(move |app, cx| {
                        if app.generation == generation {
                            app.connect_active(cx);
                        }
                    });
                });
            }
        }
        cx.notify();
    }

    /// Applies a session event to the open chat (or a parked one swapped
    /// in). `Some(completed)` when the turn ended.
    fn chat_event(&mut self, kind: &str, payload: &Value, _cx: &mut Context<Self>) -> Option<bool> {
        let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        match kind {
            "session.info" => {
                if payload.is_object() {
                    self.session_info = payload.clone();
                    if let Some(usage) = payload.get("usage").filter(|u| u.is_object()) {
                        self.apply_usage(usage);
                    }
                }
            }
            "session.usage" => self.apply_usage(&payload),
            "message.start" => self.on_assistant_started(),
            "message.delta" => {
                let delta = text("text");
                if !delta.is_empty() {
                    self.on_assistant_delta(&delta);
                }
            }
            "reasoning.delta" | "thinking.delta" => self.on_reasoning_delta(&text("text")),
            "tool.start" => {
                let label = payload
                    .get("labels")
                    .and_then(|l| l.get(0))
                    .and_then(|l| l.get("text").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                let detail = [label, text("preview"), text("context")]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .unwrap_or_default();
                let args = match payload.get("args") {
                    Some(a) if !a.is_null() => history::pretty_args(a),
                    _ => text("args_text"),
                };
                self.on_tool_started(text("tool_id"), text("name"), detail, args);
            }
            "tool.complete" => {
                let (mut result, mut diff, mut error) = history::tool_result(payload.get("result").unwrap_or(&Value::Null));
                if result.is_empty() {
                    result = [text("result_text"), text("summary")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
                }
                if !text("inline_diff").is_empty() {
                    diff = text("inline_diff");
                }
                if !text("error").is_empty() {
                    error = text("error");
                }
                let todos = payload.get("todos").map(chat::todos_from).unwrap_or_default();
                let duration = payload.get("duration_s").and_then(Value::as_f64).unwrap_or(0.0);
                self.on_tool_finished(chat::ToolDone {
                    tool_id: text("tool_id"),
                    name: text("name"),
                    summary: text("summary"),
                    result,
                    diff,
                    todos,
                    duration,
                    error,
                });
            }
            "message.interim" => {
                let streamed = payload.get("already_streamed").and_then(Value::as_bool).unwrap_or(false);
                self.on_assistant_interim(&text("text"), streamed);
            }
            "todo.updated" => {
                if let Some(todos) = payload.get("todos").filter(|t| t.is_array()) {
                    self.on_todos_updated(chat::todos_from(todos));
                }
            }
            "message.complete" => {
                let live = self.live_session.clone();
                self.mark_finished(&live);
                self.streaming = false;
                if let Some(usage) = payload.get("usage").filter(|u| u.is_object()) {
                    self.apply_usage(usage);
                }
                let status = Some(text("status")).filter(|s| !s.is_empty()).unwrap_or_else(|| "complete".into());
                let error = Some(text("error")).filter(|s| !s.is_empty()).unwrap_or_else(|| text("failure_reason"));
                let final_text = text("text");
                if !final_text.trim().is_empty() {
                    let active = self.active_bot.clone();
                    self.set_bot_preview(&active, &final_text);
                }
                self.on_turn_finished(&status, &final_text, &error);
                return Some(status == "complete");
            }
            "error" => {
                let live = self.live_session.clone();
                self.mark_finished(&live);
                self.streaming = false;
                if self.backstage {
                    // Out of sight: the error waits in that bot's chat.
                    self.on_turn_finished("error", "", &text("message"));
                } else {
                    self.error = text("message");
                }
                return Some(false);
            }
            _ => {}
        }
        None
    }

    // ── servers ──

    pub fn probe_server(&mut self, url: &str, cx: &mut Context<Self>) {
        let url = match store::normalize_url(url) {
            Ok(url) => url,
            Err(e) => {
                self.error = e;
                cx.emit(AppEvent::ServerProbed(None));
                cx.notify();
                return;
            }
        };
        self.busy = true;
        self.error.clear();
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = auth::discover(&url).await;
            queue.queue(move |app, cx| {
                app.busy = false;
                match result {
                    Ok(info) => cx.emit(AppEvent::ServerProbed(Some(info))),
                    Err(e) => {
                        app.error = e;
                        cx.emit(AppEvent::ServerProbed(None));
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn sign_in(
        &mut self,
        url: &str,
        name: &str,
        provider: &str,
        use_password: bool,
        username: &str,
        password: &str,
        cx: &mut Context<Self>,
    ) {
        let url = match store::normalize_url(url) {
            Ok(url) => url,
            Err(e) => {
                cx.emit(AppEvent::SignInFinished(false, e));
                return;
            }
        };
        let existing = self.config.servers.iter().find(|s| s.url == url).cloned();
        let name = Some(name.trim().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| existing.as_ref().map(|s| s.name.clone()))
            .unwrap_or_else(|| store::default_name(&url));
        let server = Server {
            id: existing.map(|s| s.id).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            url,
            provider: provider.to_string(),
            username: if use_password { username.to_string() } else { String::new() },
        };
        let password = password.to_string();
        self.busy = true;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let tokens = if use_password {
                auth::password_sign_in(&server.url, &server.provider, &server.username, &password).await
            } else {
                auth::browser_sign_in(&server.url, &server.provider, |url| {
                    open::that_detached(url).map_err(|e| format!("Couldn't open the browser: {e}"))
                })
                .await
            };
            let result = tokens.and_then(|t| store::save_tokens(&server.id, &t));
            queue.queue(move |app, cx| {
                app.busy = false;
                match result {
                    Ok(()) => {
                        let config = &mut app.config;
                        match config.servers.iter_mut().find(|s| s.id == server.id) {
                            Some(slot) => *slot = server.clone(),
                            None => config.servers.push(server.clone()),
                        }
                        config.active_server = Some(server.id.clone());
                        app.save_config();
                        cx.emit(AppEvent::SignInFinished(true, String::new()));
                        app.connect_active(cx);
                    }
                    Err(e) => cx.emit(AppEvent::SignInFinished(false, e)),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn remove_server(&mut self, id: &str, cx: &mut Context<Self>) {
        store::delete_tokens(id);
        let was_active = self.config.active_server.as_deref() == Some(id);
        self.config.servers.retain(|s| s.id != id);
        self.config.last_bot.remove(id);
        if was_active {
            self.config.active_server = self.config.servers.first().map(|s| s.id.clone());
        }
        self.save_config();
        if was_active {
            self.connect_active(cx);
        }
        cx.notify();
    }

    pub fn select_server(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.config.server(id).is_none() {
            return;
        }
        self.config.active_server = Some(id.to_string());
        self.save_config();
        self.leave_group();
        self.groups.sends.clear();
        self.connect_active(cx);
    }

    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.connect_active(cx);
    }

    // ── bots and chats ──

    pub fn select_bot(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.active_group.is_some() {
            self.leave_group();
            if self.active_bot == name {
                // Its chat stayed loaded behind the group.
                cx.emit(AppEvent::TranscriptLoaded);
                cx.notify();
                return;
            }
        }
        if self.active_bot == name {
            return;
        }
        if let Some(server_id) = self.config.active_server.clone() {
            self.config.last_bot.insert(server_id, name.to_string());
            self.save_config();
        }
        self.park_active();
        self.show_bot(name, cx);
    }

    /// Makes `name` the open bot: its loaded chat comes back as it was,
    /// one not loaded yet opens. Whatever was open must be parked already.
    pub(super) fn show_bot(&mut self, name: &str, cx: &mut Context<Self>) {
        self.active_bot = name.to_string();
        match self.unpark(name) {
            Some(stale) => {
                self.refresh_bot_extras(cx);
                cx.emit(AppEvent::TranscriptLoaded);
                if self.live_session.is_empty() && !self.chat_loading {
                    // No Bot Chat yet, or its load failed: look again.
                    self.chat_loading = self.gateway.is_some();
                    self.spawn_open(name.to_string(), cx);
                } else if stale {
                    self.recheck_chat(cx);
                }
                cx.notify();
            }
            None => self.open_bot_chat(cx),
        }
    }

    /// The open bot's side things: jobs, screen and models.
    pub(super) fn refresh_bot_extras(&mut self, cx: &mut Context<Self>) {
        self.jobs.clear();
        self.jobs_error.clear();
        self.refresh_jobs(cx);
        self.screen = Value::Null;
        self.screen_log.clear();
        self.refresh_screen(cx);
        self.models.clear();
        self.refresh_models(cx);
    }

    /// Shows `text` as a bot's last message in the sidebar.
    fn set_bot_preview(&mut self, profile: &str, text: &str) {
        let preview: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
        for bot in self.bots.iter_mut().filter(|b| b.name == profile) {
            bot.preview = preview.clone();
        }
    }

    fn clear_chat(&mut self) {
        self.live_session.clear();
        self.active_session.clear();
        self.load_transcript(Vec::new());
    }

    fn apply_opened(&mut self, opened: Opened, view: Option<Vec<Value>>, cx: &mut Context<Self>) {
        if opened.info.is_object() {
            self.session_info = opened.info.clone();
            if let Some(usage) = opened.info.get("usage").filter(|u| u.is_object()) {
                self.apply_usage(usage);
            }
        }
        self.live_session = opened.live;
        self.active_session = opened.stored;
        // The chat's own command set adds its workspace's skills.
        let (profile, live) = (self.active_bot.clone(), self.live_session.clone());
        self.refresh_commands(&profile, &live);
        let loaded = match view {
            Some(rows) if !rows.is_empty() => {
                self.load_transcript(rows);
                true
            }
            _ if !opened.messages.is_empty() => {
                self.load_transcript(transcript_for_view(&opened.messages));
                true
            }
            _ => false,
        };
        if loaded && !self.backstage {
            cx.emit(AppEvent::TranscriptLoaded);
        }
        let live = self.live_session.clone();
        if opened.running || self.running.contains_key(&live) {
            let profile = self.active_bot.clone();
            self.mark_running(&live, &profile);
            self.streaming = true;
        }
        cx.notify();
    }

    /// Opens the active bot's Bot Chat. A bot that has none yet shows an empty
    /// chat; the Bot Chat is created when the first message is sent.
    fn open_bot_chat(&mut self, cx: &mut Context<Self>) {
        self.streaming = false;
        self.clear_chat();
        cx.emit(AppEvent::TranscriptLoaded);
        self.refresh_bot_extras(cx);
        self.session_info = Value::Null;
        self.usage = None;
        self.pending.clear();
        if self.gateway.is_none() {
            self.chat_loading = false;
            cx.notify();
            return;
        }
        self.chat_loading = true;
        let profile = self.active_bot.clone();
        // The profile's commands, for the menu before the chat exists.
        if !self.catalogs.contains_key(&profile) {
            self.refresh_commands(&profile, "");
        }
        self.spawn_open(profile, cx);
        cx.notify();
    }

    /// Looks up `profile`'s Bot Chat and resumes it into that bot's chat,
    /// open or parked.
    pub(super) fn spawn_open(&mut self, profile: String, _cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else { return };
        let roster = self.roster_chats.get(&profile).cloned();
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result: Result<Option<(Opened, Option<Vec<Value>>)>, String> = async {
                match botchat::lookup(&gateway, &profile, roster.as_deref()).await? {
                    Some(stored) => {
                        let opened = botchat::resume(&gateway, &profile, &stored).await?;
                        // The REST transcript has tool results; fall back to the
                        // socket's history if it can't be read.
                        let view = match session::current() {
                            Some(s) => history::load(&s, &opened.stored, &profile).await.ok(),
                            None => None,
                        };
                        Ok(Some((opened, view)))
                    }
                    None => Ok(None),
                }
            }
            .await;
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                app.with_bot_chat(&profile, cx, |app, cx| {
                    app.chat_loading = false;
                    match result {
                        Ok(Some((opened, view))) => app.apply_opened(opened, view, cx),
                        Ok(None) => {}
                        // Out of sight, a failed load is tried again when the bot is opened.
                        Err(_) if app.backstage => {}
                        Err(e) => app.error = e,
                    }
                });
                cx.notify();
            });
        });
    }

    pub fn send(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(command) = slash::parse(text) {
            self.error.clear();
            self.run_slash(command, cx);
            cx.notify();
            return;
        }
        self.submit(text.to_string(), text.to_string(), cx);
    }

    /// Sends `text` as a prompt with the waiting attachments; the bubble
    /// shows `echo` (a skill's invocation rather than its expanded body).
    fn submit(&mut self, text: String, echo: String, cx: &mut Context<Self>) {
        if self.streaming {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.submit_with(text, echo, pending, false, cx);
    }

    /// `submit` with its own attachments. `queued` marks a message from the
    /// queue: should the last turn still be ending, the server holds it
    /// rather than taking it as a correction.
    fn submit_with(&mut self, text: String, echo: String, pending: Vec<attachments::Attachment>, queued: bool, cx: &mut Context<Self>) {
        if text.trim().is_empty() && pending.is_empty() {
            return;
        }
        let Some(gateway) = self.gateway.clone() else {
            self.pending.extend(pending);
            self.set_error("Not connected.", cx);
            return;
        };
        self.error.clear();
        let echo_files: Vec<FileRef> = pending.iter().map(|a| FileRef::from_view(&a.to_view())).collect();
        self.on_user_message(&echo, echo_files.clone());
        self.streaming = true;
        let profile = self.active_bot.clone();
        let roster = self.roster_chats.get(&profile).cloned();
        let live = self.live_session.clone();
        self.mark_running(&live, &profile);
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let mut sent_on = live.clone();
            let result: Result<(), String> = async {
                let live = if live.is_empty() {
                    let opened = botchat::open_or_create(&gateway, &profile, roster.as_deref()).await?;
                    let live = opened.live.clone();
                    // Queued before the prompt goes out, so the UI thread knows the
                    // session id before any of its events arrive.
                    let echo = echo.clone();
                    let profile = profile.clone();
                    queue.queue(move |app, cx| {
                        if app.generation != generation {
                            return;
                        }
                        app.mark_running(&opened.live, &profile);
                        // Switched to another bot meanwhile: its parked chat
                        // takes the session.
                        app.with_bot_chat(&profile, cx, |app, cx| {
                            // The chat already existed (made elsewhere meanwhile): show
                            // its history, then this message again below it.
                            let had_history = !opened.messages.is_empty();
                            app.apply_opened(opened, None, cx);
                            if had_history {
                                app.on_user_message(&echo, echo_files);
                            }
                        });
                        cx.notify();
                    });
                    live
                } else {
                    live
                };
                sent_on = live.clone();
                // Images are queued on the session for this prompt; files come
                // back as @file: references that lead the text.
                let mut refs = Vec::new();
                for attachment in &pending {
                    if let Some(reference) = attachments::upload(&gateway, &live, attachment).await? {
                        refs.push(reference);
                    }
                }
                let has_images = pending.iter().any(|a| a.kind == attachments::Kind::Image);
                let prompt = attachments::compose_prompt(&text, &refs, has_images);
                let mut params = json!({ "session_id": live, "text": prompt, "profile": profile });
                if queued {
                    params["queued"] = json!(true);
                }
                let submitted = gateway.request("prompt.submit", params).await?;
                if let Some(row_id) = submitted.get("user_row_id").and_then(Value::as_i64) {
                    let live = live.clone();
                    queue.queue(move |app, cx| {
                        if app.generation == generation {
                            app.with_live_chat(&live, cx, |app, _| app.on_user_row_id(row_id));
                            cx.notify();
                        }
                    });
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                queue.queue(move |app, cx| {
                    if app.generation == generation {
                        app.mark_finished(&sent_on);
                        app.with_bot_chat(&profile, cx, |app, _| app.streaming = false);
                        app.error = e;
                        cx.notify();
                    }
                });
            }
        });
        cx.notify();
    }

    // ── queued messages (see queued.rs) ──

    /// Queues are kept per server and bot.
    fn queue_key(&self, bot: &str) -> String {
        format!("{}/{bot}", self.config.active_server.as_deref().unwrap_or_default())
    }

    /// The open bot's queue, when it has messages waiting.
    pub fn queued(&self) -> Option<&MessageQueue> {
        self.queues.get(&self.queue_key(&self.active_bot)).filter(|q| !q.is_empty())
    }

    fn active_queue(&mut self) -> &mut MessageQueue {
        let key = self.queue_key(&self.active_bot);
        self.queues.entry(key).or_default()
    }

    /// Queues `text` and the waiting attachments to send after the current
    /// answer. With nothing answering it goes out now.
    pub fn queue_message(&mut self, text: &str, cx: &mut Context<Self>) {
        let attachments = std::mem::take(&mut self.pending);
        if text.trim().is_empty() && attachments.is_empty() {
            return;
        }
        if !self.streaming && self.connected {
            self.submit_with(text.to_string(), text.to_string(), attachments, false, cx);
            return;
        }
        self.next_attachment += 1;
        let message = QueuedMessage { id: self.next_attachment, text: text.to_string(), attachments };
        self.active_queue().items.push(message);
        cx.notify();
    }

    pub fn remove_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        self.active_queue().remove(id);
        cx.notify();
    }

    /// Takes a queued message back out to edit it: its attachments rejoin
    /// the waiting ones and its text is returned for the message box.
    pub fn take_queued(&mut self, id: u64, cx: &mut Context<Self>) -> Option<String> {
        let message = self.active_queue().remove(id)?;
        self.pending.extend(message.attachments);
        cx.notify();
        Some(message.text)
    }

    /// Delivers a queued message into the running answer instead of after it.
    pub fn steer_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        let steerable = self.queued().and_then(|q| q.items.iter().find(|m| m.id == id)).is_some_and(QueuedMessage::can_steer);
        if !self.streaming || !steerable {
            return;
        }
        if let Some(message) = self.active_queue().remove(id) {
            self.steer(&message.text, cx);
        }
        cx.notify();
    }

    /// Sends a queued message now, while nothing is answering.
    pub fn send_queued_now(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.streaming {
            return;
        }
        if let Some(message) = self.active_queue().remove(id) {
            self.send_queued(message, cx);
        }
    }

    /// Lifts a park; with nothing answering, the next message goes out.
    pub fn resume_queue(&mut self, cx: &mut Context<Self>) {
        self.active_queue().parked = false;
        if !self.streaming && self.connected {
            let (bot, live) = (self.active_bot.clone(), self.live_session.clone());
            self.after_turn(&bot, &live, true, cx);
        }
        cx.notify();
    }

    fn send_queued(&mut self, message: QueuedMessage, cx: &mut Context<Self>) {
        if let Some(command) = slash::parse(&message.text) {
            self.pending.extend(message.attachments);
            self.run_slash(command, cx);
        } else {
            self.submit_with(message.text.clone(), message.text, message.attachments, true, cx);
        }
        cx.notify();
    }

    /// A turn ended on `bot`'s chat `live`. A finished turn sends the next
    /// queued message; a stopped or failed one parks the queue.
    fn after_turn(&mut self, bot: &str, live: &str, completed: bool, cx: &mut Context<Self>) {
        let key = self.queue_key(bot);
        let Some(queue) = self.queues.get_mut(&key) else { return };
        if !completed {
            queue.park();
            return;
        }
        if bot == self.active_bot {
            if let Some(message) = queue.next() {
                self.send_queued(message, cx);
            }
            return;
        }
        // A bot whose chat isn't open: its messages go out unseen and show
        // when the chat is opened. Commands wait for the chat to be open.
        if queue.items.first().is_some_and(QueuedMessage::is_command) {
            queue.park();
            return;
        }
        let Some(message) = queue.next() else { return };
        if self.parked.contains_key(bot) {
            // Its parked chat shows the message going out.
            self.with_bot_chat(bot, cx, |app, cx| {
                app.submit_with(message.text.clone(), message.text, message.attachments, true, cx)
            });
        } else {
            self.submit_in_background(bot, live, message);
        }
    }

    /// Sends a queued message to a bot whose chat isn't open.
    fn submit_in_background(&mut self, bot: &str, live: &str, message: QueuedMessage) {
        let Some(gateway) = self.gateway.clone() else { return };
        let (profile, live) = (bot.to_string(), live.to_string());
        self.mark_running(&live, &profile);
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result: Result<(), String> = async {
                let mut refs = Vec::new();
                for attachment in &message.attachments {
                    if let Some(reference) = attachments::upload(&gateway, &live, attachment).await? {
                        refs.push(reference);
                    }
                }
                let has_images = message.attachments.iter().any(|a| a.kind == attachments::Kind::Image);
                let prompt = attachments::compose_prompt(&message.text, &refs, has_images);
                gateway
                    .request("prompt.submit", json!({ "session_id": live, "text": prompt, "profile": profile, "queued": true }))
                    .await
                    .map(|_| ())
            }
            .await;
            if let Err(e) = result {
                queue.queue(move |app, cx| {
                    if app.generation != generation {
                        return;
                    }
                    app.mark_finished(&live);
                    // Back at the front, held, so it isn't lost.
                    let key = app.queue_key(&profile);
                    let held = app.queues.entry(key).or_default();
                    held.items.insert(0, message);
                    held.parked = true;
                    app.error = format!("Couldn't send a queued message to {profile}: {e}");
                    cx.notify();
                });
            }
        });
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let live = self.live_session.clone();
        if live.is_empty() {
            self.streaming = false;
            cx.notify();
            return;
        }
        let profile = self.active_bot.clone();
        self.call("session.interrupt", json!({ "session_id": live, "profile": profile }), cx, |_, _, _| {});
    }

    /// Adds guidance to the running turn (after the current tool).
    pub fn steer(&mut self, text: &str, cx: &mut Context<Self>) {
        let live = self.live_session.clone();
        if text.trim().is_empty() || live.is_empty() {
            return;
        }
        let profile = self.active_bot.clone();
        let echo = text.to_string();
        self.call("session.steer", json!({ "session_id": live, "text": text, "profile": profile }), cx, move |app, reply, _| {
            let status = reply.get("status").and_then(Value::as_str).unwrap_or("queued");
            app.on_steered(&echo, status);
        });
    }

    /// Edit or regenerate: replaces your message at row `ix` with `text`,
    /// dropping it and everything after it here and on the server. The
    /// message keeps its attachments.
    pub fn resend_from(&mut self, ix: usize, text: &str, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix).filter(|r| r.role == Role::User).cloned() else { return };
        if self.streaming || (text.trim().is_empty() && row.attachments.is_empty()) {
            return;
        }
        if row.row_id < 0 {
            self.set_error("This message can't be changed yet; reopen the chat and try again.", cx);
            return;
        }
        let live = self.live_session.clone();
        let Some(gateway) = self.gateway.clone() else {
            self.set_error("Not connected.", cx);
            return;
        };
        if live.is_empty() {
            return;
        }
        let plan = edit::plan(&self.rows, ix);
        self.editing = None;
        self.truncate_rows(ix);
        self.error.clear();
        self.on_user_message(text, row.attachments.clone());
        self.streaming = true;
        let profile = self.active_bot.clone();
        self.mark_running(&live, &profile);
        let generation = self.generation;
        let queue = self.queue.clone();
        let text = text.to_string();
        runtime().spawn(async move {
            let result: Result<Value, String> = async {
                // Images go back on the session for this prompt; files ride
                // along as @file: references, as when first sent.
                let mut refs = Vec::new();
                for file in &row.attachments {
                    if file.path.is_empty() {
                        continue;
                    }
                    if file.kind == "image" {
                        gateway
                            .request("image.attach", json!({ "session_id": live, "path": file.path }))
                            .await
                            .map_err(|e| format!("Couldn't attach {}: {e}", file.name))?;
                    } else {
                        refs.push(format!("@file:{}", file.path));
                    }
                }
                let has_images = row.attachments.iter().any(|f| f.kind == "image" && !f.path.is_empty());
                let mut params = json!({
                    "session_id": live,
                    "text": attachments::compose_prompt(&text, &refs, has_images),
                    "profile": profile,
                });
                plan.add_params(&mut params);
                gateway.request("prompt.submit", params).await
            }
            .await;
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                match result {
                    Ok(reply) => {
                        app.with_live_chat(&live, cx, |app, _| {
                            app.rebind_row_ids(&plan, &reply);
                            if let Some(id) = reply.get("user_row_id").and_then(Value::as_i64) {
                                app.on_user_row_id(id);
                            }
                        });
                    }
                    Err(e) => {
                        app.mark_finished(&live);
                        app.with_live_chat(&live, cx, |app, _| app.streaming = false);
                        app.error = e;
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// The server rewrites the rows it keeps when it cuts the history, so
    /// the messages above the edit get new ids.
    fn rebind_row_ids(&mut self, plan: &edit::Plan, reply: &Value) {
        let Some(rebind) = edit::Rebind::from_reply(reply) else { return };
        for ix in 0..self.rows.len().min(plan.cut) {
            let old = self.rows[ix].row_id;
            let user_ordinal = plan.user_ordinals.iter().position(|&u| u == ix);
            if let Some(new) = rebind.new_id(old, user_ordinal) {
                self.rows[ix].row_id = new;
            }
        }
    }

    /// Starts editing your message at row `ix` in place.
    pub fn start_edit(&mut self, ix: usize) {
        let Some(row) = self.rows.get(ix).filter(|r| r.role == Role::User) else { return };
        let previous = self.editing.replace(row.id);
        self.remeasure_row_id(previous);
        self.list.splice(ix..ix + 1, 1);
    }

    pub fn cancel_edit(&mut self) {
        let previous = self.editing.take();
        self.remeasure_row_id(previous);
    }

    /// The row being edited, by index.
    pub fn editing_index(&self) -> Option<usize> {
        let id = self.editing?;
        self.rows.iter().position(|r| r.id == id)
    }

    fn remeasure_row_id(&mut self, id: Option<u64>) {
        if let Some(ix) = id.and_then(|id| self.rows.iter().position(|r| r.id == id)) {
            self.list.splice(ix..ix + 1, 1);
        }
    }

    pub fn answer_request(&mut self, id: Value, result: Value) {
        if let Some(gateway) = &self.gateway {
            gateway.respond(id, result);
        }
    }

    // ── jobs ──

    /// Reloads the active bot's scheduled jobs. Failures stay in `jobs_error`
    /// so the chat's own error bar isn't taken over by a background poll.
    pub fn refresh_jobs(&mut self, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else { return };
        if self.jobs_loading {
            return;
        }
        let profile = self.active_bot.clone();
        if profile.is_empty() {
            return;
        }
        self.jobs_loading = true;
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = gateway
                .request("cron.manage", json!({ "action": "list", "include_disabled": true, "profile": profile }))
                .await;
            queue.queue(move |app, cx| {
                app.jobs_loading = false;
                cx.notify();
                if app.generation != generation || app.active_bot != profile {
                    return;
                }
                match result {
                    Ok(reply) => match reply.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                        Some(e) => app.jobs_error = e.to_string(),
                        None => {
                            app.jobs_error.clear();
                            app.jobs = cron::jobs_for_bot(&reply, &profile);
                        }
                    },
                    Err(e) => app.jobs_error = e,
                }
            });
        });
        cx.notify();
    }

    // ── attachments ──

    fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut problems = Vec::new();
        for path in paths {
            self.next_attachment += 1;
            match attachments::from_path(self.next_attachment, &path) {
                Ok(attachment) => self.pending.push(attachment),
                Err(e) => problems.push(e),
            }
        }
        if !problems.is_empty() {
            self.error = problems.join(" ");
        }
        cx.notify();
    }

    pub fn pick_files(&mut self) {
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = attachments::pick_files().await;
            queue.queue(move |app, cx| match result {
                Ok(paths) => app.add_paths(paths, cx),
                Err(e) => app.set_error(e, cx),
            });
        });
    }

    /// Files dropped on the window.
    pub fn add_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.add_paths(paths.to_vec(), cx);
    }

    pub fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.pending.retain(|a| a.id != id);
        cx.notify();
    }

    /// Takes images, files and long texts from a paste as attachments; false
    /// means ordinary text the text box should paste itself.
    pub fn paste(&mut self, item: &gpui_kit::ClipboardItem, cx: &mut Context<Self>) -> bool {
        if let Some(paths) = item.entries().iter().find_map(|e| match e {
            gpui_kit::ClipboardEntry::ExternalPaths(p) => Some(p.paths().to_vec()),
            _ => None,
        }) {
            self.add_paths(paths, cx);
            return true;
        }
        match attachments::paste_clipboard(&ClipboardItemSource(item), &attachments::scratch_dir()) {
            Ok(attachments::Pasted::File(path)) => {
                self.add_paths(vec![path], cx);
                true
            }
            Ok(attachments::Pasted::Text) | Ok(attachments::Pasted::Nothing) => false,
            Err(e) => {
                self.set_error(e, cx);
                false
            }
        }
    }

    // ── media ──

    /// Fetches a server-side file (an image) into the local cache.
    pub fn request_media(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.media.contains_key(path) {
            return;
        }
        let Some(session) = session::current() else { return };
        self.media.insert(path.to_string(), MediaState::Loading);
        let profile = self.active_bot.clone();
        let queue = self.queue.clone();
        let path = path.to_string();
        runtime().spawn(async move {
            let result = media::fetch(&session, &path, &profile).await;
            queue.queue(move |app, cx| {
                let state = match result {
                    Ok(file) => MediaState::Ready(file),
                    Err(_) => MediaState::Failed,
                };
                app.media.insert(path, state);
                // The picture replaces its placeholder: rows change height.
                app.list.remeasure();
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Fetches a server-side file if needed and opens it with the desktop.
    pub fn open_media(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(MediaState::Ready(file)) = self.media.get(path) {
            let _ = open::that_detached(file);
            return;
        }
        let Some(session) = session::current() else { return };
        let profile = self.active_bot.clone();
        let queue = self.queue.clone();
        let path = path.to_string();
        runtime().spawn(async move {
            let result = media::fetch(&session, &path, &profile).await;
            queue.queue(move |app, cx| {
                match result {
                    Ok(file) => {
                        let _ = open::that_detached(&file);
                        app.media.insert(path, MediaState::Ready(file));
                    }
                    Err(e) => app.error = format!("Couldn't open {path}: {e}"),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    // ── models ──

    pub fn refresh_models(&mut self, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else { return };
        let profile = self.active_bot.clone();
        if profile.is_empty() {
            return;
        }
        let live = self.live_session.clone();
        self.models_loading = true;
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let mut params = json!({ "profile": profile, "explicit_only": true });
            if !live.is_empty() {
                params["session_id"] = json!(live);
            }
            let result = gateway.request("model.options", params).await;
            queue.queue(move |app, cx| {
                app.models_loading = false;
                cx.notify();
                if app.generation != generation || app.active_bot != profile {
                    return;
                }
                match result {
                    Ok(reply) => app.models = models_for_view(&reply),
                    Err(e) => app.error = format!("Couldn't load models: {e}"),
                }
            });
        });
        cx.notify();
    }

    /// Runs a session-scoped `config.set` on the Bot Chat, opening (or
    /// creating) it first: without a session the server would write the
    /// bot's config.yaml instead.
    fn session_config(&mut self, key: &'static str, value: String, extra: Value, model: Option<(String, String)>, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else {
            self.set_error("Not connected.", cx);
            return;
        };
        let profile = self.active_bot.clone();
        let roster = self.roster_chats.get(&profile).cloned();
        let live = self.live_session.clone();
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result: Result<Value, String> = async {
                let live = if live.is_empty() {
                    let opened = botchat::open_or_create(&gateway, &profile, roster.as_deref()).await?;
                    let id = opened.live.clone();
                    queue.queue(move |app, cx| {
                        if app.generation == generation {
                            app.apply_opened(opened, None, cx);
                        }
                    });
                    id
                } else {
                    live
                };
                let mut params = json!({ "session_id": live, "key": key, "value": value, "profile": profile });
                if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
                    p.extend(e.clone());
                }
                gateway.request("config.set", params).await
            }
            .await;
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                match result {
                    Ok(reply) => {
                        if reply.get("confirm_required").and_then(Value::as_bool) == Some(true) {
                            let message = reply
                                .get("confirm_message")
                                .and_then(Value::as_str)
                                .unwrap_or("This model is expensive. Use it anyway?")
                                .to_string();
                            if let Some((model, provider)) = model {
                                cx.emit(AppEvent::ModelConfirm { message, model, provider });
                            }
                            return;
                        }
                        if let Some(info) = reply.get("info").filter(|i| i.is_object()) {
                            app.session_info = info.clone();
                        }
                        if let Some(warning) = reply.get("warning").and_then(Value::as_str).filter(|w| !w.is_empty()) {
                            app.error = warning.to_string();
                        }
                    }
                    Err(e) => app.error = e,
                }
                cx.notify();
            });
        });
    }

    /// `confirmed` resends after the user accepted an expensive model.
    pub fn set_model(&mut self, model: &str, provider: &str, confirmed: bool, cx: &mut Context<Self>) {
        // --session keeps the pick on this Bot Chat; the bot's profile is untouched.
        let mut value = model.to_string();
        if !provider.is_empty() {
            value.push_str(&format!(" --provider {provider}"));
        }
        value.push_str(" --session");
        let extra = if confirmed { json!({ "confirm_expensive_model": true }) } else { json!({}) };
        self.session_config("model", value, extra, Some((model.to_string(), provider.to_string())), cx);
    }

    pub fn set_effort(&mut self, level: &str, cx: &mut Context<Self>) {
        self.session_config("reasoning", level.to_string(), json!({ "scope": "session" }), None, cx);
    }

    pub fn set_fast(&mut self, on: bool, cx: &mut Context<Self>) {
        self.session_config("fast", if on { "fast" } else { "normal" }.to_string(), json!({}), None, cx);
    }

    /// Both usage shapes: `Usage` (context_used/max) and the
    /// `session.usage` tick (context_tokens/length).
    fn apply_usage(&mut self, u: &Value) {
        let num = |keys: &[&str]| keys.iter().find_map(|k| u.get(*k).and_then(Value::as_f64));
        let Some(used) = num(&["context_used", "context_tokens"]) else { return };
        let max = num(&["context_max", "context_length", "config_context_length"]).unwrap_or(0.0);
        let percent = num(&["context_percent"]).unwrap_or(if max > 0.0 { used / max * 100.0 } else { 0.0 });
        self.usage = Some(Usage {
            context_used: used,
            context_max: max,
            percent,
            cost: num(&["cost_usd"]).unwrap_or(-1.0),
        });
    }

    // ── the bot's screen ──

    /// The active bot's screen identity in display events.
    fn screen_key(&self) -> String {
        self.screen.get("profile_key").and_then(Value::as_str).unwrap_or_default().to_string()
    }

    fn handle_display_event(&mut self, kind: &str, payload: &Value) {
        let key = payload.get("profile_key").and_then(Value::as_str).unwrap_or_default();
        let profile = payload.get("profile").and_then(Value::as_str).unwrap_or_default();
        let ours = (!key.is_empty() && key == self.screen_key()) || (!profile.is_empty() && profile == self.active_bot);
        if !ours {
            return;
        }
        match kind {
            "display.status" => self.screen = payload.clone(),
            "display.lease" => {
                if self.screen.is_object() {
                    self.screen["lease"] = payload.get("lease").cloned().unwrap_or(Value::Null);
                }
            }
            "display.install.log" => {
                let line = payload.get("line").and_then(Value::as_str).unwrap_or_default();
                self.screen_log.push_str(line);
                self.screen_log.push('\n');
                // Keep the tail; installs can be chatty.
                if self.screen_log.len() > 20_000 {
                    let mut cut = self.screen_log.len() - 20_000;
                    while !self.screen_log.is_char_boundary(cut) {
                        cut += 1;
                    }
                    self.screen_log = self.screen_log[cut..].to_string();
                }
            }
            "display.install.done" => {
                self.screen_busy = false;
                if let Some(status) = payload.get("status").filter(|s| s.is_object()) {
                    self.screen = status.clone();
                }
                let code = payload.get("code").and_then(Value::as_i64).unwrap_or(1);
                let note = match code {
                    0 => "Installed.",
                    -1 => "Install cancelled.",
                    -2 => "No sudo access: run the command above on the server.",
                    _ => "Install failed; see the log above.",
                };
                self.screen_log.push_str(note);
                self.screen_log.push('\n');
            }
            _ => {}
        }
    }

    /// Runs a display RPC for the active bot and stores the status it returns.
    fn display_call(&mut self, method: &'static str, extra: Value, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else { return };
        let profile = self.active_bot.clone();
        if profile.is_empty() {
            return;
        }
        let mut params = json!({ "profile": profile });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        self.screen_busy = true;
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = gateway.request(method, params).await;
            queue.queue(move |app, cx| {
                if app.generation != generation || app.active_bot != profile {
                    return;
                }
                match result {
                    Ok(status) => {
                        // display.install answers {started}; its status arrives with install.done.
                        if method == "display.install" {
                            return;
                        }
                        app.screen_busy = false;
                        app.screen = status;
                    }
                    Err(e) => {
                        app.screen_busy = false;
                        if e.to_lowercase().contains("method not found") || e.contains("Unknown method") {
                            app.screen = json!({ "unavailable": true });
                        } else if method == "display.status" {
                            app.screen = json!({ "error": e });
                        } else {
                            app.error = e;
                        }
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn refresh_screen(&mut self, cx: &mut Context<Self>) {
        self.display_call("display.status", json!({}), cx);
    }

    pub fn start_screen(&mut self, cx: &mut Context<Self>) {
        self.display_call("display.start", json!({}), cx);
    }

    pub fn stop_screen(&mut self, force: bool, cx: &mut Context<Self>) {
        self.display_call("display.stop", json!({ "force": force }), cx);
    }

    pub fn install_screen(&mut self, cx: &mut Context<Self>) {
        self.screen_log.clear();
        self.display_call("display.install", json!({}), cx);
    }
}

/// Fresh tokens, the WebSocket, and the event pump onto the UI thread.
async fn open_gateway(server: &Server, queue: Queue<AppState>, generation: u64) -> Result<(Arc<Gateway>, String), String> {
    let id = server.id.clone();
    let mut tokens: Tokens = tokio::task::spawn_blocking(move || store::load_tokens(&id))
        .await
        .ok()
        .flatten()
        .ok_or_else(|| format!("Sign in to {} to continue.", server.name))?;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut refreshed = false;
    let gateway = loop {
        if tokens.expires_soon() && !refreshed {
            tokens = refresh_tokens(server, &tokens).await?;
            refreshed = true;
        }
        match Gateway::connect(&server.url, &tokens.access_token, tx.clone()).await {
            Ok(gateway) => break gateway,
            Err(e) if e == UNAUTHORIZED && !refreshed => {
                tokens = refresh_tokens(server, &tokens).await?;
                refreshed = true;
            }
            Err(e) if e == UNAUTHORIZED => return Err(format!("Sign in to {} again.", server.name)),
            Err(e) => return Err(e),
        }
    };
    gateway.request("client.capabilities", json!({ "server_requests": true })).await.ok();

    tokio::spawn(async move {
        while let Some(incoming) = rx.recv().await {
            let closed = matches!(incoming, Incoming::Closed(_));
            if !queue.queue(move |app, cx| app.handle_incoming(generation, incoming, cx)) || closed {
                break;
            }
        }
    });
    Ok((gateway, tokens.access_token))
}

async fn refresh_tokens(server: &Server, tokens: &Tokens) -> Result<Tokens, String> {
    match auth::refresh(&server.url, tokens).await {
        Ok(fresh) => {
            let id = server.id.clone();
            let to_save = fresh.clone();
            tokio::task::spawn_blocking(move || store::save_tokens(&id, &to_save))
                .await
                .map_err(|e| e.to_string())??;
            Ok(fresh)
        }
        Err(RefreshError::Expired) => {
            let id = server.id.clone();
            let _ = tokio::task::spawn_blocking(move || store::delete_tokens(&id)).await;
            Err(format!("Your sign-in to {} expired. Sign in again.", server.name))
        }
        Err(RefreshError::Other(e)) => Err(e),
    }
}

/// A paste as the attachment logic reads it.
struct ClipboardItemSource<'a>(&'a gpui_kit::ClipboardItem);

fn image_mime(format: gpui_kit::ImageFormat) -> &'static str {
    use gpui_kit::ImageFormat::*;
    match format {
        Png => "image/png",
        Jpeg => "image/jpeg",
        Webp => "image/webp",
        Gif => "image/gif",
        Svg => "image/svg+xml",
        Bmp => "image/bmp",
        _ => "application/octet-stream",
    }
}

impl attachments::Clipboard for ClipboardItemSource<'_> {
    fn formats(&self) -> Vec<String> {
        self.0
            .entries()
            .iter()
            .filter_map(|e| match e {
                gpui_kit::ClipboardEntry::Image(image) => Some(image_mime(image.format).to_string()),
                gpui_kit::ClipboardEntry::String(_) => Some("text/plain".to_string()),
                _ => None,
            })
            .collect()
    }
    fn data(&self, format: &str) -> Vec<u8> {
        self.0
            .entries()
            .iter()
            .find_map(|e| match e {
                gpui_kit::ClipboardEntry::Image(image) if image_mime(image.format) == format => Some(image.bytes.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }
    fn text(&self) -> String {
        self.0.text().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_transcripts_for_the_view() {
        let rows = transcript_for_view(&[
            json!({"role":"system","text":"sys"}),
            json!({"role":"user","text":"hi"}),
            json!({"role":"assistant","content":[{"type":"text","text":"hello"}],"reasoning":"think"}),
            json!({"role":"tool","name":"web_search"}),
            json!({"role":"assistant","content":""}),
        ]);
        let roles: Vec<_> = rows.iter().map(|r| r["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool"]);
        assert_eq!(rows[1]["text"], "hello");
        assert_eq!(rows[1]["reasoning"], "think");
        assert_eq!(rows[2]["name"], "web_search");
    }

    #[test]
    fn reads_model_options() {
        let providers = models_for_view(&json!({ "providers": [
            { "slug": "a", "name": "", "models": ["m1", { "id": "m2" }], "authenticated": false },
            { "slug": "b", "models": [] },
        ]}));
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "a");
        assert_eq!(providers[0].models, vec!["m1", "m2"]);
        assert!(!providers[0].authenticated);
    }
}

#[cfg(test)]
mod transcript_tests;
#[cfg(test)]
mod slots_tests;
#[cfg(test)]
mod queue_tests;
#[cfg(test)]
mod edit_tests;
