//! The servers dialog, the bot's mid-turn questions, and the expensive
//! model confirmation.

use std::collections::HashMap;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::state::{AppEvent, AppState};
use super::theme::colors;
use super::widgets::{self, Ident, ident, heading, id, modal, mono, muted};
use crate::backend::auth::ServerInfo;

// ── servers ──

/// Add, sign in to, and remove Hermes servers.
pub struct SetupDialog {
    state: Entity<AppState>,
    pub open: bool,
    url: Entity<InputState>,
    name: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
    /// The probed server.
    info: Option<ServerInfo>,
    provider: usize,
    message: String,
    _subscriptions: Vec<Subscription>,
}

impl SetupDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://hermes.example.com"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name (optional)"));
        let user = cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let password = cx.new(|cx| InputState::new(window, cx).placeholder("Password").masked(true));
        let subscriptions = vec![
            cx.subscribe_in(&url, window, |this, input, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } => {
                    let url = input.read(cx).value().to_string();
                    this.state.update(cx, |s, cx| s.probe_server(&url, cx));
                }
                InputEvent::Change => {
                    this.info = None;
                    cx.notify();
                }
                _ => {}
            }),
            cx.subscribe_in(&password, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.sign_in(cx);
                }
            }),
            cx.subscribe_in(&user, window, |_, _, _: &InputEvent, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, window, cx| match event {
                AppEvent::ServerProbed(info) => {
                    this.info = info.clone();
                    this.message = if info.is_some() { String::new() } else { this.state.read(cx).error.clone() };
                    if let Some(info) = info {
                        this.provider = info.providers.iter().position(|p| p.supports_password).unwrap_or(0);
                    }
                    cx.notify();
                }
                AppEvent::SignInFinished(ok, message) => {
                    if *ok {
                        this.clear_form(window, cx);
                        this.open = false;
                    } else {
                        this.message = message.clone();
                    }
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        SetupDialog {
            state,
            open: false,
            url,
            name,
            user,
            password,
            info: None,
            provider: 0,
            message: String::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.message.clear();
        self.url.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.message.clear();
        self.password.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn clear_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.url, &self.name, &self.user, &self.password] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.info = None;
        self.message.clear();
    }

    /// Re-sign in to a saved server: fill the form and probe it.
    fn edit(&mut self, url: String, name: String, user: String, window: &mut Window, cx: &mut Context<Self>) {
        self.info = None;
        self.message.clear();
        self.url.update(cx, |i, cx| i.set_value(url.clone(), window, cx));
        self.name.update(cx, |i, cx| i.set_value(name, window, cx));
        self.user.update(cx, |i, cx| i.set_value(user, window, cx));
        self.password.update(cx, |i, cx| i.set_value("", window, cx));
        self.state.update(cx, |s, cx| s.probe_server(&url, cx));
        // Setting the url cleared the (not yet probed) info; that's fine.
    }

    fn uses_password(&self) -> bool {
        self.info.as_ref().and_then(|i| i.providers.get(self.provider)).is_some_and(|p| p.supports_password)
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.state.read(cx).busy {
            return;
        }
        let uses_password = self.uses_password();
        let (url, name, user, password) = (
            self.url.read(cx).value().to_string(),
            self.name.read(cx).value().to_string(),
            self.user.read(cx).value().to_string(),
            self.password.read(cx).value().to_string(),
        );
        if uses_password && (user.is_empty() || password.is_empty()) {
            return;
        }
        let provider = self.info.as_ref().and_then(|i| i.providers.get(self.provider)).map(|p| p.name.clone()).unwrap_or_default();
        self.message.clear();
        self.state.update(cx, |s, cx| s.sign_in(&url, &name, &provider, uses_password, &user, &password, cx));
    }
}

impl Render for SetupDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let busy = s.busy;
        let active = s.config.active_server.clone();
        let servers: Vec<_> = s.servers().to_vec();
        let url_text = self.url.read(cx).value().trim().to_string();
        let uses_password = self.uses_password();
        let can_sign_in = !busy
            && (!uses_password || (!self.user.read(cx).value().is_empty() && !self.password.read(cx).value().is_empty()));

        let saved = (!servers.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(heading("Saved servers", &c))
                .children(servers.iter().map(|server| {
                    let (sid, url, name, user) = (server.id.clone(), server.url.clone(), server.name.clone(), server.username.clone());
                    let remove_id = sid.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.))
                                .when(active.as_deref() == Some(sid.as_str()), |d| d.font_weight(FontWeight::BOLD))
                                .child(format!("{}  —  {}", server.name, server.url)),
                        )
                        .child(widgets::button(id("resign", &sid), "Sign in again", false, true, &c).on_click(cx.listener(
                            move |this, _, window, cx| this.edit(url.clone(), name.clone(), user.clone(), window, cx),
                        )))
                        .child(
                            widgets::button(id("remove-server", &sid), "Remove", false, true, &c)
                                .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.remove_server(&remove_id, cx)))),
                        )
                }))
                .child(div().h(px(1.)).bg(c.border))
        });

        let probed = self.info.clone().map(|info| {
            let providers = info.providers.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(muted(
                    format!("Hermes {}{}", info.version, if info.auth_required { "" } else { " (no sign-in required)" }),
                    13.,
                    &c,
                ))
                .child(Input::new(&self.name))
                .when(providers.len() > 1, |d| {
                    d.child(div().flex().flex_wrap().gap(px(6.)).children(providers.iter().enumerate().map(|(ix, p)| {
                        let label = if p.display_name.is_empty() { p.name.clone() } else { p.display_name.clone() };
                        widgets::chip(id("provider", ix), label, ix == self.provider, &c).on_click(cx.listener(move |this, _, _, cx| {
                            this.provider = ix;
                            cx.notify();
                        }))
                    })))
                })
                .when(uses_password, |d| d.child(Input::new(&self.user)).child(Input::new(&self.password).mask_toggle()))
                .when(!uses_password && info.auth_required, |d| d.child(muted("Your browser will open to sign in.", 13., &c)))
                .child(
                    div().flex().child(
                        widgets::button("sign-in", if busy { "Signing in…" } else { "Sign in" }, true, can_sign_in, &c)
                            .when(can_sign_in, |b| b.on_click(cx.listener(|this, _, _, cx| this.sign_in(cx)))),
                    ),
                )
        });

        let content = div()
            .flex()
            .flex_col()
            .child(div().px(px(20.)).pt(px(18.)).pb(px(10.)).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Servers"))
            .child(
                ident("setup-body")
                    .px(px(20.))
                    .pb(px(12.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .children(saved)
                    .child(heading("Add a server", &c))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(div().flex_1().child(Input::new(&self.url)))
                            .child(
                                widgets::button("check-server", "Check", false, !url_text.is_empty() && !busy, &c).when(
                                    !url_text.is_empty() && !busy,
                                    |b| {
                                        b.on_click(cx.listener(move |this, _, _, cx| {
                                            let url = this.url.read(cx).value().to_string();
                                            this.state.update(cx, |s, cx| s.probe_server(&url, cx));
                                        }))
                                    },
                                ),
                            ),
                    )
                    .children(probed)
                    .when(busy, |d| d.child(div().flex().justify_center().child(widgets::spinner(24.))))
                    .when(!self.message.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(c.danger).child(self.message.clone()))),
            )
            .child(
                div()
                    .px(px(20.))
                    .pb(px(18.))
                    .flex()
                    .justify_end()
                    .child(widgets::button("setup-close", "Close", false, true, &c).on_click(cx.listener(|this, _, window, cx| this.close(window, cx)))),
            );
        modal("setup-dialog", 560., content, &c).on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
            if e.keystroke.key == "escape" {
                this.close(window, cx);
            }
        }))
    }
}

