//! `HermesApp`: the app's state and actions, handed to QML.
//!
//! All network work runs on the tokio runtime; results are queued back onto
//! the Qt thread. Lists cross to QML as JSON strings and streaming arrives as
//! signals, which keeps the bridge small while the UI is plain.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, servers)]
        #[qproperty(QString, active_server, cxx_name = "activeServer")]
        #[qproperty(QString, status)]
        #[qproperty(QString, error)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, connected)]
        #[qproperty(QString, bots)]
        #[qproperty(QString, active_bot, cxx_name = "activeBot")]
        #[qproperty(QString, active_session, cxx_name = "activeSession")]
        #[qproperty(bool, streaming)]
        /// JSON array of the active bot's scheduled jobs.
        #[qproperty(QString, jobs)]
        #[qproperty(bool, jobs_loading, cxx_name = "jobsLoading")]
        #[qproperty(QString, jobs_error, cxx_name = "jobsError")]
        type HermesApp = super::AppRust;

        #[qinvokable]
        fn probe_server(self: Pin<&mut HermesApp>, url: QString);
        #[qinvokable]
        fn sign_in(
            self: Pin<&mut HermesApp>,
            url: QString,
            name: QString,
            provider: QString,
            use_password: bool,
            username: QString,
            password: QString,
        );
        #[qinvokable]
        fn remove_server(self: Pin<&mut HermesApp>, id: QString);
        #[qinvokable]
        fn select_server(self: Pin<&mut HermesApp>, id: QString);
        #[qinvokable]
        fn reconnect(self: Pin<&mut HermesApp>);
        #[qinvokable]
        fn select_bot(self: Pin<&mut HermesApp>, name: QString);
        #[qinvokable]
        fn send(self: Pin<&mut HermesApp>, text: QString);
        #[qinvokable]
        fn stop(self: Pin<&mut HermesApp>);
        #[qinvokable]
        fn refresh_jobs(self: Pin<&mut HermesApp>);
        #[qinvokable]
        fn answer_request(self: Pin<&mut HermesApp>, id: QString, result: QString);

        /// JSON `ServerInfo`, or an empty string with `error` set.
        #[qsignal]
        fn server_probed(self: Pin<&mut HermesApp>, info: QString);
        #[qsignal]
        fn sign_in_finished(self: Pin<&mut HermesApp>, ok: bool, message: QString);
        /// JSON array of `{role, text}`; replaces the conversation.
        #[qsignal]
        fn transcript_loaded(self: Pin<&mut HermesApp>, messages: QString);
        #[qsignal]
        fn user_message(self: Pin<&mut HermesApp>, text: QString);
        #[qsignal]
        fn assistant_started(self: Pin<&mut HermesApp>);
        #[qsignal]
        fn assistant_delta(self: Pin<&mut HermesApp>, text: QString);
        #[qsignal]
        fn reasoning_delta(self: Pin<&mut HermesApp>, text: QString);
        #[qsignal]
        fn tool_started(self: Pin<&mut HermesApp>, id: QString, name: QString, detail: QString);
        #[qsignal]
        fn tool_finished(self: Pin<&mut HermesApp>, id: QString, name: QString, summary: QString);
        /// `status` is complete, error or interrupted; `text` is the final answer.
        #[qsignal]
        fn turn_finished(self: Pin<&mut HermesApp>, status: QString, text: QString, error: QString);
        /// The agent asks something (approval, clarification). `id` is JSON.
        #[qsignal]
        fn server_request(self: Pin<&mut HermesApp>, id: QString, method: QString, params: QString);
    }

    impl cxx_qt::Threading for HermesApp {}
    impl cxx_qt::Initialize for HermesApp {}
}

use core::pin::Pin;
use std::collections::HashMap;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::backend::auth::{self, RefreshError};
use crate::backend::botchat::{self, Opened};
use crate::backend::cron;
use crate::backend::gateway::{Gateway, Incoming, UNAUTHORIZED};
use crate::backend::runtime;
use crate::backend::store::{self, Config, Server, Tokens};

