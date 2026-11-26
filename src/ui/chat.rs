//! The open Bot Chat: a header, the conversation in a centered column, and
//! the composer. The side panel docks beside the chat when there's room and
//! floats over it in a narrow window.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{Local, TimeZone};
use gpui_kit::component::input::{Escape, InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::composer::Composer;
use super::markdown_view;
use super::motion;
use super::screen::ScreenView;
use super::side_panel;
use super::state::{AppEvent, AppState, Role, Row};
use super::theme::colors;
use super::tool_card;
use super::widgets::{self, ident, accent_surface, id, mono};

pub const SIDEBAR_WIDTH: f32 = 256.;
pub const CHAT_COLUMN: f32 = 764.;
const HEADER_HEIGHT: f32 = 56.;
const PANEL_WIDTH: f32 = 320.;

pub struct ChatView {
    state: Entity<AppState>,
    pub(crate) composer: Entity<Composer>,
    pub(crate) screen: Entity<ScreenView>,
    screen_expanded: bool,
    /// The bot whose screen is connected ("" when none).
    run_key: String,
    pub(crate) jobs_expanded: Entity<HashSet<String>>,
    /// The text box of the message being edited in place.
    pub(crate) edit_input: Entity<TextareaState>,
    on_edit_bot: std::rc::Rc<dyn Fn(String, &mut Window, &mut App)>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

fn time_label(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_opt(ts as i64, 0).single() else { return String::new() };
    if t.date_naive() == Local::now().date_naive() { t.format("%H:%M").to_string() } else { t.format("%b %-d, %H:%M").to_string() }
}

fn k(n: f64) -> String {
    if n >= 1000.0 {
        if n >= 10000.0 { format!("{:.0}k", n / 1000.0) } else { format!("{:.1}k", n / 1000.0) }
    } else {
        format!("{}", n.round())
    }
}

impl ChatView {
    pub fn new(
        state: Entity<AppState>,
        on_edit_bot: impl Fn(String, &mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| Composer::new(state.clone(), window, cx));
        let screen = cx.new(ScreenView::new);
        let jobs_expanded = cx.new(|_| HashSet::new());
        let edit_input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 14).submit_on_enter(true));
        let weak = cx.entity().downgrade();
        state.read(cx).list.set_scroll_handler(move |_, _, cx| {
            let _ = weak.update(cx, |_, cx| cx.notify());
        });
        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| {
                this.sync_screen(cx);
                cx.notify();
            }),
            cx.observe(&screen, |_, _, cx| cx.notify()),
            cx.observe(&jobs_expanded, |_, _, cx| cx.notify()),
            cx.subscribe_in(&edit_input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    this.save_edit(window, cx);
                }
            }),
        ];
        // The job list and screen status go stale while shown; poll them.
        let poll = cx.spawn(async move |this, cx| {
            let mut tick: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                tick += 1;
                let Ok(()) = this.update(cx, |this, cx| {
                    let (open, tab, connected) = {
                        let s = this.state.read(cx);
                        (s.config.ui.side_panel_open, s.config.ui.side_panel_tab.clone(), s.connected)
                    };
                    if !connected || !open {
                        return;
                    }
                    if tab == "jobs" && tick % 4 == 0 {
                        this.state.update(cx, |s, cx| s.refresh_jobs(cx));
                    } else if tab == "screen" && tick % 3 == 0 {
                        this.state.update(cx, |s, cx| s.refresh_screen(cx));
                    }
                }) else {
                    break;
                };
            }
        });
        ChatView {
            state,
            composer,
            screen,
            screen_expanded: false,
            run_key: String::new(),
            jobs_expanded,
            edit_input,
            on_edit_bot: std::rc::Rc::new(on_edit_bot),
            _subscriptions: subscriptions,
            _poll: poll,
        }
    }

    /// Opens your message at row `ix` for editing, in place.
    pub fn start_edit(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.state.read(cx).rows.get(ix).map(|r| r.text.clone()) else { return };
        let line = text.split('\n').count().saturating_sub(1) as u32;
        let character = text.rsplit('\n').next().unwrap_or_default().encode_utf16().count() as u32;
        self.edit_input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_cursor_position(gpui_kit::component::input::Position { line, character }, window, cx);
            input.focus(window, cx);
        });
        self.state.update(cx, |s, cx| {
            s.start_edit(ix);
            cx.notify();
        });
    }

    /// Sends the edited message in place of the original.
    pub fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.state.read(cx).editing_index() else { return };
        let text = self.edit_input.read(cx).value().to_string();
        self.state.update(cx, |s, cx| s.resend_from(ix, &text, cx));
        self.composer.update(cx, |c, cx| c.focus(window, cx));
    }

    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| {
            s.cancel_edit();
            cx.notify();
        });
        self.composer.update(cx, |c, cx| c.focus(window, cx));
    }

    /// Connects the screen only while someone can see it.
    fn sync_screen(&mut self, cx: &mut Context<Self>) {
        let s = self.state.read(cx);
        let running = s.screen.get("running").and_then(|v| v.as_bool()) == Some(true);
        let lease = s.screen.get("lease").cloned();
        let shown = s.config.ui.side_panel_open && s.config.ui.side_panel_tab == "screen";
        let bot = s.active_bot.clone();
        if !running {
            self.screen_expanded = false;
        }
        let key = if running && (shown || self.screen_expanded) { bot } else { String::new() };
        if let Some(lease) = lease {
            self.screen.update(cx, |v, cx| v.apply_lease(&lease, cx));
        }
        if key != self.run_key {
            self.run_key = key.clone();
            self.screen.update(cx, |v, cx| {
                v.detach(cx);
                if !key.is_empty() {
                    v.attach(&key, cx);
                }
            });
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let bot = s.active_bot().cloned().unwrap_or_default();
        let usage = s.usage.clone().filter(|u| u.context_max > 0.0);
        let (open, tab) = s.side_panel();
        let jobs_count = s.jobs.len();
        let toggles = [("jobs", "Scheduled", jobs_count), ("screen", "Screen", 0)].map(|(name, label, count)| {
            let on = open && tab == name;
            let hover = c.raised.alpha(0.6);
            let state = self.state.clone();
            ident(id("toggle", name))
                .h(px(30.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(c.radius)
                .border_1()
                .border_color(if on { c.accent.alpha(0.5) } else { c.border })
                .bg(if on { c.raised } else { transparent_black() })
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if on { c.foreground } else { c.muted })
                .child(if count > 0 { format!("{label}  {count}") } else { label.to_string() })
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.toggle_side_panel(name, cx)))
        });
        let edit = self.on_edit_bot.clone();
        let name = bot.name.clone();
        div()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .px(px(20.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(c.border)
            .child(
                ident("bot-identity")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .cursor_pointer()
                    .tooltip(|window, cx| Tooltip::new("Edit profile").build(window, cx))
                    .on_click(move |_, window, cx| edit(name.clone(), window, cx))
                    .child(widgets::avatar(&bot.display_name, bot.avatar.clone(), bot.avatar_face, 28.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(bot.display_name.clone())),
            )
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(c.muted).child(bot.model.clone()))
            .when_some(usage, |d, u| {
                let tip = format!(
                    "{} / {} tokens in context{}",
                    k(u.context_used),
                    k(u.context_max),
                    if u.cost >= 0.0 { format!("\nCost so far: ${:.4}", u.cost) } else { String::new() }
                );
                let fill = (u.percent / 100.0).clamp(0.0, 1.0) as f32;
                d.child(
                    ident("context-gauge")
                        .w(px(120.))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .child(widgets::muted(format!("{}% context", u.percent.round()), 11., &c))
                        .child(
                            div().w_full().h(px(3.)).rounded(px(1.5)).bg(c.border).child(
                                div()
                                    .h_full()
                                    .rounded(px(1.5))
                                    .bg(if u.percent > 85.0 { c.danger } else { c.accent })
                                    // Fills to the new amount after each turn.
                                    .with_spring(
                                        "context-fill",
                                        SpringAnimation::new(motion::settle()).to(fill).from(0.0),
                                        |d, f| d.w(relative(f.clamp(0.0, 1.0))),
                                    ),
                            ),
                        ),
                )
            })
            .children(toggles)
    }

    fn render_empty(&self, cx: &App) -> impl IntoElement {
        let c = colors(cx);
        let s = self.state.read(cx);
        let bot = s.active_bot();
        let greeting = match bot {
            Some(_) => time_greeting().to_string(),
            None if s.connected => "Pick a bot".into(),
            None => "Not connected".into(),
        };
        let subtitle = bot.and_then(|b| (!b.description.is_empty()).then(|| b.description.clone()));
        const STARTERS: &[&str] = &[
            "What can you help me with?",
            "Review the latest code changes",
            "Run the tests and report failures",
        ];
        let state = self.state.clone();
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .w_full()
            .max_w(px(CHAT_COLUMN))
            .px(px(8.))
            .child(div().text_size(px(28.)).font_weight(FontWeight::SEMIBOLD).text_center().child(greeting))
            .when_some(subtitle, |d, sub| d.child(widgets::muted(sub, 15., c).text_center().max_w(px(420.))))
            .when(bot.is_some(), |d| {
                d.child(
                    div()
                        .w_full()
                        .mt(px(8.))
                        .children(STARTERS.iter().enumerate().map(|(i, text)| {
                            let state = state.clone();
                            let msg = (*text).to_string();
                            ident(id("starter", i))
                                .w_full()
                                .py(px(11.))
                                .border_b_1()
                                .border_color(c.border)
                                .text_size(px(15.))
                                .text_color(c.muted)
                                .cursor_pointer()
                                .hover(|s| s.text_color(c.foreground))
                                .on_click(move |_, _, cx| {
                                    let text = msg.clone();
                                    state.update(cx, |_, cx| cx.emit(AppEvent::Prefill(text)));
                                })
                                .child(*text)
                        })),
                )
            })
    }
}

fn time_greeting() -> &'static str {
    use chrono::Timelike;
    match Local::now().hour() {
        h if h < 12 => "Good morning",
        h if h < 17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// Where a row's text starts in the window's reading order, so a selection
/// across messages copies them top to bottom. Rows count up as they're
/// added; each gets 2^24 slots for its blocks and paragraphs.
pub fn reading_order(row_id: u64) -> u64 {
    row_id << 24
}

/// What a row needs from the chat view.
#[derive(Clone)]
struct RowContext {
    state: Entity<AppState>,
    chat: WeakEntity<ChatView>,
    edit_input: Entity<TextareaState>,
}

/// Your message as an editable text box, with Cancel and Send.
fn render_editor(row: &Row, rows: &RowContext, later: bool, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let busy = rows.state.read(cx).streaming;
    let (cancel, send, escape) = (rows.chat.clone(), rows.chat.clone(), rows.chat.clone());
    let note = if later {
        "Sending replaces this message and everything after it."
    } else {
        "Sending replaces this message."
    };
    let card = ident(id("editing", row.id))
        .w(relative(0.8))
        .px(px(14.))
        .py(px(10.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .rounded(c.radius)
        .bg(accent_surface(&c))
        .border_1()
        .border_color(c.accent.alpha(0.6))
        .capture_action(move |_: &Escape, window, cx| {
            let _ = escape.update(cx, |chat, cx| chat.cancel_edit(window, cx));
            cx.stop_propagation();
        })
        .child(div().w_full().text_color(c.foreground).child(Textarea::new(&rows.edit_input).appearance(false).text_size(px(15.))))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().child(widgets::muted(note, 12., &c)))
                .child(widgets::button(id("edit-cancel", row.id), "Cancel", false, true, &c).on_click(move |_, window, cx| {
                    let _ = cancel.update(cx, |chat, cx| chat.cancel_edit(window, cx));
                }))
                .child(widgets::button(id("edit-send", row.id), "Send", true, !busy, &c).on_click(move |_, window, cx| {
                    let _ = send.update(cx, |chat, cx| chat.save_edit(window, cx));
                })),
        );
    motion::fade_in(card, id("editing-in", row.id), motion::QUICK).into_any_element()
}

fn render_user(ix: usize, row: &Row, rows: &RowContext, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let state = &rows.state;
    let s = state.read(cx);
    let editable = !s.streaming && s.connected && s.editing.is_none();
    let editing = s.editing == Some(row.id);
    let later = ix + 1 < s.rows.len();
    let group = SharedString::from(format!("user-{}", row.id));
    let files = (!row.attachments.is_empty()).then(|| {
        let images: Vec<AnyElement> = row
            .attachments
            .iter()
            .enumerate()
            .map(|(i, f)| {
                if f.is_image() {
                    markdown_view::remote_image(id("sent-img", format!("{}-{i}", row.id)), &f.path, &f.url, 240., 180., state, cx)
                } else {
                    div()
                        .h(px(34.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(c.radius)
                        .bg(c.raised)
                        .border_1()
                        .border_color(c.border)
                        .text_size(px(12.))
                        .child("📄")
                        .child(f.name.clone())
                        .into_any_element()
                }
            })
            .collect();
        div().flex().flex_wrap().justify_end().gap(px(6.)).max_w(relative(0.8)).children(images)
    });
    let chat = rows.chat.clone();
    let actions = div()
        .flex()
        .gap(px(12.))
        .opacity(0.)
        .group_hover(group.clone(), |s| s.opacity(1.))
        .when(row.timestamp > 0.0, |d| d.child(widgets::muted(time_label(row.timestamp), 12., &c)))
        .when(editable, |d| {
            d.child(widgets::text_link(id("edit", row.id), "Edit", &c).on_click(move |_, window, cx| {
                let _ = chat.update(cx, |chat, cx| chat.start_edit(ix, window, cx));
            }))
        });
    if editing {
        return div()
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(6.))
            .children(files)
            .child(render_editor(row, rows, later, cx))
            .into_any_element();
    }
    let bubble = (!row.text.is_empty()).then(|| {
        ident(id("sent", row.id))
            .max_w(relative(0.8))
            .px(px(16.))
            .py(px(11.))
            .rounded(c.radius)
            .bg(c.bubble_user)
            .border_1()
            .border_color(c.border)
            .text_size(px(15.))
            .line_height(relative(1.45))
            .child(markdown_view::plain_text(id("sent-text", row.id), reading_order(row.id), row.text.clone(), cx))
    });
    div()
        .group(group)
        .w_full()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(6.))
        .children(files)
        .when_some(bubble, |d, bubble| {
            d.child(div().w_full().flex().items_center().justify_end().gap(px(12.)).child(actions).child(bubble))
        })
        .into_any_element()
}

fn render_assistant(ix: usize, row: &Row, state: &Entity<AppState>, window: &mut Window, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let s = state.read(cx);
    let can_regenerate = s.last_user_index.is_some_and(|u| ix > u) && !s.streaming;
    let last_user_text = s.last_user_index.and_then(|u| s.rows.get(u)).map(|r| r.text.clone()).unwrap_or_default();
    let group = SharedString::from(format!("bot-{}", row.id));
    let thinking = !row.done && row.text.is_empty();
    let reasoning_toggle = (!row.reasoning.is_empty()).then(|| {
        let label = if thinking {
            "Thinking…".to_string()
        } else if row.thought_ms > 0 {
            format!("Thought for {}s", (row.thought_ms as f64 / 1000.0).round().max(1.0))
        } else {
            "Reasoning".to_string()
        };
        let label = format!("{label}  {}", if row.show_reasoning { "▾" } else { "▸" });
        let st = state.clone();
        let el = ident(id("reasoning-toggle", row.id))
            .h(px(22.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(c.magenta)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                st.update(cx, |s, cx| {
                    s.toggle_reasoning(ix);
                    cx.notify();
                })
            })
            .child(label);
        if thinking {
            motion::breathe(el, id("thinking", row.id), 0.4).into_any_element()
        } else {
            el.into_any_element()
        }
    });
    let opened = motion::fresh(row.toggled_at, motion::SLOW);
    let reasoning = (row.show_reasoning && !row.reasoning.is_empty()).then(|| {
        let body = div()
            .border_l_2()
            .border_color(c.magenta.alpha(0.6))
            .pl(px(14.))
            .text_size(px(14.))
            .text_color(c.muted)
            .child(markdown_view::plain_text(id("reasoning-text", row.id), reading_order(row.id), row.reasoning.clone(), cx));
        motion::rise_in_when(opened, body, id("reasoning-in", row.id), -6., motion::BASE)
    });
    // A caret breathes after the text while the answer streams in.
    let caret = (!row.done && !row.text.is_empty())
        .then(|| motion::breathe(div().w(px(8.)).h(px(16.)).bg(c.accent.alpha(0.8)), id("caret", row.id), 0.15));
    let body = (!row.text.is_empty()).then(|| ident(id("answer", row.id)).w_full().child(markdown_view::render(row.id, reading_order(row.id) + (1 << 12), &row.text, state, window, cx)));
    let actions = (row.done && !row.text.is_empty()).then(|| {
        let st = state.clone();
        div()
            .flex()
            .gap(px(14.))
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .child(markdown_view::copy_link(id("copy-answer", row.id), row.text.clone(), window, cx))
            .when(can_regenerate, |d| {
                d.child(widgets::text_link(id("regenerate", row.id), "Regenerate", &c).on_click(move |_, _, cx| {
                    let text = last_user_text.clone();
                    st.update(cx, |s, cx| {
                        if let Some(u) = s.last_user_index {
                            s.resend_from(u, &text, cx);
                        }
                    });
                }))
            })
            .when(row.timestamp > 0.0, |d| d.child(widgets::muted(time_label(row.timestamp), 12., &c)))
    });
    div()
        .group(group)
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.))
        .children(reasoning_toggle)
        .children(reasoning)
        .children(body)
        .children(caret)
        .children(actions)
        .into_any_element()
}

