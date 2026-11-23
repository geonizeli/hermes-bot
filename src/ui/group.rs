//! An open bot group: a header with the members and what each is doing,
//! the room's log as a conversation, and a message box. The gateway runs
//! the group; this view reads it every two seconds while it's shown.

use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::markdown_view;
use super::motion;
use super::state::{AppEvent, AppState};
use super::theme::colors;
use super::widgets::{self, accent_surface, id, ident};
use crate::backend::groups::{Entry, Member, Speaker};

const HEADER_HEIGHT: f32 = 56.;
/// Element and reading-order keys for group lines, clear of the bot chat's.
const KEY_BASE: u64 = 1 << 40;

/// Opens the group's menu (Rename, Disband) at a point.
pub type MenuOpener = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

pub struct GroupView {
    state: Entity<AppState>,
    pub(crate) input: Entity<TextareaState>,
    placeholder: String,
    on_menu: MenuOpener,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn time_label(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_opt(ts as i64, 0).single() else { return String::new() };
    if t.date_naive() == Local::now().date_naive() { t.format("%H:%M").to_string() } else { t.format("%b %-d, %H:%M").to_string() }
}

/// `45s`, `1m20s`, `1h05m`.
fn elapsed(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// A member's avatar from its bot, when the bot is on the roster.
fn member_avatar(state: &AppState, member: Option<&Member>, fallback: &str, size: f32) -> AnyElement {
    let profile = member.map(|m| if m.profile.is_empty() { m.member_id.clone() } else { m.profile.clone() }).unwrap_or_else(|| fallback.to_string());
    let name = member.map(|m| state.member_label(m)).unwrap_or_else(|| fallback.to_string());
    match state.bots.iter().find(|b| b.name == profile) {
        Some(bot) => widgets::avatar(&name, bot.avatar.clone(), bot.avatar_face, size),
        None => widgets::avatar(&name, None, false, size),
    }
}

/// The members' avatars, overlapping, for the header and the sidebar.
pub fn stacked_avatars(state: &AppState, members: &[Member], size: f32, cx: &App) -> AnyElement {
    let c = colors(cx);
    let shown: Vec<&Member> = members.iter().take(3).collect();
    let step = size * 0.6;
    let width = size + step * shown.len().saturating_sub(1) as f32;
    div()
        .relative()
        .flex_none()
        .w(px(width.max(size)))
        .h(px(size))
        .children(shown.into_iter().enumerate().map(|(i, m)| {
            div()
                .absolute()
                .left(px(step * i as f32))
                .top_0()
                .rounded_full()
                .bg(c.surface)
                .child(member_avatar(state, Some(m), &m.member_id, size))
        }))
        .into_any_element()
}

impl GroupView {
    pub fn new(state: Entity<AppState>, on_menu: MenuOpener, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 9).submit_on_enter(true));
        let weak = cx.entity().downgrade();
        state.read(cx).group.list.set_scroll_handler(move |_, _, cx| {
            let _ = weak.update(cx, |_, cx| cx.notify());
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }),
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, window, cx| match event {
                AppEvent::GroupOpened => this.focus(window, cx),
                AppEvent::GroupDraftBack(text) => {
                    let text = text.clone();
                    this.input.update(cx, |input, cx| {
                        if input.value().trim().is_empty() {
                            input.set_value(text, window, cx);
                        }
                        input.focus(window, cx);
                    });
                }
                _ => {}
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        // The gateway runs the group; read it while it's open, and the list
        // of groups now and then.
        let poll = cx.spawn(async move |this, cx| {
            let mut tick: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                tick += 1;
                let Ok(()) = this.update(cx, |this, cx| {
                    this.state.update(cx, |s, cx| {
                        if !s.connected {
                            return;
                        }
                        if s.active_group.is_some() {
                            s.poll_group(cx);
                        }
                        if tick % 15 == 0 {
                            s.refresh_groups(cx);
                        }
                    });
                }) else {
                    break;
                };
            }
        });
        GroupView { state, input, placeholder: String::new(), on_menu, _subscriptions: subscriptions, _poll: poll }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let blocked = self.state.read(cx).group_send().is_some();
        if text.trim().is_empty() || blocked {
            return;
        }
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |s, cx| s.send_to_group(&text, cx));
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let room = s.active_room().cloned().unwrap_or_default();
        let driver = &s.group.driver;
        let at = now();
        let chips: Vec<AnyElement> = room
            .members
            .iter()
            .map(|m| {
                let status = driver.activity_of(&m.member_id).map(|a| {
                    if a.queued {
                        "queued".to_string()
                    } else {
                        let tool = if a.tool.is_empty() { "working".to_string() } else { a.tool.clone() };
                        if a.started_at > 0.0 { format!("{tool} {}", elapsed(at - a.started_at)) } else { tool }
                    }
                });
                let working = driver.activity_of(&m.member_id).is_some_and(|a| !a.queued);
                ident(id("member-chip", &m.member_id))
                    .h(px(26.))
                    .pl(px(4.))
                    .pr(px(10.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .rounded(c.radius)
                    .border_1()
                    .border_color(if working { c.accent.alpha(0.6) } else { c.border })
                    .text_size(px(12.))
                    .child(member_avatar(s, Some(m), &m.member_id, 18.))
                    .child(s.member_label(m))
                    .when_some(status, |d, status| {
                        let label = div().text_color(if working { c.accent } else { c.muted }).child(status);
                        d.child(if working { motion::breathe(label, id("member-busy", &m.member_id), 0.5).into_any_element() } else { label.into_any_element() })
                    })
                    .into_any_element()
            })
            .collect();
        let can_stop = s.groups.offers("groups.stop") && (driver.busy() || s.group_send().is_some_and(|o| o.failed.is_empty()) || s.group.stopping.is_some());
        let has_menu = s.groups.offers("groups.rename") || s.groups.offers("groups.disband");
        let stopping = s.group.stopping.is_some();
        let state = self.state.clone();
        let on_menu = self.on_menu.clone();
        div()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .px(px(20.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(c.border)
            .child(stacked_avatars(s, &room.members, 28., cx))
            .child(div().flex_none().max_w(px(260.)).truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(room.name.clone()))
            .child(ident("member-chips").flex_1().min_w_0().overflow_x_hidden().flex().gap(px(6.)).children(chips))
            .when(can_stop, |d| {
                d.child(
                    widgets::button("group-stop", if stopping { "Stopping…" } else { "Stop" }, false, true, &c)
                        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.stop_group(cx))),
                )
            })
            .when(has_menu, |d| {
                d.child(
                    ident("group-menu")
                        .size(px(30.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(c.radius)
                        .hover(|s| s.opacity(0.8))
                        .cursor_pointer()
                        .text_size(px(18.))
                        .text_color(c.muted)
                        .tooltip(|window, cx| Tooltip::new("Group options").build(window, cx))
                        .child("⋯")
                        .on_click(move |e, window, cx| on_menu(e.position(), window, cx)),
                )
            })
    }

    fn render_empty(&self, cx: &App) -> impl IntoElement {
        let c = colors(cx);
        let s = self.state.read(cx);
        let room = s.active_room().cloned().unwrap_or_default();
        let names: Vec<String> = room.members.iter().map(|m| s.member_label(m)).collect();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .px(px(24.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .child(stacked_avatars(s, &room.members, 56., cx))
                    .child(div().text_size(px(28.)).font_weight(FontWeight::SEMIBOLD).child(room.name.clone()))
                    .child(widgets::muted(format!("Say something to {}. Mention a bot with @ to ask just them.", names.join(", ")), 15., c).text_center()),
            )
    }
}

/// One line of the group's log.
fn render_entry(ix: usize, state: &Entity<AppState>, window: &mut Window, cx: &mut App) -> AnyElement {
    let (entry, count, member, label) = {
        let s = state.read(cx);
        let Some(entry) = s.group.entries.get(ix).cloned() else { return div().into_any_element() };
        let member = s.group.member(&entry.member_id).cloned();
        let label = match &entry.speaker {
            Speaker::Member(id) => member.as_ref().map(|m| s.member_label(m)).unwrap_or_else(|| id.clone()),
            Speaker::Scheduled => "Scheduled".into(),
            _ => String::new(),
        };
        (entry, s.group.entries.len(), member, label)
    };
    let c = colors(cx).clone();
    let key = KEY_BASE + entry.seq as u64;
    let order = super::chat::reading_order(key);
    let content = match &entry.speaker {
        Speaker::You => render_yours(&entry, key, cx),
        Speaker::Notice => {
            let who = member.as_ref().map(|m| state.read(cx).member_label(m));
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(c.muted)
                .when_some(who, |d, who| d.child(div().font_weight(FontWeight::MEDIUM).child(who)))
                .child(entry.text.clone())
                .into_any_element()
        }
        Speaker::Member(_) | Speaker::Scheduled => {
            let avatar = member_avatar(state.read(cx), member.as_ref(), &label, 28.);
            let group = SharedString::from(format!("group-line-{}", entry.seq));
            div()
                .group(group.clone())
                .w_full()
                .flex()
                .gap(px(12.))
                .child(div().flex_none().pt(px(2.)).child(avatar))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(label))
                                .child(widgets::muted(time_label(entry.at), 12., &c)),
                        )
                        .child(ident(id("group-answer", entry.seq)).w_full().child(markdown_view::render(key, order + (1 << 12), &entry.text, state, window, cx)))
                        .child(
                            div()
                                .opacity(0.)
                                .group_hover(group, |s| s.opacity(1.))
                                .child(markdown_view::copy_link(id("group-copy", entry.seq), entry.text.clone(), window, cx)),
                        ),
                )
                .into_any_element()
        }
    };
    let last = ix + 1 == count;
    div()
        .w_full()
        .px(px(24.))
        .pt(px(if ix == 0 { 24. } else { 18. }))
        .when(last, |d| d.pb(px(16.)))
        .child(content)
        .into_any_element()
}

/// Your message, on the right.
fn render_yours(entry: &Entry, key: u64, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let group = SharedString::from(format!("group-yours-{}", entry.seq));
    div()
        .group(group.clone())
        .w_full()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(12.))
        .child(div().opacity(0.).group_hover(group, |s| s.opacity(1.)).child(widgets::muted(time_label(entry.at), 12., &c)))
        .child(
            ident(id("group-sent", entry.seq))
                .max_w(relative(0.8))
                .px(px(16.))
                .py(px(11.))
                .rounded(c.radius)
                .bg(accent_surface(&c))
                .border_1()
                .border_color(c.accent.alpha(0.3))
                .text_size(px(15.))
                .line_height(relative(1.45))
                .child(markdown_view::plain_text(id("group-sent-text", entry.seq), super::chat::reading_order(key), entry.text.clone(), cx)),
        )
        .into_any_element()
}

