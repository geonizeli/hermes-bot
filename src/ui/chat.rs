//! The open Bot Chat: a header, the conversation in a centered column, and
//! the composer. The side panel docks beside the chat when there's room and
//! floats over it in a narrow window.

use std::collections::HashSet;
use std::time::Duration;

use chrono::{Local, TimeZone};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::composer::Composer;
use super::markdown_view;
use super::screen::ScreenView;
use super::side_panel;
use super::state::{AppState, Role, Row};
use super::theme::colors;
use super::tool_card;
use super::widgets::{self, accent_surface, id, mono};

pub const SIDEBAR_WIDTH: f32 = 280.;
const HEADER_HEIGHT: f32 = 56.;
const PANEL_WIDTH: f32 = 320.;
const COLUMN_MAX: f32 = 780.;

pub struct ChatView {
    state: Entity<AppState>,
    composer: Entity<Composer>,
    screen: Entity<ScreenView>,
    screen_expanded: bool,
    /// The bot whose screen is connected ("" when none).
    run_key: String,
    jobs_expanded: Entity<HashSet<String>>,
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
            on_edit_bot: std::rc::Rc::new(on_edit_bot),
            _subscriptions: subscriptions,
            _poll: poll,
        }
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
            div()
                .id(id("toggle", name))
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
                div()
                    .id("bot-identity")
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
                    div()
                        .id("context-gauge")
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
                                    .w(relative(fill))
                                    .rounded(px(1.5))
                                    .bg(if u.percent > 85.0 { c.danger } else { c.accent }),
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
        let title = match bot {
            Some(b) => b.display_name.clone(),
            None if s.connected => "Pick a bot".into(),
            None => "Not connected".into(),
        };
        let subtitle = match bot {
            Some(b) if !b.description.is_empty() => b.description.clone(),
            Some(_) => "Say hello to start your chat.".into(),
            None => s.status.clone(),
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .px(px(24.))
            .child(
                div()
                    .w_full()
                    .max_w(px(COLUMN_MAX))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .when_some(bot, |d, b| d.child(widgets::avatar(&b.display_name, b.avatar.clone(), b.avatar_face, 64.)))
                    .child(div().text_size(px(28.)).font_weight(FontWeight::SEMIBOLD).truncate().child(title))
                    .child(widgets::muted(subtitle, 15., c).text_center()),
            )
    }
}

fn render_user(ix: usize, row: &Row, state: &Entity<AppState>, composer: &WeakEntity<Composer>, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let s = state.read(cx);
    let editable = s.last_user_index == Some(ix) && !s.streaming;
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
    let text = row.text.clone();
    let composer = composer.clone();
    let actions = div()
        .flex()
        .gap(px(12.))
        .opacity(0.)
        .group_hover(group.clone(), |s| s.opacity(1.))
        .when(row.timestamp > 0.0, |d| d.child(widgets::muted(time_label(row.timestamp), 12., &c)))
        .when(editable, |d| {
            d.child(widgets::text_link(id("edit", row.id), "Edit", &c).on_click(move |_, window, cx| {
                let _ = composer.update(cx, |composer, cx| composer.start_edit(&text, window, cx));
            }))
        });
    let bubble = (!row.text.is_empty()).then(|| {
        div()
            .max_w(relative(0.8))
            .px(px(16.))
            .py(px(11.))
            .rounded(c.radius)
            .bg(accent_surface(&c))
            .border_1()
            .border_color(c.accent.alpha(0.3))
            .text_size(px(15.))
            .line_height(relative(1.45))
            .child(row.text.clone())
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
        let el = div()
            .id(id("reasoning-toggle", row.id))
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
            el.with_animation(
                id("thinking", row.id),
                Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.4, 1.0)),
                |el, delta| el.opacity(delta),
            )
            .into_any_element()
        } else {
            el.into_any_element()
        }
    });
    let reasoning = (row.show_reasoning && !row.reasoning.is_empty()).then(|| {
        div()
            .border_l_2()
            .border_color(c.magenta.alpha(0.6))
            .pl(px(14.))
            .text_size(px(14.))
            .text_color(c.muted)
            .child(row.reasoning.clone())
    });
    let body = (!row.text.is_empty()).then(|| markdown_view::render(row.id, &row.text, state, window, cx));
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
                    st.update(cx, |s, cx| s.resend_last(&text, cx));
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
        .child(div().text_color(c.accent).child(if row.done { row.name.clone() } else { format!("{}  · running…", row.name) }))
        .when(!row.text.is_empty(), |d| d.child(div().text_color(c.foreground).child(row.text.clone())))
        .into_any_element()
}