fn render_command(row: &Row, window: &mut Window, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let _ = window;
    div()
        .w_full()
        .px(px(12.))
        .py(px(10.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .rounded(c.radius)
        .bg(c.surface)
        .border_1()
        .border_color(c.border)
        .font_family(mono(cx))
        .text_size(px(12.))
        .child(if row.done {
            div().text_color(c.accent).child(row.name.clone()).into_any_element()
        } else {
            motion::breathe(div().text_color(c.accent).child(format!("{}  · running…", row.name)), id("command-running", row.id), 0.45)
                .into_any_element()
        })
        .when(!row.text.is_empty(), |d| {
            d.child(div().text_color(c.foreground).child(markdown_view::plain_text(id("command-text", row.id), reading_order(row.id), row.text.clone(), cx)))
        })
        .into_any_element()
}

/// The dots shown while waiting for the first word of an answer.
fn waiting_dots(cx: &App) -> impl IntoElement {
    let c = colors(cx);
    let dots = div().pt(px(14.)).flex().gap(px(6.)).children((0..3).map(|i| {
        div().size(px(7.)).rounded_full().bg(c.muted).with_animation(
            id("waiting-dot", i),
            Animation::new(Duration::from_millis(1280)).repeat(),
            move |el, t| {
                // Each dot dims in turn.
                let phase = (t - i as f32 * 0.125).rem_euclid(1.0);
                let dip = if phase < 0.3125 { phase / 0.3125 } else if phase < 0.625 { 1.0 - (phase - 0.3125) / 0.3125 } else { 0.0 };
                el.opacity(1.0 - dip * 0.75)
            },
        )
    }));
    motion::fade_in(dots, "waiting-dots-in", motion::BASE)
}

/// What the chat shows while it connects or opens: the shape of a
/// conversation, breathing, where the messages will be.
fn loading_transcript(cx: &App) -> impl IntoElement {
    let c = colors(cx);
    let exchange = |n: usize, lines: usize| {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(div().w_full().flex().justify_end().child(motion::skeleton(id("loading-ask", n), px(220. + 60. * (n % 2) as f32), 40., c)))
            .child(motion::skeleton_lines(&format!("loading-answer-{n}"), lines, 14., c))
    };
    ident("chat-loading")
        .absolute()
        .inset_0()
        .overflow_hidden()
        .px(px(24.))
        .pt(px(24.))
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(28.))
                .child(exchange(0, 3))
                .child(exchange(1, 5))
                .child(exchange(2, 2)),
        )
}

