//! Slash commands: the command menu and running what's typed. What a
//! command means comes from the server (`backend::slash`); this file only
//! connects it to the open chat.

use std::time::Duration;

use gpui_kit::Context;
use serde_json::{Value, json};

use super::{AppEvent, AppState};
use crate::backend::theme::ThemeChoice;
use crate::backend::slash::{self, Directive, Local, Route};
use crate::backend::{botchat, runtime};

/// Menu rows shown at once.
const MENU_ROWS: usize = 40;

/// Commands after which the command set may have changed.
const RELOADS_COMMANDS: &[&str] = &["/reload-skills", "/skills", "/learn", "/bundles", "/reload"];

/// A row of the command menu.
#[derive(Clone, Debug)]
pub struct Suggestion {
    /// What accepting it puts in the message box.
    pub insert: String,
    pub display: String,
    pub meta: String,
    pub skill: bool,
}

impl AppState {
    /// Command menu rows for the composer's text, from the open chat's command set.
    pub fn slash_suggestions(&self, text: &str) -> Vec<Suggestion> {
        self.catalogs
            .get(&self.active_bot)
            .map(|c| c.suggestions(text, MENU_ROWS))
            .unwrap_or_default()
            .iter()
            .map(|v| {
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                Suggestion { insert: s("insert"), display: s("display"), meta: s("meta"), skill: v["skill"] == true }
            })
            .collect()
    }

    /// Loads a bot's command set; `live` adds the chat's workspace skills.
    pub(super) fn refresh_commands(&mut self, profile: &str, live: &str) {
        let mut params = json!({ "profile": profile });
        if !live.is_empty() {
            params["session_id"] = json!(live);
        }
        let Some(gateway) = self.gateway.clone() else { return };
        let generation = self.generation;
        let profile = profile.to_string();
        let queue = self.queue.clone();
        runtime().spawn(async move {
            // A missing menu is no reason to bother the user: commands still
            // run when typed, and the next open tries again.
            let Ok(result) = gateway.request("commands.catalog", params).await else { return };
            let catalog = slash::Catalog::from_json(&result);
            queue.queue(move |app, _| {
                if app.generation == generation {
                    app.catalogs.insert(profile, catalog);
                }
            });
        });
    }

    pub(super) fn run_slash(&mut self, command: slash::Parsed, cx: &mut Context<Self>) {
        let profile = self.active_bot.clone();
        let route = self.catalogs.get(&profile).map(|c| c.route(&command.name));
        let route = route.unwrap_or_else(|| slash::Catalog::default().route(&command.name));
        let typed = format!("/{}", command.name);
        let arg = command.arg.clone();
        match route {
            Route::Unavailable(text) => self.say(&typed, &text),
            Route::Local(Local::Help) => {
                let help = self.catalogs.get(&profile).map(|c| c.help()).unwrap_or_else(|| slash::Catalog::default().help());
                self.say(&typed, &help);
            }
            Route::Local(Local::Stop) => {
                if self.streaming {
                    self.stop(cx);
                } else {
                    self.say(&typed, "The bot isn't answering anything right now.");
                }
            }
            Route::Local(Local::Steer) if self.streaming => {
                if arg.is_empty() {
                    self.say(&typed, "Usage: /steer <guidance>");
                } else {
                    self.steer(&arg, cx);
                }
            }
            Route::Local(Local::Model) => cx.emit(AppEvent::OpenModelPicker),
            Route::Local(Local::Skin) => {
                let theme = super::super::theme::theme_state(cx).clone();
                let names: Vec<&str> = theme.choices().iter().map(|c| c.key()).collect();
                if arg.is_empty() {
                    let current = theme.effective();
                    self.say(&typed, &format!("Theme: {}. Switch with /skin {}.", current.label(), names.join(" | ")));
                } else {
                    match ThemeChoice::parse(&arg).filter(|c| theme.choices().contains(c)) {
                        Some(choice) => {
                            self.set_theme(choice, cx);
                            self.say(&typed, &format!("Theme: {}.", choice.label()));
                        }
                        None => self.say(&typed, &format!("No theme called \"{arg}\". Pick one of: {}.", names.join(", "))),
                    }
                }
            }
            Route::Local(Local::Reasoning) if slash::EFFORTS.contains(&arg.to_lowercase().as_str()) => {
                let level = arg.to_lowercase();
                self.set_effort(&level, cx);
                self.say(&typed, &format!("Reasoning effort: {level}."));
            }
            Route::Local(Local::Compress) if !arg.starts_with("here") && !arg.starts_with("--") => {
                // A focus topic only means something to /compress itself;
                // `/new name` names a fresh session, which a Bot Chat isn't.
                let canonical = self.catalogs.get(&profile).map(|c| c.canonical(&command.name));
                let focus = (canonical.as_deref().unwrap_or(&typed) == "/compress").then_some(arg).filter(|a| !a.is_empty());
                self.compress(typed, focus);
            }
            _ => self.run_on_server(typed, command, cx),
        }
    }