#[derive(Default)]
pub struct AppRust {
    servers: QString,
    active_server: QString,
    status: QString,
    error: QString,
    busy: bool,
    connected: bool,
    bots: QString,
    active_bot: QString,
    active_session: QString,
    streaming: bool,
    jobs: QString,
    jobs_loading: bool,
    jobs_error: QString,

    config: Config,
    gateway: Option<Arc<Gateway>>,
    base_url: String,
    /// The live (gateway) id of the open conversation; events carry it.
    live_session: String,
    /// Bumped on every (re)connect so stale callbacks are ignored.
    generation: u64,
    /// Each bot's Bot Chat as the roster reports it (`canonical_session`),
    /// the fallback when a lookup answers empty.
    roster_chats: HashMap<String, String>,
}

fn q(s: impl AsRef<str>) -> QString {
    QString::from(s.as_ref())
}

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

/// The server's transcript, reduced to what the conversation view shows.
pub fn transcript_for_view(messages: &[Value]) -> Value {
    let rows: Vec<Value> = messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
            match role {
                "user" | "assistant" => {
                    let text = message_text(m);
                    let reasoning = m.get("reasoning").and_then(Value::as_str).unwrap_or_default();
                    (!text.trim().is_empty()).then(|| json!({ "role": role, "text": text, "reasoning": reasoning }))
                }
                "tool" => {
                    let name = m.get("name").and_then(Value::as_str).unwrap_or("tool");
                    Some(json!({ "role": "tool", "text": name, "reasoning": "" }))
                }
                _ => None,
            }
        })
        .collect();
    Value::Array(rows)
}

impl qobject::HermesApp {
    fn save_config(mut self: Pin<&mut Self>) {
        if let Err(e) = store::save(&store::config_path(), &self.rust().config) {
            self.as_mut().set_error(q(e));
        }
        self.publish_servers();
    }

    fn publish_servers(mut self: Pin<&mut Self>) {
        let list: Vec<Value> = self
            .rust()
            .config
            .servers
            .iter()
            .map(|s| json!({ "id": s.id, "name": s.name, "url": s.url, "provider": s.provider, "username": s.username }))
            .collect();
        let active = self.rust().config.active_server.clone().unwrap_or_default();
        self.as_mut().set_servers(q(Value::Array(list).to_string()));
        self.as_mut().set_active_server(q(active));
    }

    fn set_status_text(self: Pin<&mut Self>, text: &str) {
        self.set_status(q(text));
    }

    fn disconnect(mut self: Pin<&mut Self>) {
        let mut rust = self.as_mut().rust_mut();
        rust.generation += 1;
        if let Some(gateway) = rust.gateway.take() {
            gateway.close();
        }
        rust.live_session.clear();
        rust.roster_chats.clear();
        self.as_mut().set_connected(false);
        self.as_mut().set_streaming(false);
        self.as_mut().set_bots(q("[]"));
        self.as_mut().set_jobs(q("[]"));
        self.as_mut().set_jobs_error(q(""));
        self.as_mut().set_active_session(q(""));
        self.as_mut().transcript_loaded(q("[]"));
    }

