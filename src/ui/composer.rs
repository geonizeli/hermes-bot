//! The message box: grows with its text, Enter sends, Shift+Enter breaks the
//! line, and the round button turns into Stop while the bot answers. Below
//! the text: the chat's model, reasoning effort and fast mode. Typing "/"
//! opens the command menu above it.

use gpui_kit::component::input::{
    Escape, IndentInline, InputEvent, InputState, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::motion;
use super::state::{AppEvent, AppState, Provider};
use super::state::commands::Suggestion;
use super::theme::colors;
use super::widgets::{self, ident, MenuItem, PopupMenu, id, id as id_for, mono};

pub struct Composer {
    state: Entity<AppState>,
    pub(crate) input: Entity<TextareaState>,
    suggestions: Vec<Suggestion>,
    suggestion_index: usize,
    menu_dismissed: bool,
    pub(crate) picker_open: bool,
    picker_search: Entity<InputState>,
    effort_menu: Option<PopupMenu>,
    placeholder: String,
    _subscriptions: Vec<Subscription>,
}

const EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

impl Composer {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 9).submit_on_enter(true));
        let picker_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search models"));
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.menu_dismissed = false;
                    this.refresh_suggestions(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { shift: false, .. } => this.enter(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&picker_search, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    if let Some((model, provider)) = this.model_rows(cx).into_iter().next().map(|r| (r.0, r.1)) {
                        this.pick_model(&model, &provider, cx);
                    }
                }
                _ => {}
            }),
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, window, cx| match event {
                AppEvent::Prefill(text) => this.set_text(text, window, cx),
                AppEvent::OpenModelPicker => this.open_picker(window, cx),
                AppEvent::TranscriptLoaded => this.focus(window, cx),
                _ => {}
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        Composer {
            state,
            input,
            suggestions: Vec::new(),
            suggestion_index: 0,
            menu_dismissed: false,
            picker_open: false,
            picker_search,
            effort_menu: None,
            placeholder: String::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        let value = text.clone();
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        let line = text.split('\n').count().saturating_sub(1) as u32;
        let character = text.rsplit('\n').next().unwrap_or_default().encode_utf16().count() as u32;
        self.input.update(cx, |input, cx| {
            input.set_cursor_position(gpui_kit::component::input::Position { line, character }, window, cx)
        });
        self.refresh_suggestions(cx);
        cx.notify();
    }

    /// Queues the message box to send after the bot's current answer.
    fn queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        self.state.update(cx, |s, cx| s.queue_message(&text, cx));
        self.set_text("", window, cx);
    }

    /// Takes a queued message back into the message box, after anything
    /// already typed.
    fn edit_queued(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(queued) = self.state.update(cx, |s, cx| s.take_queued(id, cx)) else { return };
        let current = self.text(cx);
        let text = if current.trim().is_empty() { queued } else { format!("{}\n\n{queued}", current.trim_end()) };
        self.set_text(&text, window, cx);
    }

    /// The messages waiting for the bot's answer to end.
    fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let queue = s.queued()?.clone();
        let streaming = s.streaming;
        let count = queue.items.len();
        let title = format!("{count} queued{}", if queue.parked { " · paused" } else { "" });
        let header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(c.muted).child(title))
            .when(queue.parked, |d| {
                d.child(widgets::text_link("queue-resume", "Resume", &c).on_click(cx.listener(|this, _, _, cx| {
                    this.state.update(cx, |s, cx| s.resume_queue(cx))
                })))
            });
        let rows = queue.items.iter().map(|message| {
            let id = message.id;
            let group = SharedString::from(format!("queued-{id}"));
            let attachments = message.attachments.len();
            // While the bot answers a message can go in as a steer; once
            // it's idle, it can go now.
            let now = if streaming {
                message.can_steer().then(|| {
                    widgets::text_link(id_for("queued-steer", id), "Steer now", &c)
                        .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.steer_queued(id, cx))))
                })
            } else {
                Some(
                    widgets::text_link(id_for("queued-send", id), "Send now", &c)
                        .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.send_queued_now(id, cx)))),
                )
            };
            let row = ident(id_for("queued", id))
                .group(group.clone())
                .flex()
                .items_start()
                .gap(px(10.))
                .py(px(4.))
                .child(div().flex_none().pt(px(1.)).text_size(px(12.)).text_color(c.muted).child("↳"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_size(px(13.)).line_clamp(2).child(message.preview()))
                        .when(attachments > 0 && !message.text.trim().is_empty(), |d| {
                            d.child(widgets::muted(
                                if attachments == 1 { "1 attachment".to_string() } else { format!("{attachments} attachments") },
                                11.,
                                &c,
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(12.))
                        .children(now)
                        .child(
                            widgets::text_link(id_for("queued-edit", id), "Edit", &c)
                                .on_click(cx.listener(move |this, _, window, cx| this.edit_queued(id, window, cx))),
                        )
                        .child(
                            widgets::text_link(id_for("queued-remove", id), "✕", &c)
                                .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.remove_queued(id, cx)))),
                        ),
                );
            motion::rise_in(row, id_for("queued-in", id), 6., motion::QUICK)
        });
        let panel = ident("queue-panel")
            .flex()
            .flex_col()
            .gap(px(2.))
            .pb(px(8.))
            .mb(px(2.))
            .border_b_1()
            .border_color(c.border)
            .child(header)
            .children(rows);
        Some(panel.into_any_element())
    }

    fn menu_open(&self, window: &Window, cx: &App) -> bool {
        !self.suggestions.is_empty() && !self.menu_dismissed && self.input.focus_handle(cx).is_focused(window)
    }

    fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if !text.starts_with('/') {
            self.suggestions.clear();
            return;
        }
        self.suggestions = self.state.read(cx).slash_suggestions(&text);
        self.suggestion_index = 0;
    }

    fn accept_suggestion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.suggestions.get(index).cloned() {
            self.set_text(&s.insert, window, cx);
        }
    }

    /// Enter completes a partly typed command, then runs it.
    fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if self.menu_open(window, cx) {
            if let Some(s) = self.suggestions.get(self.suggestion_index) {
                if s.insert.trim() != text.trim() {
                    self.accept_suggestion(self.suggestion_index, window, cx);
                    return;
                }
            }
        }
        self.send(window, cx);
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let (streaming, pending) = {
            let s = self.state.read(cx);
            (s.streaming, !s.pending.is_empty())
        };
        // Commands go through send() even while the bot answers (/stop, /usage…).
        let is_command = text.starts_with('/')
            && text[1..].chars().next().is_some_and(|c| !c.is_whitespace() && c != '/');
        if is_command {
            self.state.update(cx, |s, cx| s.send(&text, cx));
            self.set_text("", window, cx);
            return;
        }
        if streaming {
            // Typing while the bot works steers it; an empty send stops it.
            // A steer can't carry attachments, so with any waiting the
            // message queues for after the answer instead.
            if pending {
                self.queue(window, cx);
            } else if !text.trim().is_empty() {
                self.state.update(cx, |s, cx| s.steer(&text, cx));
                self.set_text("", window, cx);
            } else {
                self.state.update(cx, |s, cx| s.stop(cx));
            }
            return;
        }
        if text.trim().is_empty() && !pending {
            return;
        }
        self.state.update(cx, |s, cx| s.send(&text, cx));
        self.set_text("", window, cx);
    }

    // ── model picker ──

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.refresh_models(cx));
        self.picker_open = true;
        self.picker_search.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn pick_model(&mut self, model: &str, provider: &str, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.set_model(model, provider, false, cx));
        self.picker_open = false;
        cx.notify();
    }

    /// (model, provider slug, provider label) filtered by the search.
    fn model_rows(&self, cx: &App) -> Vec<(String, String, String)> {
        let q = self.picker_search.read(cx).value().trim().to_lowercase();
        let mut out = Vec::new();
        for p in &self.state.read(cx).models {
            let label = if p.authenticated { p.name.clone() } else { format!("{}  (not signed in)", p.name) };
            for m in &p.models {
                if !q.is_empty() && !m.to_lowercase().contains(&q) && !p.name.to_lowercase().contains(&q) {
                    continue;
                }
                out.push((m.clone(), p.slug.clone(), label.clone()));
            }
        }
        out
    }

    fn render_picker(&self, current_model: &str, current_provider: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let rows = self.model_rows(cx);
        let loading = self.state.read(cx).models_loading;
        let mut children: Vec<AnyElement> = Vec::new();
        let mut last_section = String::new();
        for (ix, (model, provider, label)) in rows.iter().enumerate() {
            if *label != last_section {
                last_section = label.clone();
                children.push(
                    div()
                        .pt(px(8.))
                        .pb(px(4.))
                        .px(px(8.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(c.muted)
                        .child(label.to_uppercase())
                        .into_any_element(),
                );
            }
            let current = model == current_model && (current_provider.is_empty() || provider == current_provider);
            let (m, p) = (model.clone(), provider.clone());
            let hover = c.raised;
            children.push(
                ident(id("model", ix))
                    .h(px(32.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(c.radius)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_model(&m, &p, cx)))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.))
                            .when(current, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(model.clone()),
                    )
                    .when(current, |d| d.child(div().text_color(c.accent).child("✓")))
                    .into_any_element(),
            );
        }
        if rows.is_empty() {
            children.push(
                div()
                    .py(px(16.))
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap(px(8.))
                    .text_color(c.muted)
                    .when(loading, |d| d.child(widgets::spinner(14.)))
                    .child(if loading { "Loading models…" } else { "No models match" })
                    .into_any_element(),
            );
        }
        ident("model-picker")
            .occlude()
            .absolute()
            .left_0()
            .bottom(relative(1.))
            .mb(px(8.))
            .w(px(420.))
            .max_w_full()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .bg(c.surface)
            .border_1()
            .border_color(c.border)
            .rounded(c.radius)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.picker_open = false;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    this.picker_open = false;
                    this.focus(window, cx);
                    cx.notify();
                }
            }))
            .child(gpui_kit::component::input::Input::new(&self.picker_search))
            .child(ident("model-list").max_h(px(400.)).overflow_y_scroll().flex().flex_col().children(children))
            .with_animation("model-picker-in", Animation::new(motion::QUICK).with_easing(motion::ease_out), rise_from_composer)
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let font = mono(cx);
        let rows = self.suggestions.iter().enumerate().map(|(ix, s)| {
            let selected = ix == self.suggestion_index;
            let hover = c.surface;
            ident(id("slash", ix))
                .h(px(34.))
                .px(px(10.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .rounded(c.radius)
                .when(selected, |d| d.bg(c.accent.alpha(0.18)))
                .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| this.accept_suggestion(ix, window, cx)))
                .child(div().flex_none().font_family(font.clone()).text_size(px(13.)).child(s.display.clone()))
                .when(s.skill, |d| d.child(div().flex_none().text_size(px(11.)).text_color(c.accent).child("skill")))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(c.muted).child(s.meta.clone()))
        });
        ident("slash-menu")
            .occlude()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .mb(px(8.))
            .max_h(px(328.))
            .overflow_y_scroll()
            .p(px(4.))
            .flex()
            .flex_col()
            .bg(c.raised)
            .border_1()
            .border_color(c.border)
            .rounded(c.radius)
            .children(rows)
            .with_animation("slash-menu-in", Animation::new(motion::QUICK).with_easing(motion::ease_out), rise_from_composer)
    }
}

