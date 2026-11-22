//! Slash commands on `HermesApp`: the command menu and running what's typed.
//! What a command means comes from the server (`backend::slash`); this file
//! only connects it to the open chat.

use core::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use serde_json::{Value, json};

use super::{Directive, Local, Route, q, qobject, slash};
use crate::backend::{botchat, runtime};

/// Menu rows shown at once.
const MENU_ROWS: usize = 40;

/// Commands after which the command set may have changed.
const RELOADS_COMMANDS: &[&str] = &["/reload-skills", "/skills", "/learn", "/bundles", "/reload"];

impl qobject::HermesApp {
    pub fn slash_suggestions(&self, text: QString) -> QString {
        let profile = self.active_bot().to_string();
        let rows = self
            .rust()
            .catalogs
            .get(&profile)
            .map(|c| c.suggestions(&text.to_string(), MENU_ROWS))
            .unwrap_or_default();
        q(Value::Array(rows).to_string())
    }

    /// Loads a bot's command set; `live` adds the chat's workspace skills.
    pub(super) fn refresh_commands(self: Pin<&mut Self>, profile: &str, live: &str) {
        let mut params = json!({ "profile": profile });
        if !live.is_empty() {
            params["session_id"] = json!(live);
        }
        let Some(gateway) = self.rust().gateway.clone() else { return };
        let generation = self.rust().generation;
        let profile = profile.to_string();
        let qt = self.qt_thread();
        runtime().spawn(async move {
            // A missing menu is no reason to bother the user: commands still
            // run when typed, and the next open tries again.
            let Ok(result) = gateway.request("commands.catalog", params).await else { return };
            let catalog = slash::Catalog::from_json(&result);
            let _ = qt.queue(move |mut app| {
                if app.rust().generation == generation {
                    app.as_mut().rust_mut().catalogs.insert(profile, catalog);
                }
            });
        });
    }

    pub(super) fn run_slash(mut self: Pin<&mut Self>, command: slash::Parsed) {
        let profile = self.active_bot().to_string();
        let route = self.rust().catalogs.get(&profile).map(|c| c.route(&command.name));
        let route = route.unwrap_or_else(|| slash::Catalog::default().route(&command.name));
        let typed = format!("/{}", command.name);
        let arg = command.arg.clone();
        match route {
            Route::Unavailable(text) => self.as_mut().say(&typed, &text),
            Route::Local(Local::Help) => {
                let help = self.rust().catalogs.get(&profile).map(|c| c.help()).unwrap_or_else(|| slash::Catalog::default().help());
                self.as_mut().say(&typed, &help);
            }
            Route::Local(Local::Stop) => {
                if self.rust().streaming {
                    self.as_mut().stop();
                } else {
                    self.as_mut().say(&typed, "The bot isn't answering anything right now.");
                }
            }
            Route::Local(Local::Steer) if self.rust().streaming => {
                if arg.is_empty() {
                    self.as_mut().say(&typed, "Usage: /steer <guidance>");
                } else {
                    self.as_mut().steer(q(&arg));
                }
            }
            Route::Local(Local::Model) => self.as_mut().open_model_picker(),
            Route::Local(Local::Reasoning) if slash::EFFORTS.contains(&arg.to_lowercase().as_str()) => {
                let level = arg.to_lowercase();
                self.as_mut().set_effort(q(&level));
                self.as_mut().say(&typed, &format!("Reasoning effort: {level}."));
            }
            Route::Local(Local::Compress) if !arg.starts_with("here") && !arg.starts_with("--") => {
                // A focus topic only means something to /compress itself;
                // `/new name` names a fresh session, which a Bot Chat isn't.
                let canonical = self.rust().catalogs.get(&profile).map(|c| c.canonical(&command.name));
                let focus = (canonical.as_deref().unwrap_or(&typed) == "/compress").then_some(arg).filter(|a| !a.is_empty());
                self.compress(typed, focus);
            }
            _ => self.run_on_server(typed, command),
        }
    }

    /// Shows a command and what it printed.
    fn say(mut self: Pin<&mut Self>, command: &str, text: &str) {
        self.as_mut().command_started(q(command));
        self.as_mut().command_output(q(command), q(text));
    }

