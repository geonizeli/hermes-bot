//! The group dialogs: New group (pick bots and a name), Rename, and the
//! Disband confirmation. One entity, one dialog open at a time.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::state::{AppEvent, AppState};
use super::theme::colors;
use super::widgets::{self, heading, id, ident, modal, muted};
use crate::backend::groups::{MAX_MEMBERS, MIN_MEMBERS};

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    New { picked: Vec<String> },
    Rename { room_id: String },
    Disband { room_id: String },
}

pub struct GroupDialog {
    state: Entity<AppState>,
    pub mode: Option<Mode>,
    name: Entity<InputState>,
    busy: bool,
    message: String,
    _subscriptions: Vec<Subscription>,
}

impl GroupDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } => this.confirm(cx),
                _ => cx.notify(),
            }),
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, _, cx| {
                if let AppEvent::GroupActionDone(result) = event {
                    if !this.busy {
                        return;
                    }
                    this.busy = false;
                    match result {
                        Ok(()) => this.mode = None,
                        Err(e) => this.message = e.clone(),
                    }
                    cx.notify();
                }
            }),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        GroupDialog { state, mode: None, name, busy: false, message: String::new(), _subscriptions: subscriptions }
    }

    pub fn open(&self) -> bool {
        self.mode.is_some()
    }

    pub fn show(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let (value, placeholder) = match &mode {
            Mode::Rename { room_id } => (self.state.read(cx).room_name(room_id), "Group name"),
            _ => (String::new(), "Name (optional: the bots' names)"),
        };
        self.name.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.set_placeholder(placeholder, window, cx);
            input.focus(window, cx);
        });
        self.mode = Some(mode);
        self.busy = false;
        self.message.clear();
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.mode = None;
        self.busy = false;
        self.message.clear();
        cx.notify();
    }

    fn toggle(&mut self, profile: &str, cx: &mut Context<Self>) {
        if let Some(Mode::New { picked }) = self.mode.as_mut() {
            if let Some(i) = picked.iter().position(|p| p == profile) {
                picked.remove(i);
            } else if picked.len() < MAX_MEMBERS {
                picked.push(profile.to_string());
            }
            self.message.clear();
            cx.notify();
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let name = self.name.read(cx).value().to_string();
        let Some(mode) = self.mode.clone() else { return };
        match mode {
            Mode::New { picked } => {
                if picked.len() < MIN_MEMBERS {
                    self.message = format!("Pick {MIN_MEMBERS} to {MAX_MEMBERS} bots.");
                    cx.notify();
                    return;
                }
                self.busy = true;
                self.message.clear();
                self.state.update(cx, |s, cx| s.create_group(&name, picked, cx));
            }
            Mode::Rename { room_id } => {
                if name.trim().is_empty() {
                    return;
                }
                self.busy = true;
                self.state.update(cx, |s, cx| s.rename_group(&room_id, &name, cx));
            }
            Mode::Disband { room_id } => {
                self.busy = true;
                self.state.update(cx, |s, cx| s.disband_group(&room_id, cx));
            }
        }
        cx.notify();
    }

    fn render_bots(&self, picked: &[String], cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let bots = self.state.read(cx).seatable_bots();
        let full = picked.len() >= MAX_MEMBERS;
        ident("group-bot-picker")
            .max_h(px(320.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(bots.into_iter().map(|bot| {
                let on = picked.contains(&bot.name);
                let enabled = on || !full;
                let name = bot.name.clone();
                let hover = c.raised.alpha(0.6);
                ident(id("pick-bot", &bot.name))
                    .h(px(46.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(c.radius)
                    .when(on, |d| d.bg(c.raised))
                    .when(enabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _, _, cx| this.toggle(&name, cx))))
                    .when(!enabled, |d| d.opacity(0.5))
                    .child(
                        div()
                            .size(px(16.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(if on { c.accent } else { c.border })
                            .bg(if on { c.accent } else { transparent_black() })
                            .text_size(px(11.))
                            .text_color(c.accent_foreground)
                            .when(on, |d| d.child("✓")),
                    )
                    .child(widgets::avatar(&bot.display_name, bot.avatar.clone(), bot.avatar_face, 28.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_size(px(14.)).child(bot.display_name.clone()))
                            .when(!bot.description.is_empty(), |d| d.child(div().truncate().text_size(px(12.)).text_color(c.muted).child(bot.description.clone()))),
                    )
            }))
    }
}

impl Render for GroupDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let Some(mode) = self.mode.clone() else { return div().into_any_element() };
        let room_name = match &mode {
            Mode::Rename { room_id } | Mode::Disband { room_id } => self.state.read(cx).room_name(room_id),
            Mode::New { .. } => String::new(),
        };
        let (title, action, danger) = match &mode {
            Mode::New { .. } => ("New group".to_string(), "Create", false),
            Mode::Rename { .. } => ("Rename group".to_string(), "Rename", false),
            Mode::Disband { .. } => (format!("Disband {room_name}?"), "Disband", true),
        };
        let ready = !self.busy
            && match &mode {
                Mode::New { picked } => picked.len() >= MIN_MEMBERS,
                Mode::Rename { .. } => !self.name.read(cx).value().trim().is_empty(),
                Mode::Disband { .. } => true,
            };
        let body = match &mode {
            Mode::New { picked } => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(muted("The server runs the group, so the bots keep talking with the app closed.", 13., &c))
                .child(heading(format!("Bots ({} of {MIN_MEMBERS}–{MAX_MEMBERS})", picked.len()), &c))
                .child(self.render_bots(picked, cx))
                .child(heading("Name", &c))
                .child(Input::new(&self.name))
                .into_any_element(),
            Mode::Rename { .. } => div().child(Input::new(&self.name)).into_any_element(),
            Mode::Disband { .. } => muted(
                "The bots stop, and the group and its history go away for good. Their own chats aren't touched.",
                14.,
                &c,
            )
            .into_any_element(),
        };
        let message = (!self.message.is_empty()).then(|| div().text_size(px(13.)).text_color(c.danger).child(self.message.clone()));
        let content = div()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title))
            .child(body)
            .children(message)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(widgets::button("group-dialog-cancel", "Cancel", false, true, &c).on_click(cx.listener(|this, _, _, cx| this.close(cx))))
                    .child({
                        let label = if self.busy { "Working…".to_string() } else { action.to_string() };
                        let button = widgets::button("group-dialog-confirm", label, !danger, ready, &c)
                            .when(danger && ready, |d| d.bg(c.danger).text_color(white()));
                        button.when(ready, |d| d.on_click(cx.listener(|this, _, _, cx| this.confirm(cx))))
                    }),
            );
        modal("group-dialog", 460., content, &c).into_any_element()
    }
}