/// Popups above the message box rise out of it as they open.
fn rise_from_composer<E: Styled>(el: E, t: f32) -> E {
    el.opacity(t).mb(px(8. - 6. * (1. - t)))
}

/// Capabilities of the current model, from whichever provider lists it.
fn capabilities(providers: &[Provider], model: &str, provider: &str) -> Value {
    for p in providers {
        if !provider.is_empty() && p.slug != provider && p.name != provider {
            continue;
        }
        if let Some(caps) = p.capabilities.get(model).filter(|c| c.is_object()) {
            return caps.clone();
        }
    }
    Value::Null
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let bot_name = s.active_bot().map(|b| b.display_name.clone()).unwrap_or_else(|| "the bot".into());
        let connected = s.connected;
        let enabled = connected && !s.chat_loading;
        let streaming = s.streaming;
        let info = s.session_info.clone();
        let text_of = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let current_model = Some(text_of("model")).filter(|m| !m.is_empty()).unwrap_or_else(|| s.active_bot().map(|b| b.model.clone()).unwrap_or_default());
        let current_provider = text_of("provider");
        let caps = capabilities(&s.models, &current_model, &current_provider);
        let shows_effort = caps.get("reasoning") != Some(&Value::Bool(false));
        let fast_on = info.get("fast").and_then(Value::as_bool).unwrap_or(false);
        let shows_fast = caps.get("fast").and_then(Value::as_bool).unwrap_or(false)
            || caps.get("ultrafast").and_then(Value::as_bool).unwrap_or(false)
            || fast_on;
        let effort = Some(text_of("reasoning_effort")).filter(|e| !e.is_empty()).unwrap_or_else(|| "medium".into());
        let can_disable = caps.get("can_disable_reasoning").and_then(Value::as_bool).unwrap_or(false);
        let pending: Vec<(u64, bool, String, std::path::PathBuf)> = s
            .pending
            .iter()
            .map(|a| (a.id, a.kind == crate::backend::attachments::Kind::Image, a.name.clone(), a.path.clone()))
            .collect();

        let placeholder = if !connected {
            "Not connected".to_string()
        } else if streaming {
            format!("Steer {bot_name}… (empty send stops)")
        } else {
            format!("Message {bot_name}")
        };
        if placeholder != self.placeholder {
            self.placeholder = placeholder.clone();
            self.input.update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
        }

        let text = self.text(cx);
        let has_text = !text.trim().is_empty();
        let can_send = connected && (has_text || !pending.is_empty());
        let live = streaming || can_send;
        let stopping = streaming && !has_text && pending.is_empty();
        let focused = self.input.focus_handle(cx).is_focused(window);
        let menu_open = self.menu_open(window, cx);

        let queue_panel = self.render_queue(cx);
        let chips = (!pending.is_empty()).then(|| {
            div().flex().flex_wrap().gap(px(6.)).children(pending.iter().map(|(aid, image, name, path)| {
                let aid = *aid;
                let remove = ident(id("remove-attachment", aid))
                    .absolute()
                    .top(px(3.))
                    .right(px(3.))
                    .size(px(18.))
                    .rounded_full()
                    .bg(black().alpha(0.55))
                    .hover(|s| s.bg(black().alpha(0.8)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.))
                    .text_color(white())
                    .cursor_pointer()
                    .child("✕")
                    .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.remove_attachment(aid, cx))));
                let chip = div()
                    .relative()
                    .rounded(c.radius)
                    .bg(c.surface)
                    .border_1()
                    .border_color(c.border)
                    .overflow_hidden();
                if *image {
                    chip.size(px(64.)).child(img(path.clone()).size_full().object_fit(ObjectFit::Cover)).child(remove)
                } else {
                    chip.h(px(34.))
                        .pl(px(8.))
                        .pr(px(26.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().text_size(px(13.)).child("📄"))
                        .child(div().max_w(px(200.)).truncate().text_size(px(12.)).child(name.clone()))
                        .child(remove)
                }
            }))
        });

        let state = self.state.clone();
        let editor = Textarea::new(&self.input)
            .appearance(false)
            .disabled(!enabled)
            .text_size(px(15.))
            .on_paste(move |item, _, cx| state.update(cx, |s, cx| s.paste(item, cx)));

        let effort_chip = shows_effort.then(|| {
            let mut levels: Vec<&str> = Vec::new();
            if can_disable {
                levels.push("none");
            }
            levels.extend_from_slice(EFFORTS);
            let levels: Vec<String> = levels.into_iter().map(str::to_string).collect();
            let current = effort.clone();
            widgets::chip("effort-chip", format!("Effort: {effort}  ▾"), false, &c).on_click(cx.listener(
                move |this, e: &ClickEvent, _, cx| {
                    let state = this.state.clone();
                    let items = levels
                        .iter()
                        .map(|level| {
                            let (state, value) = (state.clone(), level.clone());
                            let label = if level == "none" { "none (no thinking)".to_string() } else { level.clone() };
                            MenuItem::new(label, move |_, cx| state.update(cx, |s, cx| s.set_effort(&value, cx)))
                                .checked(*level == current)
                        })
                        .collect();
                    this.effort_menu = Some(PopupMenu::new(e.position(), items).upward());
                    cx.notify();
                },
            ))
        });

        let action_button = {
            let ink = if live { c.accent_foreground } else { c.muted };
            ident("send")
                .size(px(36.))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(live, |d| d.cursor_pointer().hover(|s| s.opacity(0.85)).on_click(cx.listener(|this, _, window, cx| this.send(window, cx))))
                .child(if stopping {
                    div().size(px(12.)).rounded(px(2.)).bg(c.accent_foreground).into_any_element()
                } else {
                    div().text_size(px(18.)).font_weight(FontWeight::BOLD).text_color(ink).child("↑").into_any_element()
                })
                // Lights up as soon as there's something to send.
                .with_spring("send-live", SpringAnimation::new(motion::settle()).to(live), {
                    let (off, on) = (c.border, c.accent);
                    move |d, p| d.bg(p.interpolate_clamped(off, on))
                })
        };

        let controls = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .when(!enabled, |d| d.opacity(0.5))
            .child(
                widgets::chip("attach", "+", false, &c)
                    .w(px(28.))
                    .px_0()
                    .justify_center()
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Attach files (or drop / paste them)").build(window, cx))
                    .when(enabled, |d| d.on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |s, _| s.pick_files())))),
            )
            .child(
                widgets::chip("model-chip", format!("{}  ▾", if current_model.is_empty() { "Model" } else { &current_model }), false, &c)
                    .when(enabled, |d| d.on_click(cx.listener(|this, _, window, cx| this.open_picker(window, cx)))),
            )
            .children(effort_chip.filter(|_| enabled))
            .when(shows_fast && enabled, |d| {
                d.child(widgets::chip("fast-chip", "⚡ Fast", fast_on, &c).on_click(cx.listener(move |this, _, _, cx| {
                    this.state.update(cx, |s, cx| s.set_fast(!fast_on, cx))
                })))
            })
            .child(div().flex_1())
            // While the bot answers, Enter steers; this sends after it instead.
            .when(streaming && (has_text || !pending.is_empty()), |d| {
                d.child(
                    widgets::chip("queue", "Queue", false, &c)
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new("Send after the bot's current answer").build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.queue(window, cx))),
                )
            })
            .child(action_button);

        let close_effort: widgets::Action = {
            let this = cx.entity().downgrade();
            std::rc::Rc::new(move |_, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.effort_menu = None;
                    cx.notify();
                });
            })
        };

        ident("composer")
            .relative()
            .w_full()
            .px(px(16.))
            .pr(px(10.))
            .py(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .rounded(c.radius)
            .bg(c.raised)
            .border_1()
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.menu_open(window, cx) {
                    let n = this.suggestions.len();
                    this.suggestion_index = (this.suggestion_index + n - 1) % n;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.menu_open(window, cx) {
                    this.suggestion_index = (this.suggestion_index + 1) % this.suggestions.len();
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.menu_open(window, cx) {
                    this.accept_suggestion(this.suggestion_index, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.menu_open(window, cx) {
                    this.menu_dismissed = true;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .children(queue_panel)
            .children(chips)
            .child(div().w_full().text_color(c.foreground).child(editor))
            .child(controls)
            .when(menu_open, |d| d.child(self.render_menu(cx)))
            .when(self.picker_open, |d| d.child(self.render_picker(&current_model, &current_provider, cx)))
            .when_some(self.effort_menu.as_ref(), |d, menu| d.child(menu.render(close_effort.clone(), cx)))
            // The border warms to the accent while you type.
            .with_spring("composer-focus", SpringAnimation::new(motion::settle()).to(focused), {
                let (off, on) = (c.border, c.accent.alpha(0.6));
                move |d, p| d.border_color(p.interpolate_clamped(off, on))
            })
    }
}