fn render_row(ix: usize, rows: &RowContext, window: &mut Window, cx: &mut App) -> AnyElement {
    let state = &rows.state;
    let (row, count, waiting) = {
        let s = state.read(cx);
        let Some(row) = s.rows.get(ix).cloned() else { return div().into_any_element() };
        (row, s.rows.len(), s.waiting_for_answer())
    };
    let c = colors(cx).clone();
    let content = match row.role {
        Role::User => render_user(ix, &row, rows, cx),
        Role::Assistant => render_assistant(ix, &row, state, window, cx),
        Role::Tool => tool_card::render(ix, &row, state, cx),
        Role::Todos => tool_card::render_todos(&row, cx),
        Role::Error => div().text_size(px(14.)).text_color(c.danger).child(row.text.clone()).into_any_element(),
        Role::Command => render_command(&row, window, cx),
        Role::Info => div().text_size(px(13.)).italic().text_color(c.muted).child(row.text.clone()).into_any_element(),
    };
    let last = ix + 1 == count;
    // Messages that arrive while the chat is open rise into place; your own
    // come up from the composer a little further.
    let lift = if row.role == Role::User { 14. } else { 8. };
    let column = div()
        .w_full()
        .flex()
        .flex_col()
        .child(content)
        .when(last && waiting, |d| d.child(waiting_dots(cx)));
    div()
        .w_full()
        .px(px(24.))
        .pt(px(if ix == 0 { 24. } else { 18. }))
        .when(last, |d| d.pb(px(24.)))
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(CHAT_COLUMN))
                .child(motion::rise_in_when(motion::fresh(row.born, motion::SLOW * 2), column, id("row-in", row.id), lift, motion::BASE)),
        )
        .into_any_element()
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let width = window.viewport_size().width.as_f32() - SIDEBAR_WIDTH;
        let s = self.state.read(cx);
        let has_bot = s.active_bot().is_some();
        let (open, tab) = s.side_panel();
        let tab = tab.to_string();
        let panel_shown = open && has_bot;
        let docked = width - PANEL_WIDTH >= 480.;
        let panel_space = if docked && panel_shown { PANEL_WIDTH } else { 0. };
        let loading = s.chat_loading;
        let empty = s.rows.is_empty();
        let connected = s.connected;
        let error = s.error.clone();
        let streaming = s.streaming;
        let following = s.list.is_following_tail();
        let list_state = s.list.clone();
        let chat_key = s.active_bot.clone();

        let rows = RowContext { state: self.state.clone(), chat: cx.entity().downgrade(), edit_input: self.edit_input.clone() };
        let conversation = list(list_state, move |ix, window, cx| render_row(ix, &rows, window, cx))
            .size_full();
        // Each chat fades in as it opens, so switching bots reads as a change
        // of place rather than a jump.
        let conversation = motion::rise_in(div().size_full().child(conversation), id("conversation-in", &chat_key), 6., motion::SLOW);

        let jump = (!following && !empty && !loading).then(|| {
            let st = self.state.clone();
            let hover = c.raised;
            let button = div().absolute().bottom(px(16.)).left_0().right_0().flex().justify_center().child(
                ident("jump-down")
                    .size(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(c.radius)
                    .bg(c.surface)
                    .hover(move |s| s.bg(hover))
                    .border_1()
                    .border_color(c.border)
                    .cursor_pointer()
                    .text_size(px(18.))
                    .text_color(if streaming { c.accent } else { c.foreground })
                    .child("↓")
                    .on_click(move |_, _, cx| {
                        st.read(cx).scroll_to_end();
                        cx.refresh_windows();
                    }),
            );
            button.with_animation("jump-down-in", Animation::new(motion::QUICK).with_easing(motion::ease_out), |d, t| {
                d.opacity(t).bottom(px(16. - 10. * (1. - t)))
            })
        });

        let empty_home = empty && !loading && has_bot;
        let conversation_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(!empty_home && !loading, |d| d.child(conversation))
            .when(loading, |d| d.child(loading_transcript(cx)))
            .when(empty_home, |d| {
                d.flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(20.))
                    .px(px(24.))
                    .child(self.render_empty(cx))
                    .child(div().w_full().max_w(px(CHAT_COLUMN)).child(self.composer.clone()))
            })
            .children(jump);

        // Errors stay until dismissed or the next action clears them.
        let error_bar = (!error.is_empty()).then(|| {
            let st = self.state.clone();
            let bar = div().w_full().px(px(24.)).pb(px(8.)).flex().justify_center().child(
                div()
                    .w_full()
                    .pl(px(14.))
                    .pr(px(10.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(c.radius)
                    .bg(c.danger.alpha(0.12))
                    .border_1()
                    .border_color(c.danger.alpha(0.5))
                    .child(div().flex_1().text_size(px(13.)).child(error.clone()))
                    .child(
                        ident("dismiss-error")
                            .text_color(c.muted)
                            .cursor_pointer()
                            .child("✕")
                            .on_click(move |_, _, cx| st.update(cx, |s, cx| s.set_error("", cx))),
                    ),
            );
            motion::rise_in(bar, "error-in", 8., motion::BASE)
        });

        let composer = (connected && !empty_home).then(|| {
            div()
                .w_full()
                .px(px(24.))
                .pt(px(8.))
                .pb(px(20.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .w_full()
                        .max_w(px(CHAT_COLUMN))
                        .child(
                            div()
                                .w_full()
                                .pt(px(12.))
                                .child(self.composer.clone()),
                        ),
                )
        });

        let chat_area = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(conversation_area)
            .children(error_bar)
            .children(composer);

        let this = cx.entity().downgrade();
        let on_expand = move |expand: bool, _: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |this, cx| {
                this.screen_expanded = expand;
                this.sync_screen(cx);
                cx.notify();
            });
        };

        let panel = panel_shown.then(|| {
            let content = if tab == "screen" {
                side_panel::screen_panel(&self.state, &self.screen, self.screen_expanded, on_expand.clone(), cx)
            } else {
                side_panel::jobs_panel(&self.state, &self.jobs_expanded, cx)
            };
            let panel_width = if docked { PANEL_WIDTH } else { PANEL_WIDTH.min(width - 40.) };
            ident("side-panel")
                .absolute()
                .top(px(HEADER_HEIGHT))
                .bottom_0()
                .right_0()
                .w(px(panel_width))
                .occlude()
                .bg(c.surface)
                .border_l_1()
                .border_color(c.border)
                .when(!docked, |d| d.shadow_lg())
                .child(content)
                // Slides in from the window's edge.
                .with_animation("side-panel-in", Animation::new(motion::SLOW).with_easing(motion::ease_out), move |d, t| {
                    d.right(px(-panel_width * 0.35 * (1. - t))).opacity(t)
                })
        });

        let expanded_screen = (self.screen_expanded && has_bot).then(|| {
            let collapse = on_expand.clone();
            ident("expanded-screen")
                .absolute()
                .top(px(HEADER_HEIGHT))
                .left_0()
                .bottom_0()
                .right(px(panel_space))
                .occlude()
                .bg(black())
                .child(self.screen.clone())
                .child(
                    ident("collapse-screen")
                        .absolute()
                        .top(px(12.))
                        .right(px(12.))
                        .h(px(30.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .rounded(c.radius)
                        .bg(c.surface.alpha(0.75))
                        .hover(|s| s.opacity(0.95))
                        .border_1()
                        .border_color(c.border)
                        .cursor_pointer()
                        .text_size(px(13.))
                        .child("Collapse screen")
                        .on_click(move |_, w, cx| collapse(false, w, cx)),
                )
                .with_animation("expanded-screen-in", Animation::new(motion::BASE).with_easing(motion::ease_out), |d, t| d.opacity(t))
        });

        let drop_target = c.accent;
        ident("chat-view")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .when(has_bot, |d| d.child(self.render_header(cx)))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(chat_area)
                    // Holds the docked panel's place; the chat column eases
                    // aside as the panel opens and back as it closes.
                    .child(div().flex_none().with_spring(
                        "panel-space",
                        SpringAnimation::new(motion::settle()).to(px(panel_space)),
                        |d, w| d.w(w.max(px(0.))),
                    )),
            )
            .children(expanded_screen)
            .children(panel)
            // Drop files anywhere on the chat to attach them.
            .when(connected && has_bot, |d| {
                d.drag_over::<ExternalPaths>(move |style, _, _, _| style.border_2().border_color(drop_target))
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                        this.state.update(cx, |s, cx| s.add_files(paths.paths(), cx));
                    }))
            })
    }
}