    fn compress(mut self: Pin<&mut Self>, typed: String, focus: Option<String>) {
        let live = self.rust().live_session.clone();
        if live.is_empty() {
            self.as_mut().say(&typed, "This chat is empty; there's nothing to compress.");
            return;
        }
        if self.rust().streaming {
            self.as_mut().say(&typed, "The bot is answering. Stop it or wait, then try again.");
            return;
        }
        let Some(gateway) = self.rust().gateway.clone() else { return };
        self.as_mut().command_started(q(&typed));
        let profile = self.active_bot().to_string();
        let mut params = json!({ "session_id": live, "profile": profile });
        if let Some(focus) = focus {
            params["focus_topic"] = json!(focus);
        }
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            // Summarizing a long chat is a model call; give it time.
            let result = gateway.request_within("session.compress", params, Duration::from_secs(600)).await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                let text = match result {
                    Ok(reply) => {
                        if let Some(usage) = reply.get("usage").filter(|u| u.is_object()) {
                            app.as_mut().apply_usage(usage);
                        }
                        slash::compress_summary(&reply)
                    }
                    Err(e) => format!("error: {e}"),
                };
                app.as_mut().command_output(q(&typed), q(text));
            });
        });
    }

    /// Runs a command on the server, opening (or creating) the Bot Chat
    /// first, since commands run against a session.
    fn run_on_server(mut self: Pin<&mut Self>, typed: String, command: slash::Parsed) {
        let Some(gateway) = self.rust().gateway.clone() else {
            self.as_mut().set_error(q("Not connected."));
            return;
        };
        self.as_mut().command_started(q(&typed));
        let profile = self.active_bot().to_string();
        let roster = self.rust().roster_chats.get(&profile).cloned();
        let live = self.rust().live_session.clone();
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result: Result<Directive, String> = async {
                let live = if live.is_empty() {
                    let opened = botchat::open_or_create(&gateway, &profile, roster.as_deref()).await?;
                    let live = opened.live.clone();
                    let profile = profile.clone();
                    let _ = qt.queue(move |mut app| {
                        if app.rust().generation == generation && app.active_bot().to_string() == profile {
                            app.as_mut().apply_opened(opened, None);
                        }
                    });
                    live
                } else {
                    live
                };
                slash::run(&gateway, &live, &profile, &command.name, &command.arg).await
            }
            .await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation || app.active_bot().to_string() != profile {
                    return;
                }
                app.as_mut().apply_directive(&typed, result);
                let canonical = app.rust().catalogs.get(&profile).map(|c| c.canonical(&command.name));
                if RELOADS_COMMANDS.contains(&canonical.as_deref().unwrap_or(&typed)) {
                    let live = app.rust().live_session.clone();
                    app.as_mut().refresh_commands(&profile, &live);
                }
            });
        });
    }

    fn apply_directive(mut self: Pin<&mut Self>, typed: &str, result: Result<Directive, String>) {
        match result {
            Err(e) => self.as_mut().command_output(q(typed), q(format!("error: {e}"))),
            Ok(Directive::Output(text)) => self.as_mut().command_output(q(typed), q(text)),
            Ok(Directive::Alias(target)) => self.as_mut().command_output(q(typed), q(format!("→ {target}"))),
            Ok(Directive::Prefill { message, notice }) => {
                let text = if notice.is_empty() { "Your draft is back in the message box." } else { &notice };
                self.as_mut().command_output(q(typed), q(text));
                self.as_mut().prefill(q(message));
            }
            Ok(Directive::Send { message, display, notice }) => {
                if message.trim().is_empty() {
                    self.as_mut().command_output(q(typed), q(format!("{typed}: the server sent nothing to say.")));
                    return;
                }
                if self.rust().streaming {
                    let text = "The bot is answering. Stop it or wait, then send this again.";
                    self.as_mut().command_output(q(typed), q(text));
                    self.as_mut().prefill(q(display));
                    return;
                }
                // The command's row stays only when it has something to say.
                self.as_mut().command_output(q(typed), q(notice));
                self.as_mut().submit(message, display);
            }
        }
    }
}