// ── the bot's questions ──

const ANSWERABLE: &[&str] = &["approval", "clarify", "sudo", "secret", "vault.code", "vault.unlock_prompt", "display.install.sudo"];

/// Answers the questions a bot asks mid-turn: command approvals, clarifying
/// questions, and masked values (sudo, secrets, vault prompts).
pub struct RequestDialog {
    state: Entity<AppState>,
    pub open: bool,
    request_id: Value,
    method: String,
    params: Value,
    /// Clarify answers by question id; free-text ones live in `fields`.
    choices: HashMap<String, String>,
    fields: Vec<(String, Entity<InputState>)>,
    value: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl RequestDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscriptions = vec![cx.subscribe_in(&value, window, |this, input, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let value = input.read(cx).value().to_string();
                this.answer(json!({ "value": value }), cx);
            }
        })];
        RequestDialog {
            state,
            open: false,
            request_id: Value::Null,
            method: String::new(),
            params: Value::Null,
            choices: HashMap::new(),
            fields: Vec::new(),
            value,
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, id: Value, method: String, params: Value, window: &mut Window, cx: &mut Context<Self>) {
        // Requests that only make sense inside the official desktop app are
        // declined at once rather than leaving the bot waiting.
        if !ANSWERABLE.contains(&method.as_str()) {
            self.state.update(cx, |s, _| s.answer_request(id, json!({ "value": "" })));
            return;
        }
        self.choices.clear();
        self.fields.clear();
        if method == "clarify" {
            for q in params.get("questions").and_then(Value::as_array).cloned().unwrap_or_default() {
                let qid = q.get("qid").and_then(Value::as_str).unwrap_or_default().to_string();
                let choices = q.get("choices").and_then(Value::as_array).cloned().unwrap_or_default();
                match choices.first().and_then(Value::as_str) {
                    Some(first) => {
                        self.choices.insert(qid, first.to_string());
                    }
                    None => {
                        let input = cx.new(|cx| InputState::new(window, cx));
                        self.fields.push((qid, input));
                    }
                }
            }
        }
        self.value.update(cx, |i, cx| {
            i.set_value("", window, cx);
            i.focus(window, cx);
        });
        self.request_id = id;
        self.method = method;
        self.params = params;
        self.open = true;
        cx.notify();
    }

    fn answer(&mut self, result: Value, cx: &mut Context<Self>) {
        let id = std::mem::take(&mut self.request_id);
        self.state.update(cx, |s, _| s.answer_request(id, result));
        self.open = false;
        cx.notify();
    }

    fn clarify_answers(&self, cx: &App) -> Value {
        let mut answers = serde_json::Map::new();
        for (qid, choice) in &self.choices {
            answers.insert(qid.clone(), json!(choice));
        }
        for (qid, input) in &self.fields {
            answers.insert(qid.clone(), json!(input.read(cx).value().to_string()));
        }
        Value::Object(answers)
    }
}