    /// Connects to the active server with its stored tokens, refreshing them
    /// first when they're about to expire.
    fn connect_active(mut self: Pin<&mut Self>) {
        self.as_mut().disconnect();
        let Some(server) = self
            .rust()
            .config
            .active_server
            .as_ref()
            .and_then(|id| self.rust().config.server(id))
            .cloned()
        else {
            self.set_status_text("Add a server to start.");
            return;
        };
        self.as_mut().rust_mut().base_url = server.url.clone();
        let generation = self.rust().generation;
        self.as_mut().set_status_text(&format!("Connecting to {}…", server.name));
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = open_gateway(&server, qt.clone(), generation).await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation {
                    return;
                }
                match result {
                    Ok(gateway) => {
                        app.as_mut().rust_mut().gateway = Some(gateway);
                        app.as_mut().set_connected(true);
                        app.as_mut().set_error(q(""));
                        app.as_mut().set_status_text(&format!("Connected to {}", server.name));
                        app.as_mut().load_bots();
                    }
                    Err(e) => {
                        app.as_mut().set_status_text(&format!("Not connected to {}", server.name));
                        app.as_mut().set_error(q(e));
                    }
                }
            });
        });
    }

    /// Runs one gateway request and hands the result to `done` on the Qt thread.
    fn call(
        mut self: Pin<&mut Self>,
        method: &'static str,
        params: Value,
        done: impl FnOnce(Pin<&mut Self>, Value) + Send + 'static,
    ) {
        let Some(gateway) = self.rust().gateway.clone() else {
            self.as_mut().set_error(q("Not connected."));
            return;
        };
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = gateway.request(method, params).await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation {
                    return;
                }
                match result {
                    Ok(value) => done(app, value),
                    Err(e) => {
                        app.as_mut().set_streaming(false);
                        app.as_mut().set_error(q(e));
                    }
                }
            });
        });
    }

    fn load_bots(self: Pin<&mut Self>) {
        self.call("profiles.list", json!({ "include_sessions": true }), |mut app, result| {
            let profiles = result.get("profiles").and_then(Value::as_array).cloned().unwrap_or_default();
            let bots: Vec<Value> = profiles
                .iter()
                .map(|p| {
                    let name = p.get("name").and_then(Value::as_str).unwrap_or("default");
                    json!({
                        "name": name,
                        "displayName": p.get("display_name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(name),
                        "description": p.get("description").and_then(Value::as_str).unwrap_or_default(),
                        "model": p.get("model").and_then(Value::as_str).unwrap_or_default(),
                        "isDefault": p.get("is_default").and_then(Value::as_bool).unwrap_or(false),
                        "preview": p.pointer("/canonical_session/preview").and_then(Value::as_str).unwrap_or_default(),
                    })
                })
                .collect();
            let server_id = app.rust().config.active_server.clone().unwrap_or_default();
            let remembered = app.rust().config.last_bot.get(&server_id).cloned();
            let pick = remembered
                .filter(|name| bots.iter().any(|b| b["name"] == *name))
                .or_else(|| bots.iter().find(|b| b["isDefault"] == true).and_then(|b| b["name"].as_str().map(str::to_string)))
                .or_else(|| bots.first().and_then(|b| b["name"].as_str().map(str::to_string)))
                .unwrap_or_else(|| "default".into());
            app.as_mut().rust_mut().roster_chats = profiles
                .iter()
                .filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), botchat::canonical_from_roster(p)?)))
                .collect();
            app.as_mut().set_bots(q(Value::Array(bots).to_string()));
            app.as_mut().set_active_bot(q(&pick));
            app.as_mut().open_bot_chat();
        });
    }

    fn handle_incoming(mut self: Pin<&mut Self>, generation: u64, incoming: Incoming) {
        if self.rust().generation != generation {
            return;
        }
        match incoming {
            Incoming::Event { kind, session_id, payload } => {
                let ours = session_id.is_empty() || session_id == self.rust().live_session;
                let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
                match kind.as_str() {
                    _ if !ours => {}
                    "message.start" => self.as_mut().assistant_started(),
                    "message.delta" => {
                        let delta = text("text");
                        if !delta.is_empty() {
                            self.as_mut().assistant_delta(q(delta));
                        }
                    }
                    "reasoning.delta" | "thinking.delta" => self.as_mut().reasoning_delta(q(text("text"))),
                    "tool.start" => {
                        let detail = Some(text("preview"))
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| text("args_text"));
                        self.as_mut().tool_started(q(text("tool_id")), q(text("name")), q(detail));
                    }
                    "tool.complete" => {
                        let summary = Some(text("summary"))
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| text("result_text"));
                        self.as_mut().tool_finished(q(text("tool_id")), q(text("name")), q(summary));
                    }
                    "message.complete" => {
                        self.as_mut().set_streaming(false);
                        let status = Some(text("status")).filter(|s| !s.is_empty()).unwrap_or_else(|| "complete".into());
                        let error = Some(text("error")).filter(|s| !s.is_empty()).unwrap_or_else(|| text("failure_reason"));
                        let final_text = text("text");
                        if !final_text.trim().is_empty() {
                            self.as_mut().set_bot_preview(&final_text);
                        }
                        self.as_mut().turn_finished(q(status), q(final_text), q(error));
                    }
                    "error" => {
                        self.as_mut().set_streaming(false);
                        self.as_mut().set_error(q(text("message")));
                    }
                    _ => {}
                }
            }
            Incoming::ServerRequest { id, method, params } => {
                self.as_mut().server_request(q(id.to_string()), q(method), q(params.to_string()));
            }
            Incoming::Closed(reason) => {
                self.as_mut().rust_mut().gateway = None;
                self.as_mut().set_connected(false);
                self.as_mut().set_streaming(false);
                if reason == UNAUTHORIZED {
                    self.as_mut().set_status_text("Signed out");
                    self.as_mut().set_error(q("The server ended this sign-in. Sign in again."));
                    return;
                }
                self.as_mut().set_status_text("Reconnecting…");
                self.as_mut().set_error(q(reason));
                let qt = self.qt_thread();
                runtime().spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    let _ = qt.queue(move |app| {
                        if app.rust().generation == generation {
                            app.connect_active();
                        }
                    });
                });
            }
        }
    }

    // ---- invokables ----

    pub fn probe_server(mut self: Pin<&mut Self>, url: QString) {
        let url = match store::normalize_url(&url.to_string()) {
            Ok(url) => url,
            Err(e) => {
                self.as_mut().set_error(q(e));
                self.as_mut().server_probed(q(""));
                return;
            }
        };
        self.as_mut().set_busy(true);
        self.as_mut().set_error(q(""));
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = auth::discover(&url).await;
            let _ = qt.queue(move |mut app| {
                app.as_mut().set_busy(false);
                match result {
                    Ok(info) => app.as_mut().server_probed(q(serde_json::to_string(&info).unwrap_or_default())),
                    Err(e) => {
                        app.as_mut().set_error(q(e));
                        app.as_mut().server_probed(q(""));
                    }
                }
            });
        });
    }

    pub fn sign_in(
        mut self: Pin<&mut Self>,
        url: QString,
        name: QString,
        provider: QString,
        use_password: bool,
        username: QString,
        password: QString,
    ) {
        let url = match store::normalize_url(&url.to_string()) {
            Ok(url) => url,
            Err(e) => {
                self.as_mut().sign_in_finished(false, q(e));
                return;
            }
        };
        let existing = self.rust().config.servers.iter().find(|s| s.url == url).cloned();
        let name = Some(name.to_string().trim().to_string())
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
        self.as_mut().set_busy(true);
        let qt = self.qt_thread();
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
            let _ = qt.queue(move |mut app| {
                app.as_mut().set_busy(false);
                match result {
                    Ok(()) => {
                        let config = &mut app.as_mut().rust_mut().config;
                        match config.servers.iter_mut().find(|s| s.id == server.id) {
                            Some(slot) => *slot = server.clone(),
                            None => config.servers.push(server.clone()),
                        }
                        config.active_server = Some(server.id.clone());
                        app.as_mut().save_config();
                        app.as_mut().sign_in_finished(true, q(""));
                        app.as_mut().connect_active();
                    }
                    Err(e) => app.as_mut().sign_in_finished(false, q(e)),
                }
            });
        });
    }

    pub fn remove_server(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        store::delete_tokens(&id);
        let was_active = self.rust().config.active_server.as_deref() == Some(id.as_str());
        {
            let config = &mut self.as_mut().rust_mut().config;
            config.servers.retain(|s| s.id != id);
            config.last_bot.remove(&id);
            if was_active {
                config.active_server = config.servers.first().map(|s| s.id.clone());
            }
        }
        self.as_mut().save_config();
        if was_active {
            self.as_mut().connect_active();
        }
    }

    pub fn select_server(mut self: Pin<&mut Self>, id: QString) {
        let id = id.to_string();
        if self.rust().config.server(&id).is_none() {
            return;
        }
        self.as_mut().rust_mut().config.active_server = Some(id);
        self.as_mut().save_config();
        self.as_mut().connect_active();
    }

    pub fn reconnect(self: Pin<&mut Self>) {
        self.connect_active();
    }

    pub fn select_bot(mut self: Pin<&mut Self>, name: QString) {
        let name = name.to_string();
        if let Some(server_id) = self.rust().config.active_server.clone() {
            self.as_mut().rust_mut().config.last_bot.insert(server_id, name.clone());
            self.as_mut().save_config();
        }
        self.as_mut().set_active_bot(q(&name));
        self.as_mut().open_bot_chat();
    }

    /// Shows `text` as the active bot's last message in the sidebar.
    fn set_bot_preview(mut self: Pin<&mut Self>, text: &str) {
        let active = self.active_bot().to_string();
        let Ok(Value::Array(mut bots)) = serde_json::from_str::<Value>(&self.bots().to_string()) else { return };
        let preview: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
        for bot in bots.iter_mut().filter(|b| b["name"] == active.as_str()) {
            bot["preview"] = json!(preview);
        }
        self.as_mut().set_bots(q(Value::Array(bots).to_string()));
    }

    fn clear_chat(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().live_session.clear();
        self.as_mut().set_active_session(q(""));
        self.as_mut().transcript_loaded(q("[]"));
    }

    fn apply_opened(mut self: Pin<&mut Self>, opened: Opened) {
        self.as_mut().rust_mut().live_session = opened.live;
        self.as_mut().set_active_session(q(&opened.stored));
        if !opened.messages.is_empty() {
            self.as_mut().transcript_loaded(q(transcript_for_view(&opened.messages).to_string()));
        }
        if opened.running {
            self.as_mut().set_streaming(true);
        }
    }

    /// Opens the active bot's Bot Chat. A bot that has none yet shows an empty
    /// chat; the Bot Chat is created when the first message is sent.
    fn open_bot_chat(mut self: Pin<&mut Self>) {
        if self.rust().streaming {
            return;
        }
        self.as_mut().clear_chat();
        self.as_mut().set_jobs(q("[]"));
        self.as_mut().set_jobs_error(q(""));
        self.as_mut().refresh_jobs();
        let Some(gateway) = self.rust().gateway.clone() else { return };
        let profile = self.active_bot().to_string();
        let roster = self.rust().roster_chats.get(&profile).cloned();
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = async {
                match botchat::lookup(&gateway, &profile, roster.as_deref()).await? {
                    Some(stored) => botchat::resume(&gateway, &profile, &stored).await.map(Some),
                    None => Ok(None),
                }
            }
            .await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                match result {
                    Ok(Some(opened)) => app.as_mut().apply_opened(opened),
                    Ok(None) => {}
                    Err(e) => app.as_mut().set_error(q(e)),
                }
            });
        });
    }

    pub fn send(mut self: Pin<&mut Self>, text: QString) {
        let text = text.to_string();
        if text.trim().is_empty() || self.rust().streaming {
            return;
        }
        let Some(gateway) = self.rust().gateway.clone() else {
            self.as_mut().set_error(q("Not connected."));
            return;
        };
        self.as_mut().set_error(q(""));
        self.as_mut().user_message(q(&text));
        self.as_mut().set_streaming(true);
        let profile = self.active_bot().to_string();
        let roster = self.rust().roster_chats.get(&profile).cloned();
        let live = self.rust().live_session.clone();
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result: Result<(), String> = async {
                let live = if live.is_empty() {
                    let opened = botchat::open_or_create(&gateway, &profile, roster.as_deref()).await?;
                    let live = opened.live.clone();
                    // Queued before the prompt goes out, so the Qt thread knows the
                    // session id before any of its events arrive.
                    let echo = text.clone();
                    let _ = qt.queue(move |mut app| {
                        if app.rust().generation != generation {
                            return;
                        }
                        // The chat already existed (made elsewhere meanwhile): show
                        // its history, then this message again below it.
                        let had_history = !opened.messages.is_empty();
                        app.as_mut().apply_opened(opened);
                        if had_history {
                            app.as_mut().user_message(q(echo));
                        }
                    });
                    live
                } else {
                    live
                };
                gateway
                    .request("prompt.submit", json!({ "session_id": live, "text": text, "profile": profile }))
                    .await?;
                Ok(())
            }
            .await;
            if let Err(e) = result {
                let _ = qt.queue(move |mut app| {
                    if app.rust().generation == generation {
                        app.as_mut().set_streaming(false);
                        app.as_mut().set_error(q(e));
                    }
                });
            }
        });
    }

    pub fn stop(mut self: Pin<&mut Self>) {
        let live = self.rust().live_session.clone();
        if live.is_empty() {
            self.as_mut().set_streaming(false);
            return;
        }
        let profile = self.active_bot().to_string();
        self.call("session.interrupt", json!({ "session_id": live, "profile": profile }), |_, _| {});
    }

    /// Reloads the active bot's scheduled jobs. Failures stay in `jobsError`
    /// so the chat's own error bar isn't taken over by a background poll.
    pub fn refresh_jobs(mut self: Pin<&mut Self>) {
        let Some(gateway) = self.rust().gateway.clone() else { return };
        if self.rust().jobs_loading {
            return;
        }
        let profile = self.active_bot().to_string();
        if profile.is_empty() {
            return;
        }
        self.as_mut().set_jobs_loading(true);
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = gateway
                .request("cron.manage", json!({ "action": "list", "include_disabled": true, "profile": profile }))
                .await;
            let _ = qt.queue(move |mut app| {
                app.as_mut().set_jobs_loading(false);
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                match result {
                    Ok(reply) => match reply.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                        Some(e) => app.as_mut().set_jobs_error(q(e)),
                        None => {
                            app.as_mut().set_jobs_error(q(""));
                            app.as_mut().set_jobs(q(Value::Array(cron::jobs_for_bot(&reply, &profile)).to_string()));
                        }
                    },
                    Err(e) => app.as_mut().set_jobs_error(q(e)),
                }
            });
        });
    }

    pub fn answer_request(self: Pin<&mut Self>, id: QString, result: QString) {
        let (Ok(id), Ok(result)) = (
            serde_json::from_str::<Value>(&id.to_string()),
            serde_json::from_str::<Value>(&result.to_string()),
        ) else {
            return;
        };
        if let Some(gateway) = &self.rust().gateway {
            gateway.respond(id, result);
        }
    }
}

/// Fresh tokens, the WebSocket, and the event pump onto the Qt thread.
async fn open_gateway(
    server: &Server,
    qt: cxx_qt::CxxQtThread<qobject::HermesApp>,
    generation: u64,
) -> Result<Arc<Gateway>, String> {
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
            if qt.queue(move |app| app.handle_incoming(generation, incoming)).is_err() || closed {
                break;
            }
        }
    });
    Ok(gateway)
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

impl cxx_qt::Initialize for qobject::HermesApp {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_bots(q("[]"));
        self.as_mut().set_jobs(q("[]"));
        match store::load(&store::config_path()) {
            Ok(config) => self.as_mut().rust_mut().config = config,
            Err(e) => self.as_mut().set_error(q(e)),
        }
        self.as_mut().publish_servers();
        self.connect_active();
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
        assert_eq!(
            rows,
            json!([
                {"role":"user","text":"hi","reasoning":""},
                {"role":"assistant","text":"hello","reasoning":"think"},
                {"role":"tool","text":"web_search","reasoning":""},
            ])
        );
    }
}
