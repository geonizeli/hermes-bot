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
        /// True while connecting or opening a bot's chat; the view shows a spinner.
        #[qproperty(bool, chat_loading, cxx_name = "chatLoading")]
        /// JSON array of the active bot's scheduled jobs.
        #[qproperty(QString, jobs)]
        #[qproperty(bool, jobs_loading, cxx_name = "jobsLoading")]
        #[qproperty(QString, jobs_error, cxx_name = "jobsError")]
        /// JSON `SessionLiveInfo` of the open Bot Chat (model, provider,
        /// reasoning_effort, fast…), "" before it exists.
        #[qproperty(QString, session_info, cxx_name = "sessionInfo")]
        /// JSON array of providers with their models for the active bot.
        #[qproperty(QString, models)]
        /// JSON array of files waiting to go with the next message.
        #[qproperty(QString, attachments)]
        #[qproperty(bool, models_loading, cxx_name = "modelsLoading")]
        /// JSON `DisplayStatus` of the active bot's screen ("" while unknown;
        /// `{"unavailable": true}` when the server has no screens).
        #[qproperty(QString, screen)]
        #[qproperty(bool, screen_busy, cxx_name = "screenBusy")]
        /// Output of a running screen install.
        #[qproperty(QString, screen_log, cxx_name = "screenLog")]
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
        fn refresh_models(self: Pin<&mut HermesApp>);
        /// Display blocks (JSON) for a bot answer; `colors` is JSON with
        /// foreground, muted, accent, surface and border.
        #[qinvokable]
        fn render_markdown(self: Pin<&mut HermesApp>, text: QString, colors: QString) -> QString;
        #[qinvokable]
        fn pick_files(self: Pin<&mut HermesApp>);
        /// JSON array of file:// URLs or paths (drops).
        #[qinvokable]
        fn add_files(self: Pin<&mut HermesApp>, urls: QString);
        #[qinvokable]
        fn remove_attachment(self: Pin<&mut HermesApp>, id: i32);
        /// Takes images and long texts from the clipboard as attachments;
        /// false means ordinary text the text box should paste itself.
        #[qinvokable]
        fn paste_clipboard(self: Pin<&mut HermesApp>) -> bool;
        /// Fetches a server-side file (an image) into the local cache.
        #[qinvokable]
        fn request_media(self: Pin<&mut HermesApp>, path: QString);
        /// `confirmed` resends after the user accepted an expensive model.
        #[qinvokable]
        fn set_model(self: Pin<&mut HermesApp>, model: QString, provider: QString, confirmed: bool);
        #[qinvokable]
        fn set_effort(self: Pin<&mut HermesApp>, level: QString);
        #[qinvokable]
        fn set_fast(self: Pin<&mut HermesApp>, on: bool);
        #[qinvokable]
        fn refresh_screen(self: Pin<&mut HermesApp>);
        #[qinvokable]
        fn start_screen(self: Pin<&mut HermesApp>);
        #[qinvokable]
        fn stop_screen(self: Pin<&mut HermesApp>, force: bool);
        #[qinvokable]
        fn install_screen(self: Pin<&mut HermesApp>);
        #[qinvokable]
        fn answer_request(self: Pin<&mut HermesApp>, id: QString, result: QString);

        /// JSON `ServerInfo`, or an empty string with `error` set.
        #[qsignal]
        fn server_probed(self: Pin<&mut HermesApp>, info: QString);
        /// The server wants confirmation before switching to this model.
        #[qsignal]
        fn model_confirm(self: Pin<&mut HermesApp>, message: QString, model: QString, provider: QString);
        #[qsignal]
        fn sign_in_finished(self: Pin<&mut HermesApp>, ok: bool, message: QString);
        /// JSON array of `{role, text}`; replaces the conversation.
        #[qsignal]
        fn transcript_loaded(self: Pin<&mut HermesApp>, messages: QString);
        /// `attachments` is a JSON array of `{kind, name, url}`.
        #[qsignal]
        fn user_message(self: Pin<&mut HermesApp>, text: QString, attachments: QString);
        #[qsignal]
        fn media_ready(self: Pin<&mut HermesApp>, path: QString, url: QString);
        #[qsignal]
        fn assistant_started(self: Pin<&mut HermesApp>);
        #[qsignal]
        fn assistant_delta(self: Pin<&mut HermesApp>, text: QString);
        #[qsignal]
        fn reasoning_delta(self: Pin<&mut HermesApp>, text: QString);
        /// A tool row's fields (JSON): toolId, name, detail, args.
        #[qsignal]
        fn tool_started(self: Pin<&mut HermesApp>, tool: QString);
        /// toolId, name, result, diff, todos, duration, error (JSON).
        #[qsignal]
        fn tool_finished(self: Pin<&mut HermesApp>, tool: QString);
        /// Commentary the bot wrote before calling tools; it ends a section.
        #[qsignal]
        fn assistant_interim(self: Pin<&mut HermesApp>, text: QString, already_streamed: bool);
        /// The bot's to-do list (JSON array of {content, status}).
        #[qsignal]
        fn todos_updated(self: Pin<&mut HermesApp>, todos: QString);
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
use crate::backend::{attachments, history, markdown, media};
use crate::backend::gateway::{Gateway, Incoming, UNAUTHORIZED};
use crate::backend::{runtime, session};
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
    chat_loading: bool,
    jobs: QString,
    jobs_loading: bool,
    jobs_error: QString,
    session_info: QString,
    models: QString,
    attachments: QString,
    models_loading: bool,
    screen: QString,
    screen_busy: bool,
    screen_log: QString,

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
    pending: Vec<attachments::Attachment>,
    next_attachment: u64,
    /// Server path → local file URL.
    media_cache: HashMap<String, String>,
    /// Rendered answers by (text, colours) hash; answers re-render on every
    /// streamed delta, finished ones shouldn't.
    markdown_cache: HashMap<u64, String>,
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

