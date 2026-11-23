//! The window: the bots sidebar beside the open chat, with the dialogs on
//! top.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::chat::{ChatView, SIDEBAR_WIDTH};
use super::dialogs::{ConfirmModel, RequestDialog, SetupDialog, render_confirm};
use super::edit_bot::EditBotDialog;
use super::motion;
use super::state::{AppEvent, AppState};
use super::theme::colors;
use super::widgets::{self, ident, Action, MenuItem, PopupMenu, id};

pub struct RootView {
    pub(crate) state: Entity<AppState>,
    pub(crate) chat: Entity<ChatView>,
    pub(crate) setup: Entity<SetupDialog>,
    pub(crate) request: Entity<RequestDialog>,
    pub(crate) edit: Entity<EditBotDialog>,
    confirm: Option<ConfirmModel>,
    menu: Option<PopupMenu>,
    title: String,
    _subscriptions: Vec<Subscription>,
}

impl RootView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(AppState::new);
        Self::with_state(state, window, cx)
    }

    pub fn with_state(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let setup = cx.new(|cx| SetupDialog::new(state.clone(), window, cx));
        let request = cx.new(|cx| RequestDialog::new(state.clone(), window, cx));
        let edit = cx.new(|cx| EditBotDialog::new(state.clone(), window, cx));
        let edit_for_chat = edit.clone();
        let chat = cx.new(|cx| {
            ChatView::new(
                state.clone(),
                move |bot, window, cx| edit_for_chat.update(cx, |d, cx| d.show(&bot, window, cx)),
                window,
                cx,
            )
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.observe(&setup, |_, _, cx| cx.notify()),
            cx.observe(&request, |_, _, cx| cx.notify()),
            cx.observe(&edit, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, window, cx| match event {
                AppEvent::ServerRequest { id, method, params } => {
                    let (id, method, params) = (id.clone(), method.clone(), params.clone());
                    this.request.update(cx, |d, cx| d.show(id, method, params, window, cx));
                }
                AppEvent::ModelConfirm { message, model, provider } => {
                    this.confirm = Some(ConfirmModel { message: message.clone(), model: model.clone(), provider: provider.clone() });
                    cx.notify();
                }
                _ => {}
            }),
        ];
        if state.read(cx).servers().is_empty() {
            setup.update(cx, |d, cx| d.show(window, cx));
        }
        RootView { state, chat, setup, request, edit, confirm: None, menu: None, title: String::new(), _subscriptions: subscriptions }
    }

    fn close_menu(&self, cx: &mut Context<Self>) -> Action {
        let this = cx.entity().downgrade();
        Rc::new(move |_, cx| {
            let _ = this.update(cx, |this, cx| {
                this.menu = None;
                cx.notify();
            });
        })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let rows: Vec<_> = s
            .bots
            .iter()
            .map(|bot| {
                let active = bot.name == s.active_bot;
                let busy = s.is_busy(&bot.name);
                let subtitle = if busy {
                    "Working…".to_string()
                } else {
                    [&bot.preview, &bot.description, &bot.model].into_iter().find(|t| !t.is_empty()).cloned().unwrap_or_default()
                };
                let (name, edit_name) = (bot.name.clone(), bot.name.clone());
                let state = self.state.clone();
                let edit = self.edit.clone();
                let hover = c.raised.alpha(0.5);
                ident(id("bot", &bot.name))
                    .relative()
                    .h(px(60.))
                    .mx(px(8.))
                    .pl(px(12.))
                    .pr(px(8.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(12.))
                    .rounded(c.radius)
                    // The highlight fades in under the picked bot while its
                    // accent bar grows out from the middle.
                    .when(active, |d| {
                        d.child(div().absolute().inset_0().rounded(c.radius).bg(c.raised).with_animation(
                            id("bot-active-bg", &bot.name),
                            Animation::new(motion::BASE).with_easing(motion::ease_out),
                            |d, t| d.opacity(t),
                        ))
                        .child(div().absolute().left_0().w(px(3.)).bg(c.accent).with_animation(
                            id("bot-active-bar", &bot.name),
                            Animation::new(motion::SLOW).with_easing(motion::ease_out),
                            |d, t| {
                                let inset = 30. * (1. - t);
                                d.top(px(inset)).bottom(px(inset))
                            },
                        ))
                    })
                    .when(!active, |d| {
                        d.hover(move |s| s.bg(hover)).cursor_pointer().on_click(move |_, _, cx| {
                            state.update(cx, |s, cx| s.select_bot(&name, cx));
                        })
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            let (edit, bot) = (edit.clone(), edit_name.clone());
                            this.menu = Some(PopupMenu::new(
                                e.position,
                                vec![MenuItem::new("Edit profile…", move |window, cx| {
                                    edit.update(cx, |d, cx| d.show(&bot, window, cx));
                                })],
                            ));
                            cx.notify();
                        }),
                    )
                    .child(widgets::avatar(&bot.display_name, bot.avatar.clone(), bot.avatar_face, 38.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(15.))
                                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                                    .child(bot.display_name.clone()),
                            )
                            .when(!subtitle.is_empty(), |d| d.child(div().truncate().text_size(px(13.)).text_color(c.muted).child(subtitle))),
                    )
                    // Pulses while the bot is answering.
                    .when(busy, |d| d.child(widgets::pulse_dot(id("busy", &bot.name), c.accent, 8.)))
            })
            .collect();
        let empty = rows.is_empty() && !s.chat_loading && s.connected;
        let connecting = s.status.starts_with("Connecting") || s.status.starts_with("Reconnecting");
        // Until the roster arrives, the shape of a few bots holds its place.
        let placeholders = (rows.is_empty() && s.chat_loading).then(|| {
            div().flex().flex_col().gap(px(2.)).children((0..3).map(|i| {
                div()
                    .h(px(60.))
                    .mx(px(8.))
                    .pl(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(motion::skeleton_circle(id("bot-loading-avatar", i), 38., &c))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(7.))
                            .child(motion::skeleton(id("bot-loading-name", i), px(110. - 18. * i as f32), 12., &c))
                            .child(motion::skeleton(id("bot-loading-preview", i), relative(0.85), 10., &c)),
                    )
            }))
        });
        let server_name = s.active_server().map(|sv| sv.name.clone()).unwrap_or_else(|| "No server".into());
        let server_status = if s.connected { "Connected".to_string() } else { s.status.clone() };
        let dot = if s.connected {
            hsla(129. / 360., 0.49, 0.48, 1.)
        } else if connecting {
            hsla(39. / 360., 0.73, 0.48, 1.)
        } else {
            c.danger
        };
        let servers: Vec<_> = s.servers().iter().map(|sv| (sv.id.clone(), sv.name.clone())).collect();
        let active_server = s.config.active_server.clone();
        let (connected, streaming) = (s.connected, s.streaming);
        let hover = c.raised;

        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(c.surface)
            .border_r_1()
            .border_color(c.border)
            .child(
                div()
                    .pl(px(16.))
                    .pt(px(18.))
                    .pb(px(8.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c.muted)
                    .child("BOTS"),
            )
            .child(
                ident("bot-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(rows)
                    .children(placeholders)
                    .when(empty, |d| d.child(widgets::muted("No bots on this server", 14., &c).pt(px(40.)).text_center())),
            )
            .child(
                ident("server-switcher")
                    .h(px(56.))
                    .flex_none()
                    .border_t_1()
                    .border_color(c.border)
                    .p(px(8.))
                    .child(
                        div()
                            .size_full()
                            .pl(px(10.))
                            .pr(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .rounded(c.radius)
                            .hover(move |s| s.bg(hover))
                            .child(if connecting {
                                motion::breathe(div().size(px(8.)).rounded_full().bg(dot), "server-dot", 0.25).into_any_element()
                            } else {
                                div().size(px(8.)).rounded_full().bg(dot).into_any_element()
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(server_name))
                                    .child(div().truncate().text_size(px(12.)).text_color(c.muted).child(server_status)),
                            )
                            .child(div().text_size(px(18.)).text_color(c.muted).child("⋯")),
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        let mut items: Vec<MenuItem> = servers
                            .iter()
                            .map(|(sid, name)| {
                                let (state, sid2) = (this.state.clone(), sid.clone());
                                let is_active = active_server.as_deref() == Some(sid.as_str());
                                MenuItem::new(name.clone(), move |_, cx| {
                                    if !is_active {
                                        state.update(cx, |s, cx| s.select_server(&sid2, cx));
                                    }
                                })
                                .checked(is_active)
                                .enabled(!streaming)
                            })
                            .collect();
                        if !items.is_empty() {
                            items.push(MenuItem::separator());
                            if !connected {
                                let state = this.state.clone();
                                items.push(MenuItem::new("Reconnect", move |_, cx| state.update(cx, |s, cx| s.reconnect(cx))));
                            }
                        }
                        let setup = this.setup.clone();
                        items.push(MenuItem::new("Manage servers…", move |window, cx| setup.update(cx, |d, cx| d.show(window, cx))));
                        this.menu = Some(PopupMenu::new(e.position(), items).upward());
                        cx.notify();
                    })),
            )
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let title = match self.state.read(cx).active_bot() {
            Some(bot) => format!("{} — Hermes Bot", bot.display_name),
            None => "Hermes Bot".to_string(),
        };
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        let close_menu = self.close_menu(cx);
        let (yes, no) = (cx.entity().downgrade(), cx.entity().downgrade());
        let confirm = self.confirm.as_ref().map(|confirm| {
            let (model, provider) = (confirm.model.clone(), confirm.provider.clone());
            let state = self.state.clone();
            render_confirm(
                confirm,
                move |_, cx| {
                    state.update(cx, |s, cx| s.set_model(&model, &provider, true, cx));
                    let _ = yes.update(cx, |this, cx| {
                        this.confirm = None;
                        cx.notify();
                    });
                },
                move |_, cx| {
                    let _ = no.update(cx, |this, cx| {
                        this.confirm = None;
                        cx.notify();
                    });
                },
                cx,
            )
        });
        let sidebar = self.render_sidebar(cx).into_any_element();
        ident("root")
            .relative()
            .size_full()
            .flex()
            .bg(c.background)
            .text_color(c.foreground)
            .text_size(px(14.))
            .child(sidebar)
            .child(self.chat.clone())
            .when(self.setup.read(cx).open, |d| d.child(self.setup.clone()))
            .when(self.edit.read(cx).open, |d| d.child(self.edit.clone()))
            .when(self.request.read(cx).open, |d| d.child(self.request.clone()))
            .children(confirm)
            .when_some(self.menu.as_ref(), |d, menu| d.child(menu.render(close_menu, cx)))
    }
}