    /// Shows a command and what it printed.
    fn say(&mut self, command: &str, text: &str) {
        self.on_command_started(command);
        self.on_command_output(command, text);
    }

    fn compress(&mut self, typed: String, focus: Option<String>) {
        let live = self.live_session.clone();
        if live.is_empty() {
            self.say(&typed, "This chat is empty; there's nothing to compress.");
            return;
        }
        if self.streaming {
            self.say(&typed, "The bot is answering. Stop it or wait, then try again.");
            return;
        }
        let Some(gateway) = self.gateway.clone() else { return };
        self.on_command_started(&typed);
        let profile = self.active_bot.clone();
        let mut params = json!({ "session_id": live, "profile": profile });
        if let Some(focus) = focus {
            params["focus_topic"] = json!(focus);
        }
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            // Summarizing a long chat is a model call; give it time.
            let result = gateway.request_within("session.compress", params, Duration::from_secs(600)).await;
            queue.queue(move |app, cx| {
                if app.generation != generation || app.active_bot != profile {
                    return;
                }
                let text = match result {
                    Ok(reply) => {
                        if let Some(usage) = reply.get("usage").filter(|u| u.is_object()) {
                            app.apply_usage(usage);
                        }
                        slash::compress_summary(&reply)
                    }
                    Err(e) => format!("error: {e}"),
                };
                app.on_command_output(&typed, &text);
                cx.notify();
            });
        });
    }

    /// Runs a command on the server, opening (or creating) the Bot Chat
    /// first, since commands run against a session.
    fn run_on_server(&mut self, typed: String, command: slash::Parsed, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else {
            self.set_error("Not connected.", cx);
            return;
        };
        self.on_command_started(&typed);
        let profile = self.active_bot.clone();
        let roster = self.roster_chats.get(&profile).cloned();
        let live = self.live_session.clone();
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result: Result<Directive, String> = async {
                let live = if live.is_empty() {
                    let opened = botchat::open_or_create(&gateway, &profile, roster.as_deref()).await?;
                    let live = opened.live.clone();
                    let profile = profile.clone();
                    queue.queue(move |app, cx| {
                        if app.generation == generation && app.active_bot == profile {
                            app.apply_opened(opened, None, cx);
                        }
                    });
                    live
                } else {
                    live
                };
                slash::run(&gateway, &live, &profile, &command.name, &command.arg).await
            }
            .await;
            queue.queue(move |app, cx| {
                if app.generation != generation || app.active_bot != profile {
                    return;
                }
                app.apply_directive(&typed, result, cx);
                let canonical = app.catalogs.get(&profile).map(|c| c.canonical(&command.name));
                if RELOADS_COMMANDS.contains(&canonical.as_deref().unwrap_or(&typed)) {
                    let live = app.live_session.clone();
                    app.refresh_commands(&profile, &live);
                }
                cx.notify();
            });
        });
    }

    fn apply_directive(&mut self, typed: &str, result: Result<Directive, String>, cx: &mut Context<Self>) {
        match result {
            Err(e) => self.on_command_output(typed, &format!("error: {e}")),
            Ok(Directive::Output(text)) => self.on_command_output(typed, &text),
            Ok(Directive::Alias(target)) => self.on_command_output(typed, &format!("→ {target}")),
            Ok(Directive::Prefill { message, notice }) => {
                let text = if notice.is_empty() { "Your draft is back in the message box." } else { &notice };
                self.on_command_output(typed, text);
                cx.emit(AppEvent::Prefill(message));
            }
            Ok(Directive::Send { message, display, notice }) => {
                if message.trim().is_empty() {
                    self.on_command_output(typed, &format!("{typed}: the server sent nothing to say."));
                    return;
                }
                if self.streaming {
                    let text = "The bot is answering. Stop it or wait, then send this again.";
                    self.on_command_output(typed, text);
                    cx.emit(AppEvent::Prefill(display));
                    return;
                }
                // The command's row stays only when it has something to say.
                self.on_command_output(typed, &notice);
                self.submit(message, display, cx);
            }
        }
    }
}