impl Render for GroupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let connected = s.connected;
        let loading = s.group.loading;
        let empty = s.group.entries.is_empty();
        let list_state = s.group.list.clone();
        let room_key = s.group.room_id.clone();
        let room_name = s.active_room().map(|r| r.name.clone()).unwrap_or_default();
        let can_send = s.groups.offers("groups.send");
        let outgoing = s.group_send().cloned();
        let error = s.error.clone();
        let driver = s.group.driver.clone();
        let read_error = s.group.read_error.clone();
        let answering: Vec<(String, AnyElement, String)> = driver
            .answering()
            .into_iter()
            .map(|a| {
                let member = s.group.member(&a.member_id);
                let name = member.map(|m| s.member_label(m)).unwrap_or_else(|| a.member_id.clone());
                let tool = if a.tool.is_empty() { String::new() } else { format!(" · {}", a.tool) };
                (a.member_id.clone(), member_avatar(s, member, &name, 22.), format!("{name} is working{tool}…"))
            })
            .collect();

        let placeholder = if !connected {
            "Not connected".to_string()
        } else if !can_send {
            "This server doesn't take group messages".to_string()
        } else {
            format!("Message {room_name}")
        };
        if placeholder != self.placeholder {
            self.placeholder = placeholder.clone();
            self.input.update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
        }

        let state = self.state.clone();
        let conversation = list(list_state, move |ix, window, cx| render_entry(ix, &state, window, cx)).size_full();
        let conversation = motion::rise_in(div().size_full().child(conversation), id("group-in", &room_key), 6., motion::SLOW);

        let conversation_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .when(!loading, |d| d.child(conversation))
            .when(loading, |d| {
                d.child(div().absolute().inset_0().p(px(24.)).flex().flex_col().gap(px(18.)).children((0..3).map(|i| {
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(motion::skeleton_circle(id("group-loading-avatar", i), 28., &c))
                        .child(div().flex_1().child(motion::skeleton_lines(&format!("group-loading-{i}"), 2 + i % 2, 14., &c)))
                })))
            })
            .when(empty && !loading && outgoing.is_none(), |d| d.child(self.render_empty(cx)));

        // What's under way: who's answering, and your message on its way.
        let typing = (!answering.is_empty()).then(|| {
            div().w_full().px(px(24.)).pb(px(6.)).flex().flex_col().gap(px(6.)).children(answering.into_iter().map(|(mid, avatar, line)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(avatar)
                    .child(motion::breathe(widgets::muted(line, 13., &c), id("group-typing", &mid), 0.45))
            }))
        });
        let sending = outgoing.map(|out| {
            let failed = !out.failed.is_empty();
            let st = self.state.clone();
            div()
                .w_full()
                .px(px(24.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .items_end()
                .gap(px(4.))
                .child(
                    ident("group-outgoing")
                        .max_w(relative(0.8))
                        .px(px(16.))
                        .py(px(11.))
                        .rounded(c.radius)
                        .bg(accent_surface(&c))
                        .border_1()
                        .border_color(if failed { c.danger.alpha(0.6) } else { c.accent.alpha(0.3) })
                        .opacity(0.7)
                        .text_size(px(15.))
                        .child(out.text.clone()),
                )
                .child(if failed {
                    div()
                        .flex()
                        .gap(px(10.))
                        .items_center()
                        .child(widgets::muted(format!("Not confirmed: {}", out.failed), 12., &c))
                        .child(widgets::text_link("group-retry", "Retry", &c).on_click(move |_, _, cx| st.update(cx, |s, cx| s.resend_to_group(cx))))
                        .into_any_element()
                } else {
                    widgets::muted("Sending…", 12., &c).into_any_element()
                })
        });

        // The room's own troubles, then errors from what you did.
        let mut notes: Vec<String> = Vec::new();
        if driver.missing && !loading && read_error.is_empty() {
            notes.push("The server's group driver is unavailable, so the bots won't answer until it's back.".into());
        }
        if driver.pending_actions > 0 {
            notes.push("A bot is waiting for an approval or a retry. Answer it in Hermes Desktop for now.".into());
        }
        if !driver.last_error.is_empty() {
            notes.push(format!("The group reported an error: {}", driver.last_error));
        }
        if !read_error.is_empty() {
            notes.push(read_error);
        }
        let notes = (!notes.is_empty()).then(|| {
            div().w_full().px(px(24.)).pb(px(8.)).flex().flex_col().gap(px(4.)).children(notes.into_iter().map(|n| widgets::muted(n, 13., &c)))
        });
        let error_bar = (!error.is_empty()).then(|| {
            let st = self.state.clone();
            div().w_full().px(px(24.)).pb(px(8.)).child(
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
                        ident("group-dismiss-error")
                            .text_color(c.muted)
                            .cursor_pointer()
                            .child("✕")
                            .on_click(move |_, _, cx| st.update(cx, |s, cx| s.set_error("", cx))),
                    ),
            )
        });

        let has_text = !self.input.read(cx).value().trim().is_empty();
        let send_ready = connected && can_send && has_text && self.state.read(cx).group_send().is_none();
        let focused = self.input.focus_handle(cx).is_focused(window);
        let composer = connected.then(|| {
            div().w_full().px(px(24.)).pt(px(8.)).pb(px(20.)).child(
                ident("group-composer")
                    .w_full()
                    .pl(px(16.))
                    .pr(px(10.))
                    .py(px(10.))
                    .flex()
                    .items_end()
                    .gap(px(10.))
                    .rounded(c.radius)
                    .bg(c.surface)
                    .border_1()
                    .border_color(if focused { c.accent.alpha(0.7) } else { c.border })
                    .child(div().flex_1().min_w_0().py(px(6.)).child(Textarea::new(&self.input).appearance(false).text_size(px(15.))))
                    .child(
                        ident("group-send")
                            .size(px(34.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(if send_ready { c.accent } else { c.raised })
                            .text_color(if send_ready { c.accent_foreground } else { c.muted })
                            .text_size(px(16.))
                            .child("↑")
                            .when(send_ready, |d| d.cursor_pointer().hover(|s| s.opacity(0.85)).on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))),
                    ),
            )
        });

        ident("group-view")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_header(cx))
            .child(conversation_area)
            .children(typing)
            .children(sending)
            .children(notes)
            .children(error_bar)
            .children(composer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn elapsed_reads_short() {
        assert_eq!(elapsed(45.4), "45s");
        assert_eq!(elapsed(80.), "1m20s");
        assert_eq!(elapsed(3900.), "1h05m");
    }
}