impl Render for RequestDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let p = self.params.clone();
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let is_approval = self.method == "approval";
        let is_clarify = self.method == "clarify";
        let is_value = !is_approval && !is_clarify;
        let title = if is_approval {
            "Approve command?".to_string()
        } else if is_clarify {
            "The bot has a question".to_string()
        } else if !text("prompt").is_empty() {
            text("prompt")
        } else if self.method == "sudo" {
            "Sudo password".to_string()
        } else {
            "Value needed".to_string()
        };
        let command = text("command");
        let show_command = is_approval || (self.method == "sudo" && !command.is_empty());

        let questions = is_clarify.then(|| {
            let questions = p.get("questions").and_then(Value::as_array).cloned().unwrap_or_default();
            div().flex().flex_col().gap(px(12.)).children(questions.into_iter().enumerate().map(|(qi, q)| {
                let qid = q.get("qid").and_then(Value::as_str).unwrap_or_default().to_string();
                let question = q.get("question").and_then(Value::as_str).unwrap_or_default().to_string();
                let choices: Vec<String> = q
                    .get("choices")
                    .and_then(Value::as_array)
                    .map(|cs| cs.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let picked = self.choices.get(&qid).cloned();
                let field = self.fields.iter().find(|(id, _)| *id == qid).map(|(_, input)| input.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(question))
                    .when(!choices.is_empty(), |d| {
                        d.child(div().flex().flex_wrap().gap(px(6.)).children(choices.into_iter().enumerate().map(|(ci, choice)| {
                            let on = picked.as_deref() == Some(choice.as_str());
                            let qid = qid.clone();
                            widgets::chip(id("choice", format!("{qi}-{ci}")), choice.clone(), on, &c).on_click(cx.listener(move |this, _, _, cx| {
                                this.choices.insert(qid.clone(), choice.clone());
                                cx.notify();
                            }))
                        })))
                    })
                    .when_some(field, |d, input| d.child(Input::new(&input)))
            }))
        });

        let approval_choices: Vec<String> = p
            .get("choices")
            .and_then(Value::as_array)
            .map(|cs| cs.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .filter(|cs: &Vec<String>| !cs.is_empty())
            .unwrap_or_else(|| ["once", "session", "always", "deny"].map(str::to_string).to_vec());

        let content = div()
            .flex()
            .flex_col()
            .child(div().px(px(20.)).pt(px(18.)).pb(px(10.)).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                ident("request-body")
                    .px(px(20.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .when(is_approval && !text("description").is_empty(), |d| d.child(div().text_size(px(14.)).child(text("description"))))
                    .when(show_command, |d| {
                        d.child(
                            div()
                                .p(px(8.))
                                .rounded(c.radius)
                                .bg(c.background)
                                .border_1()
                                .border_color(c.border)
                                .font_family(mono(cx))
                                .text_size(px(13.))
                                .child(command.clone()),
                        )
                    })
                    .children(questions)
                    .when(is_value, |d| d.child(Input::new(&self.value).mask_toggle())),
            )
            .child(
                div()
                    .px(px(20.))
                    .py(px(16.))
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .when(is_approval, |d| {
                        d.children(approval_choices.into_iter().map(|choice| {
                            let label = match choice.as_str() {
                                "once" => "Allow once".to_string(),
                                "session" => "Allow for this chat".to_string(),
                                "always" => "Always allow".to_string(),
                                "deny" => "Deny".to_string(),
                                other => other.to_string(),
                            };
                            let primary = choice == "once";
                            widgets::button(id("approve", &choice), label, primary, true, &c)
                                .on_click(cx.listener(move |this, _, _, cx| this.answer(json!({ "choice": choice }), cx)))
                        }))
                    })
                    .when(is_clarify, |d| {
                        d.child(
                            widgets::button("clarify-skip", "Skip", false, true, &c)
                                .on_click(cx.listener(|this, _, _, cx| this.answer(json!({ "outcome": "cancelled" }), cx))),
                        )
                        .child(widgets::button("clarify-answer", "Answer", true, true, &c).on_click(cx.listener(|this, _, _, cx| {
                            let answers = this.clarify_answers(cx);
                            this.answer(json!({ "answers": answers, "outcome": "submitted" }), cx);
                        })))
                    })
                    .when(is_value, |d| {
                        d.child(
                            widgets::button("value-decline", "Decline", false, true, &c)
                                .on_click(cx.listener(|this, _, _, cx| this.answer(json!({ "value": "" }), cx))),
                        )
                        .child(widgets::button("value-send", "Send", true, true, &c).on_click(cx.listener(|this, _, _, cx| {
                            let value = this.value.read(cx).value().to_string();
                            this.answer(json!({ "value": value }), cx);
                        })))
                    }),
            );
        modal("request-dialog", 560., content, &c)
    }
}

// ── expensive model ──

/// Expensive models need a yes first.
pub struct ConfirmModel {
    pub message: String,
    pub model: String,
    pub provider: String,
}

pub fn render_confirm(confirm: &ConfirmModel, on_yes: impl Fn(&mut Window, &mut App) + 'static, on_no: impl Fn(&mut Window, &mut App) + 'static, cx: &App) -> Ident {
    let c = colors(cx);
    let content = div()
        .p(px(20.))
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Switch model?"))
        .child(div().text_size(px(14.)).child(confirm.message.clone()))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(widgets::button("confirm-cancel", "Cancel", false, true, c).on_click(move |_, w, cx| on_no(w, cx)))
                .child(widgets::button("confirm-yes", "Yes", true, true, c).on_click(move |_, w, cx| on_yes(w, cx))),
        );
    modal("confirm-model", 420., content, c)
}