/// The dots shown while waiting for the first word of an answer.
fn waiting_dots(cx: &App) -> impl IntoElement {
    let c = colors(cx);
    div().pt(px(14.)).flex().gap(px(6.)).children((0..3).map(|i| {
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
    }))
}

fn render_row(ix: usize, state: &Entity<AppState>, composer: &WeakEntity<Composer>, window: &mut Window, cx: &mut App) -> AnyElement {
    let (row, count, waiting) = {
        let s = state.read(cx);
        let Some(row) = s.rows.get(ix).cloned() else { return div().into_any_element() };
        (row, s.rows.len(), s.waiting_for_answer())
    };
    let c = colors(cx).clone();
    let content = match row.role {
        Role::User => render_user(ix, &row, state, composer, cx),
        Role::Assistant => render_assistant(ix, &row, state, window, cx),
        Role::Tool => tool_card::render(ix, &row, state, cx),
        Role::Todos => tool_card::render_todos(&row, cx),
        Role::Error => div().text_size(px(14.)).text_color(c.danger).child(row.text.clone()).into_any_element(),
        Role::Command => render_command(&row, window, cx),
        Role::Info => div().text_size(px(13.)).italic().text_color(c.muted).child(row.text.clone()).into_any_element(),
    };
    let last = ix + 1 == count;
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
                .max_w(px(COLUMN_MAX))
                .flex()
                .flex_col()
                .child(content)
                .when(last && waiting, |d| d.child(waiting_dots(cx))),
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

        let state = self.state.clone();
        let composer = self.composer.downgrade();
        let conversation = list(list_state, move |ix, window, cx| render_row(ix, &state, &composer, window, cx))
            .size_full();

        let jump = (!following && !empty && !loading).then(|| {
            let st = self.state.clone();
            let hover = c.raised;
            div().absolute().bottom(px(16.)).left_0().right_0().flex().justify_center().child(
                div()
                    .id("jump-down")
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
            )
        });

        let conversation_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .when(!loading, |d| d.child(conversation))
            .when(loading, |d| d.child(div().absolute().inset_0().flex().items_center().justify_center().child(widgets::spinner(40.))))
            .when(empty && !loading, |d| d.child(self.render_empty(cx)))
            .children(jump);

        // Errors stay until dismissed or the next action clears them.
        let error_bar = (!error.is_empty()).then(|| {
            let st = self.state.clone();
            div().w_full().px(px(24.)).pb(px(8.)).flex().justify_center().child(
                div()
                    .w_full()
                    .max_w(px(COLUMN_MAX))
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
                        div()
                            .id("dismiss-error")
                            .text_color(c.muted)
                            .cursor_pointer()
                            .child("✕")
                            .on_click(move |_, _, cx| st.update(cx, |s, cx| s.set_error("", cx))),
                    ),
            )
        });

        let composer = connected.then(|| {
            div()
                .w_full()
                .px(px(24.))
                .pt(px(8.))
                .pb(px(20.))
                .flex()
                .justify_center()
                .child(div().w_full().max_w(px(COLUMN_MAX)).child(self.composer.clone()))
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
            div()
                .id("side-panel")
                .absolute()
                .top(px(HEADER_HEIGHT))
                .bottom_0()
                .right_0()
                .w(px(if docked { PANEL_WIDTH } else { PANEL_WIDTH.min(width - 40.) }))
                .occlude()
                .bg(c.surface)
                .border_l_1()
                .border_color(c.border)
                .when(!docked, |d| d.shadow_lg())
                .child(content)
        });

        let expanded_screen = (self.screen_expanded && has_bot).then(|| {
            let collapse = on_expand.clone();
            div()
                .id("expanded-screen")
                .absolute()
                .top(px(HEADER_HEIGHT))
                .left_0()
                .bottom_0()
                .right(px(panel_space))
                .occlude()
                .bg(black())
                .child(self.screen.clone())
                .child(
                    div()
                        .id("collapse-screen")
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
        });

        let drop_target = c.accent;
        div()
            .id("chat-view")
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
                    // Holds the docked panel's place.
                    .child(div().w(px(panel_space)).flex_none()),
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
