//! The window: the bots sidebar beside the open chat, with the dialogs on
//! top.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::chat::{ChatView, SIDEBAR_WIDTH};
use super::dialogs::{ConfirmModel, RequestDialog, SetupDialog, render_confirm};
use super::edit_bot::EditBotDialog;
use super::group::{GroupView, stacked_avatars};
use super::group_dialogs::{GroupDialog, Mode};
use super::motion;
use super::state::{AppEvent, AppState};
use super::theme::{self, colors};
use super::widgets::{self, ident, Action, MenuItem, PopupMenu, id};

pub struct RootView {
    pub(crate) state: Entity<AppState>,
    pub(crate) chat: Entity<ChatView>,
    pub(crate) setup: Entity<SetupDialog>,
    pub(crate) request: Entity<RequestDialog>,
    pub(crate) edit: Entity<EditBotDialog>,
    pub(crate) group: Entity<GroupView>,
    pub(crate) group_dialog: Entity<GroupDialog>,
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
        let group_dialog = cx.new(|cx| GroupDialog::new(state.clone(), window, cx));
        let root = cx.entity().downgrade();
        let group = cx.new(|cx| {
            let on_menu = std::rc::Rc::new(move |at: Point<Pixels>, _: &mut Window, cx: &mut App| {
                let _ = root.update(cx, |this, cx| {
                    this.menu = Some(this.group_menu(at, cx));
                    cx.notify();
                });
            });
            GroupView::new(state.clone(), on_menu, window, cx)
        });
        theme::set_appearance(window.appearance(), cx);
        let subscriptions = vec![
            cx.observe(&group_dialog, |_, _, cx| cx.notify()),
            // "Match system" follows the desktop's light/dark switch live.
            cx.observe_window_appearance(window, |_, window, cx| theme::set_appearance(window.appearance(), cx)),
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
        RootView { state, chat, setup, request, edit, group, group_dialog, confirm: None, menu: None, title: String::new(), _subscriptions: subscriptions }
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

    /// The themes to pick from, the one in effect checked.
    fn theme_menu(&self, at: Point<Pixels>, cx: &App) -> PopupMenu {
        let theme = theme::theme_state(cx);
        let current = theme.effective();
        let items = theme
            .choices()
            .into_iter()
            .map(|choice| {
                let state = self.state.clone();
                MenuItem::new(choice.label(), move |_, cx| state.update(cx, |s, cx| s.set_theme(choice, cx))).checked(choice == current)
            })
            .collect();
        PopupMenu::new(at, items).upward()
    }

    /// The open group's options.
    fn group_menu(&self, at: Point<Pixels>, cx: &App) -> PopupMenu {
        let room_id = self.state.read(cx).active_group.clone().unwrap_or_default();
        self.room_menu(&room_id, at, cx)
    }

    /// A group's options: Rename and Disband, when the server offers them.
    fn room_menu(&self, room_id: &str, at: Point<Pixels>, cx: &App) -> PopupMenu {
        let s = self.state.read(cx);
        let mut items = Vec::new();
        if s.groups.offers("groups.rename") {
            let (dialog, room_id) = (self.group_dialog.clone(), room_id.to_string());
            items.push(MenuItem::new("Rename…", move |window, cx| {
                dialog.update(cx, |d, cx| d.show(Mode::Rename { room_id: room_id.clone() }, window, cx))
            }));
        }
        if s.groups.offers("groups.disband") {
            let (dialog, room_id) = (self.group_dialog.clone(), room_id.to_string());
            items.push(MenuItem::new("Disband…", move |window, cx| {
                dialog.update(cx, |d, cx| d.show(Mode::Disband { room_id: room_id.clone() }, window, cx))
            }));
        }
        PopupMenu::new(at, items)
    }

    /// The server's groups, under the bots.
    fn render_groups(&self, cx: &mut Context<Self>) -> Option<Div> {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        if !s.connected || !s.groups.caps.as_ref().is_some_and(|caps| caps.usable()) {
            return None;
        }
        let can_create = s.groups.offers("groups.create");
        let rows: Vec<_> = s
            .groups
            .rooms
            .iter()
            .map(|room| {
                let active = s.active_group.as_deref() == Some(room.room_id.as_str());
                let busy = active && s.group.driver.busy();
                let names: Vec<String> = room.members.iter().map(|m| s.member_label(m)).collect();
                let subtitle = if busy {
                    "Working…".to_string()
                } else {
                    s.groups.previews.get(&room.room_id).cloned().unwrap_or_else(|| names.join(", "))
                };
                let room_id = room.room_id.clone();
                let menu_room = room.room_id.clone();
                let has_menu = s.groups.offers("groups.rename") || s.groups.offers("groups.disband");
                let state = self.state.clone();
                let hover = c.raised.alpha(0.5);
                ident(id("group", &room.room_id))
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
                    .when(active, |d| {
                        d.child(div().absolute().inset_0().rounded(c.radius).bg(c.raised).with_animation(
                            id("group-active-bg", &room.room_id),
                            Animation::new(motion::BASE).with_easing(motion::ease_out),
                            |d, t| d.opacity(t),
                        ))
                        .child(div().absolute().left_0().w(px(3.)).bg(c.accent).with_animation(
                            id("group-active-bar", &room.room_id),
                            Animation::new(motion::SLOW).with_easing(motion::ease_out),
                            |d, t| {
                                let inset = 30. * (1. - t);
                                d.top(px(inset)).bottom(px(inset))
                            },
                        ))
                    })
                    .when(!active, |d| {
                        d.hover(move |s| s.bg(hover)).cursor_pointer().on_click(move |_, _, cx| {
                            state.update(cx, |s, cx| s.select_group(&room_id, cx));
                        })
                    })
                    .when(has_menu, |d| {
                        d.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                this.menu = Some(this.room_menu(&menu_room, e.position, cx));
                                cx.notify();
                            }),
                        )
                    })
                    .child(div().w(px(38.)).flex().flex_none().justify_center().child(stacked_avatars(s, &room.members, 24., cx)))
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
                                    .child(room.name.clone()),
                            )
                            .when(!subtitle.is_empty(), |d| d.child(div().truncate().text_size(px(13.)).text_color(c.muted).child(subtitle))),
                    )
                    .when(busy, |d| d.child(widgets::pulse_dot(id("group-busy", &room.room_id), c.accent, 8.)))
            })
            .collect();
        let empty = rows.is_empty();
        let dialog = self.group_dialog.clone();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .pl(px(16.))
                        .pr(px(12.))
                        .pt(px(18.))
                        .pb(px(6.))
                        .flex()
                        .items_center()
                        .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(c.muted).child("GROUPS"))
                        .when(can_create, |d| {
                            d.child(widgets::glyph_button("new-group", "+", "New group", &c).on_click(move |_, window, cx| {
                                dialog.update(cx, |d, cx| d.show(Mode::New { picked: Vec::new() }, window, cx));
                            }))
                        }),
                )
                .children(rows)
                .when(empty, |d| d.child(widgets::muted("No groups yet", 13., &c).pl(px(16.)).pt(px(4.)))),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let rows: Vec<_> = s
            .bots
            .iter()
            .map(|bot| {
                let active = bot.name == s.active_bot && s.active_group.is_none();
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
        let groups = self.render_groups(cx);

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
                    .when(empty, |d| d.child(widgets::muted("No bots on this server", 14., &c).pt(px(40.)).text_center()))
                    .children(groups)
                    .child(div().h(px(12.)).flex_none()),
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
                        let (root, at) = (cx.entity().downgrade(), e.position());
                        items.push(MenuItem::new("Theme…", move |_, cx| {
                            let _ = root.update(cx, |this, cx| {
                                this.menu = Some(this.theme_menu(at, cx));
                                cx.notify();
                            });
                        }));
                        this.menu = Some(PopupMenu::new(e.position(), items).upward());
                        cx.notify();
                    })),
            )
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let s = self.state.read(cx);
        let in_group = s.active_group.is_some();
        let title = match (s.active_room(), s.active_bot()) {
            (Some(room), _) if in_group => format!("{} — Hermes Bot", room.name),
            (_, Some(bot)) => format!("{} — Hermes Bot", bot.display_name),
            _ => "Hermes Bot".to_string(),
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
        // Most compositors draw the window's frame; where they leave it to
        // the app (GNOME on Wayland) the window draws its own title bar,
        // shadow and resize edges.
        let client_decorated = matches!(window.window_decorations(), Decorations::Client { .. });
        let radii = gpui_kit::component::window_corner_radii(window, cx);
        let sidebar = self.render_sidebar(cx).rounded_bl(radii.bottom_left);
        let body = ident("root")
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .rounded_bl(radii.bottom_left)
            .rounded_br(radii.bottom_right)
            .flex()
            .bg(c.background)
            .text_color(c.foreground)
            .text_size(px(14.))
            .child(sidebar)
            .when(!in_group, |d| d.child(self.chat.clone()))
            .when(in_group, |d| d.child(self.group.clone()))
            .when(self.group_dialog.read(cx).open(), |d| d.child(self.group_dialog.clone()))
            .when(self.setup.read(cx).open, |d| d.child(self.setup.clone()))
            .when(self.edit.read(cx).open, |d| d.child(self.edit.clone()))
            .when(self.request.read(cx).open, |d| d.child(self.request.clone()))
            .children(confirm)
            .when_some(self.menu.as_ref(), |d, menu| d.child(menu.render(close_menu, cx)));
        let title_bar = client_decorated.then(|| {
            gpui_kit::component::TitleBar::new().child(
                div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(c.muted).child(self.title.clone()),
            )
        });
        gpui_kit::component::window_border().child(
            div().size_full().flex().flex_col().text_color(c.foreground).children(title_bar).child(body),
        )
    }
}