/// The chat socket's transcript (no tool results), as conversation rows;
/// used when the REST history isn't available.
pub fn transcript_for_view(messages: &[Value]) -> Value {
    let rows: Vec<Value> = messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
            let mut r = history::row(role);
            r["timestamp"] = m.get("timestamp").cloned().unwrap_or(json!(0.0));
            match role {
                "user" => {
                    let (text, refs) = attachments::split_refs(&message_text(m));
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
        session::set(None);
        rust.live_session.clear();
        rust.roster_chats.clear();
        self.as_mut().set_connected(false);
        self.as_mut().set_streaming(false);
        self.as_mut().set_chat_loading(false);
        self.as_mut().set_bots(q("[]"));
        self.as_mut().set_jobs(q("[]"));
        self.as_mut().set_jobs_error(q(""));
        self.as_mut().set_screen(q(""));
        self.as_mut().set_screen_log(q(""));
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
        self.as_mut().set_chat_loading(true);
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = open_gateway(&server, qt.clone(), generation).await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation {
                    return;
                }
                match result {
                    Ok((gateway, access_token)) => {
                        session::set(Some(session::Session {
                            gateway: gateway.clone(),
                            base_url: server.url.clone(),
                            access_token,
                        }));
                        app.as_mut().rust_mut().gateway = Some(gateway);
                        app.as_mut().set_connected(true);
                        app.as_mut().set_error(q(""));
                        app.as_mut().set_status_text(&format!("Connected to {}", server.name));
                        app.as_mut().load_bots();
                    }
                    Err(e) => {
                        app.as_mut().set_chat_loading(false);
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
                        app.as_mut().set_chat_loading(false);
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
            if bots.is_empty() {
                app.as_mut().set_chat_loading(false);
            }
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
                if kind.starts_with("display.") {
                    self.as_mut().handle_display_event(&kind, &payload);
                    return;
                }
                match kind.as_str() {
                    _ if !ours => {}
                    "session.info" => {
                        if payload.is_object() {
                            self.as_mut().set_session_info(q(payload.to_string()));
                        }
                    }
                    "message.start" => self.as_mut().assistant_started(),
                    "message.delta" => {
                        let delta = text("text");
                        if !delta.is_empty() {
                            self.as_mut().assistant_delta(q(delta));
                        }
                    }
                    "reasoning.delta" | "thinking.delta" => self.as_mut().reasoning_delta(q(text("text"))),
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
                        let tool = json!({ "toolId": text("tool_id"), "name": text("name"), "detail": detail, "args": args });
                        self.as_mut().tool_started(q(tool.to_string()));
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
                        let tool = json!({
                            "toolId": text("tool_id"),
                            "name": text("name"),
                            "summary": text("summary"),
                            "result": result,
                            "diff": diff,
                            "todos": payload.get("todos").filter(|t| t.is_array()).map(|t| t.to_string()).unwrap_or_else(|| "[]".into()),
                            "duration": payload.get("duration_s").and_then(Value::as_f64).unwrap_or(0.0),
                            "error": error,
                        });
                        self.as_mut().tool_finished(q(tool.to_string()));
                    }
                    "message.interim" => {
                        let streamed = payload.get("already_streamed").and_then(Value::as_bool).unwrap_or(false);
                        self.as_mut().assistant_interim(q(text("text")), streamed);
                    }
                    "todo.updated" => {
                        if let Some(todos) = payload.get("todos").filter(|t| t.is_array()) {
                            self.as_mut().todos_updated(q(todos.to_string()));
                        }
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
                session::set(None);
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

    fn apply_opened(mut self: Pin<&mut Self>, opened: Opened, view: Option<Vec<Value>>) {
        if opened.info.is_object() {
            self.as_mut().set_session_info(q(opened.info.to_string()));
        }
        self.as_mut().rust_mut().live_session = opened.live;
        self.as_mut().set_active_session(q(&opened.stored));
        match view {
            Some(rows) if !rows.is_empty() => self.as_mut().transcript_loaded(q(Value::Array(rows).to_string())),
            _ if !opened.messages.is_empty() => {
                self.as_mut().transcript_loaded(q(transcript_for_view(&opened.messages).to_string()))
            }
            _ => {}
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
        self.as_mut().set_screen(q(""));
        self.as_mut().set_screen_log(q(""));
        self.as_mut().refresh_screen();
        self.as_mut().set_session_info(q(""));
        self.as_mut().set_models(q("[]"));
        self.as_mut().refresh_models();
        self.as_mut().rust_mut().pending.clear();
        self.as_mut().publish_attachments();
        let Some(gateway) = self.rust().gateway.clone() else {
            self.as_mut().set_chat_loading(false);
            return;
        };
        self.as_mut().set_chat_loading(true);
        let profile = self.active_bot().to_string();
        let roster = self.rust().roster_chats.get(&profile).cloned();
        let generation = self.rust().generation;
        let qt = self.qt_thread();
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
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                app.as_mut().set_chat_loading(false);
                match result {
                    Ok(Some((opened, view))) => app.as_mut().apply_opened(opened, view),
                    Ok(None) => {}
                    Err(e) => app.as_mut().set_error(q(e)),
                }
            });
        });
    }

    pub fn send(mut self: Pin<&mut Self>, text: QString) {
        let text = text.to_string();
        if (text.trim().is_empty() && self.rust().pending.is_empty()) || self.rust().streaming {
            return;
        }
        let Some(gateway) = self.rust().gateway.clone() else {
            self.as_mut().set_error(q("Not connected."));
            return;
        };
        self.as_mut().set_error(q(""));
        let pending = std::mem::take(&mut self.as_mut().rust_mut().pending);
        self.as_mut().publish_attachments();
        let view: Vec<Value> = pending.iter().map(|a| a.to_view()).collect();
        let echo_attachments = Value::Array(view).to_string();
        self.as_mut().user_message(q(&text), q(&echo_attachments));
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
                        app.as_mut().apply_opened(opened, None);
                        if had_history {
                            app.as_mut().user_message(q(echo), q(echo_attachments));
                        }
                    });
                    live
                } else {
                    live
                };
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
                gateway
                    .request("prompt.submit", json!({ "session_id": live, "text": prompt, "profile": profile }))
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

    pub fn render_markdown(mut self: Pin<&mut Self>, text: QString, colors: QString) -> QString {
        use std::hash::{Hash, Hasher};
        let (text, colors) = (text.to_string(), colors.to_string());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&text, &colors).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(hit) = self.rust().markdown_cache.get(&key) {
            return q(hit);
        }
        let palette: markdown::Colors = serde_json::from_str(&colors).unwrap_or_default();
        let rendered = markdown::render(&text, &palette).to_string();
        let cache = &mut self.as_mut().rust_mut().markdown_cache;
        if cache.len() > 400 {
            cache.clear();
        }
        cache.insert(key, rendered.clone());
        q(rendered)
    }

    fn publish_attachments(mut self: Pin<&mut Self>) {
        let view: Vec<Value> = self.rust().pending.iter().map(|a| a.to_view()).collect();
        self.as_mut().set_attachments(q(Value::Array(view).to_string()));
    }

    fn add_paths(mut self: Pin<&mut Self>, paths: Vec<std::path::PathBuf>) {
        let mut problems = Vec::new();
        for path in paths {
            let id = {
                let mut rust = self.as_mut().rust_mut();
                rust.next_attachment += 1;
                rust.next_attachment
            };
            match attachments::from_path(id, &path) {
                Ok(attachment) => self.as_mut().rust_mut().pending.push(attachment),
                Err(e) => problems.push(e),
            }
        }
        if !problems.is_empty() {
            self.as_mut().set_error(q(problems.join(" ")));
        }
        self.as_mut().publish_attachments();
    }

    pub fn pick_files(self: Pin<&mut Self>) {
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = attachments::pick_files().await;
            let _ = qt.queue(move |mut app| match result {
                Ok(paths) => app.as_mut().add_paths(paths),
                Err(e) => app.as_mut().set_error(q(e)),
            });
        });
    }

    pub fn add_files(self: Pin<&mut Self>, urls: QString) {
        let list: Vec<String> = serde_json::from_str(&urls.to_string()).unwrap_or_default();
        let paths = list.iter().filter_map(|u| attachments::path_from_url(u)).collect();
        self.add_paths(paths);
    }

    pub fn remove_attachment(mut self: Pin<&mut Self>, id: i32) {
        self.as_mut().rust_mut().pending.retain(|a| a.id != id as u64);
        self.publish_attachments();
    }

    pub fn paste_clipboard(mut self: Pin<&mut Self>) -> bool {
        match attachments::paste_clipboard(&attachments::scratch_dir()) {
            Ok(attachments::Pasted::File(path)) => {
                self.add_paths(vec![path]);
                true
            }
            Ok(attachments::Pasted::Text) | Ok(attachments::Pasted::Nothing) => false,
            Err(e) => {
                self.as_mut().set_error(q(e));
                false
            }
        }
    }

    pub fn request_media(mut self: Pin<&mut Self>, path: QString) {
        let path = path.to_string();
        if let Some(url) = self.rust().media_cache.get(&path).cloned() {
            self.as_mut().media_ready(q(&path), q(url));
            return;
        }
        let Some(session) = session::current() else { return };
        let profile = self.active_bot().to_string();
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = media::fetch(&session, &path, &profile).await;
            let _ = qt.queue(move |mut app| {
                if let Ok(file) = result {
                    let url = url::Url::from_file_path(&file).map(|u| u.to_string()).unwrap_or_default();
                    app.as_mut().rust_mut().media_cache.insert(path.clone(), url.clone());
                    app.as_mut().media_ready(q(path), q(url));
                }
            });
        });
    }

    /// Reduces `model.options` providers to what the picker needs.
    fn models_for_view(result: &Value) -> Value {
        let providers = result.get("providers").and_then(Value::as_array).cloned().unwrap_or_default();
        Value::Array(
            providers
                .iter()
                .filter(|p| p.get("models").and_then(Value::as_array).is_some_and(|m| !m.is_empty()))
                .map(|p| {
                    let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                    json!({
                        "slug": text("slug"),
                        "name": Some(text("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| text("slug")),
                        "models": p.get("models").cloned().unwrap_or(json!([])),
                        "current": p.get("is_current").and_then(Value::as_bool).unwrap_or(false),
                        "authenticated": p.get("authenticated").and_then(Value::as_bool).unwrap_or(true),
                        "warning": text("warning"),
                        "capabilities": p.get("capabilities").cloned().unwrap_or(json!({})),
                    })
                })
                .collect(),
        )
    }

    pub fn refresh_models(mut self: Pin<&mut Self>) {
        let Some(gateway) = self.rust().gateway.clone() else { return };
        let profile = self.active_bot().to_string();
        if profile.is_empty() {
            return;
        }
        let live = self.rust().live_session.clone();
        self.as_mut().set_models_loading(true);
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let mut params = json!({ "profile": profile, "explicit_only": true });
            if !live.is_empty() {
                params["session_id"] = json!(live);
            }
            let result = gateway.request("model.options", params).await;
            let _ = qt.queue(move |mut app| {
                app.as_mut().set_models_loading(false);
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                match result {
                    Ok(reply) => app.as_mut().set_models(q(Self::models_for_view(&reply).to_string())),
                    Err(e) => app.as_mut().set_error(q(format!("Couldn't load models: {e}"))),
                }
            });
        });
    }

    /// Runs a session-scoped `config.set` on the Bot Chat, opening (or
    /// creating) it first: without a session the server would write the
    /// bot's config.yaml instead.
    fn session_config(mut self: Pin<&mut Self>, key: &'static str, value: String, extra: Value, model: Option<(String, String)>) {
        let Some(gateway) = self.rust().gateway.clone() else {
            self.as_mut().set_error(q("Not connected."));
            return;
        };
        let profile = self.active_bot().to_string();
        let roster = self.rust().roster_chats.get(&profile).cloned();
        let live = self.rust().live_session.clone();
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result: Result<Value, String> = async {
                let live = if live.is_empty() {
                    let opened = botchat::open_or_create(&gateway, &profile, roster.as_deref()).await?;
                    let id = opened.live.clone();
                    let _ = qt.queue(move |mut app| {
                        if app.rust().generation == generation {
                            app.as_mut().apply_opened(opened, None);
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
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation {
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
                                app.as_mut().model_confirm(q(message), q(model), q(provider));
                            }
                            return;
                        }
                        if let Some(info) = reply.get("info").filter(|i| i.is_object()) {
                            app.as_mut().set_session_info(q(info.to_string()));
                        }
                        if let Some(warning) = reply.get("warning").and_then(Value::as_str).filter(|w| !w.is_empty()) {
                            app.as_mut().set_error(q(warning));
                        }
                    }
                    Err(e) => app.as_mut().set_error(q(e)),
                }
            });
        });
    }

    pub fn set_model(self: Pin<&mut Self>, model: QString, provider: QString, confirmed: bool) {
        let (model, provider) = (model.to_string(), provider.to_string());
        // --session keeps the pick on this Bot Chat; the bot's profile is untouched.
        let mut value = model.clone();
        if !provider.is_empty() {
            value.push_str(&format!(" --provider {provider}"));
        }
        value.push_str(" --session");
        let extra = if confirmed { json!({ "confirm_expensive_model": true }) } else { json!({}) };
        self.session_config("model", value, extra, Some((model, provider)));
    }

    pub fn set_effort(self: Pin<&mut Self>, level: QString) {
        self.session_config("reasoning", level.to_string(), json!({ "scope": "session" }), None);
    }

    pub fn set_fast(self: Pin<&mut Self>, on: bool) {
        self.session_config("fast", if on { "fast" } else { "normal" }.to_string(), json!({}), None);
    }

    /// The active bot's screen identity in display events.
    fn screen_key(&self) -> String {
        serde_json::from_str::<Value>(&self.screen().to_string())
            .ok()
            .and_then(|s| s.get("profile_key").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default()
    }

    fn handle_display_event(mut self: Pin<&mut Self>, kind: &str, payload: &Value) {
        let key = payload.get("profile_key").and_then(Value::as_str).unwrap_or_default();
        let profile = payload.get("profile").and_then(Value::as_str).unwrap_or_default();
        let ours = (!key.is_empty() && key == self.screen_key())
            || (!profile.is_empty() && profile == self.active_bot().to_string());
        if !ours {
            return;
        }
        match kind {
            "display.status" => self.as_mut().set_screen(q(payload.to_string())),
            "display.lease" => {
                if let Ok(mut status) = serde_json::from_str::<Value>(&self.screen().to_string()) {
                    status["lease"] = payload.get("lease").cloned().unwrap_or(Value::Null);
                    self.as_mut().set_screen(q(status.to_string()));
                }
            }
            "display.install.log" => {
                let line = payload.get("line").and_then(Value::as_str).unwrap_or_default();
                let mut log = self.screen_log().to_string();
                log.push_str(line);
                log.push('\n');
                // Keep the tail; installs can be chatty.
                if log.len() > 20_000 {
                    log = log[log.len() - 20_000..].to_string();
                }
                self.as_mut().set_screen_log(q(log));
            }
            "display.install.done" => {
                self.as_mut().set_screen_busy(false);
                if let Some(status) = payload.get("status").filter(|s| s.is_object()) {
                    self.as_mut().set_screen(q(status.to_string()));
                }
                let code = payload.get("code").and_then(Value::as_i64).unwrap_or(1);
                let note = match code {
                    0 => "Installed.",
                    -1 => "Install cancelled.",
                    -2 => "No sudo access: run the command above on the server.",
                    _ => "Install failed; see the log above.",
                };
                let log = format!("{}{note}\n", self.screen_log());
                self.as_mut().set_screen_log(q(log));
            }
            _ => {}
        }
    }

    /// Runs a display RPC for the active bot and stores the status it returns.
    fn display_call(mut self: Pin<&mut Self>, method: &'static str, extra: Value) {
        let Some(gateway) = self.rust().gateway.clone() else { return };
        let profile = self.active_bot().to_string();
        if profile.is_empty() {
            return;
        }
        let mut params = json!({ "profile": profile });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        self.as_mut().set_screen_busy(true);
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = gateway.request(method, params).await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                match result {
                    Ok(status) => {
                        // display.install answers {started}; its status arrives with install.done.
                        if method == "display.install" {
                            return;
                        }
                        app.as_mut().set_screen_busy(false);
                        app.as_mut().set_screen(q(status.to_string()));
                    }
                    Err(e) => {
                        app.as_mut().set_screen_busy(false);
                        if e.to_lowercase().contains("method not found") || e.contains("Unknown method") {
                            app.as_mut().set_screen(q(r#"{"unavailable":true}"#));
                        } else if method == "display.status" {
                            app.as_mut().set_screen(q(json!({ "error": e }).to_string()));
                        } else {
                            app.as_mut().set_error(q(e));
                        }
                    }
                }
            });
        });
    }

    pub fn refresh_screen(self: Pin<&mut Self>) {
        self.display_call("display.status", json!({}));
    }

    pub fn start_screen(self: Pin<&mut Self>) {
        self.display_call("display.start", json!({}));
    }

    pub fn stop_screen(self: Pin<&mut Self>, force: bool) {
        self.display_call("display.stop", json!({ "force": force }));
    }

    pub fn install_screen(mut self: Pin<&mut Self>) {
        self.as_mut().set_screen_log(q(""));
        self.display_call("display.install", json!({}));
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
) -> Result<(Arc<Gateway>, String), String> {
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

impl cxx_qt::Initialize for qobject::HermesApp {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_bots(q("[]"));
        self.as_mut().set_jobs(q("[]"));
        self.as_mut().set_models(q("[]"));
        self.as_mut().set_attachments(q("[]"));
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
        let rows = rows.as_array().unwrap();
        let roles: Vec<_> = rows.iter().map(|r| r["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool"]);
        assert_eq!(rows[1]["text"], "hello");
        assert_eq!(rows[1]["reasoning"], "think");
        assert_eq!(rows[2]["name"], "web_search");
    }
}
