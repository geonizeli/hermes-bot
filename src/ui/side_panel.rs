//! The chat's side panel: the bot's scheduled jobs (Hermes cron) or its
//! screen. Whether it's open, and on which tab, is remembered between runs.

use std::collections::HashSet;

use chrono::{DateTime, Local};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::screen::{ScreenState, ScreenView};
use super::state::AppState;
use super::theme::{Colors, colors};
use super::widgets::{self, ident, id, mono};

fn parse_time(iso: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(iso.trim())
        .ok()
        .map(|t| t.with_timezone(&Local))
        .or_else(|| {
            // Naive timestamps are UTC.
            chrono::NaiveDateTime::parse_from_str(iso.trim(), "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|n| n.and_utc().with_timezone(&Local))
        })
}

fn relative(iso: &str) -> String {
    let Some(t) = parse_time(iso) else { return String::new() };
    let seconds = (t - Local::now()).num_seconds();
    let abs = seconds.abs();
    let span = if abs < 60 {
        "less than a minute".to_string()
    } else if abs < 3600 {
        format!("{} min", (abs as f64 / 60.0).round())
    } else if abs < 86400 {
        format!("{} h", (abs as f64 / 3600.0).round())
    } else {
        format!("{} d", (abs as f64 / 86400.0).round())
    };
    if seconds >= 0 { format!("in {span}") } else { format!("{span} ago") }
}

fn absolute(iso: &str) -> String {
    parse_time(iso).map(|t| t.format("%b %-d, %H:%M").to_string()).unwrap_or_default()
}

/// Hermes schedules are intervals ("every 60m", "once in 2h") or cron
/// expressions; spell out the common shapes and pass the rest through.
pub fn schedule_label(s: &str) -> String {
    let s = s.trim();
    if let Some(min) = s.strip_prefix("every ").and_then(|r| r.strip_suffix('m')).and_then(|n| n.parse::<u64>().ok()) {
        return if min % 1440 == 0 {
            if min == 1440 { "Daily".into() } else { format!("Every {} days", min / 1440) }
        } else if min % 60 == 0 {
            if min == 60 { "Hourly".into() } else { format!("Every {} hours", min / 60) }
        } else {
            format!("Every {min} minutes")
        };
    }
    if let Some(rest) = s.strip_prefix("once in ") {
        return format!("Once, in {rest}");
    }
    if s.len() >= 2 && s[..s.len() - 1].chars().all(|c| c.is_ascii_digit()) && s.ends_with(['m', 'h', 'd']) {
        return format!("Once, in {s}");
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 5 && parts[2] == "*" && parts[3] == "*" {
        let (Ok(minute), hour) = (parts[0].parse::<u32>(), parts[1]) else { return s.to_string() };
        if hour == "*" && parts[4] == "*" {
            return format!("Hourly at :{minute:02}");
        }
        let Ok(hour) = hour.parse::<u32>() else { return s.to_string() };
        let time = format!("{hour:02}:{minute:02}");
        let dow = parts[4];
        if dow == "*" {
            return format!("Daily at {time}");
        }
        if dow == "1-5" {
            return format!("Weekdays at {time}");
        }
        const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        let name = |d: &str| d.parse::<usize>().ok().and_then(|n| NAMES.get(n)).copied();
        let days: Option<Vec<String>> = dow
            .split(',')
            .map(|d| match d.split_once('-') {
                Some((a, b)) => Some(format!("{}–{}", name(a)?, name(b)?)),
                None => name(d).map(str::to_string),
            })
            .collect();
        if let Some(days) = days {
            return format!("{} at {time}", days.join(", "));
        }
    }
    s.to_string()
}

fn status_text(status: &str) -> String {
    match status {
        "ok" => "Succeeded",
        "error" => "Failed",
        "delivery_failed" => "Ran, but delivery failed",
        "blocked_config" => "Blocked by configuration",
        other => other,
    }
    .to_string()
}

fn header(title: &'static str, busy: bool, on_refresh: impl Fn(&mut Window, &mut App) + 'static, on_close: impl Fn(&mut Window, &mut App) + 'static, c: &Colors) -> impl IntoElement {
    div()
        .h(px(48.))
        .flex_none()
        .pl(px(16.))
        .pr(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .border_b_1()
        .border_color(c.border)
        .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title))
        .when(busy, |d| d.child(widgets::spinner(16.)))
        .child(div().flex_1())
        .child(widgets::glyph_button(SharedString::from(format!("{title}-refresh")), "↻", "Refresh", c).on_click(move |_, w, cx| on_refresh(w, cx)))
        .child(widgets::glyph_button(SharedString::from(format!("{title}-hide")), "✕", "Hide", c).on_click(move |_, w, cx| on_close(w, cx)))
}

pub fn jobs_panel(state: &Entity<AppState>, expanded: &Entity<HashSet<String>>, cx: &App) -> AnyElement {
    let c = colors(cx).clone();
    let s = state.read(cx);
    let (refresh, close) = (state.clone(), state.clone());
    let open = expanded.read(cx).clone();
    let jobs: Vec<AnyElement> = s
        .jobs
        .iter()
        .enumerate()
        .map(|(ix, job)| {
            let text = |k: &str| job.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let key = Some(text("id")).filter(|k| !k.is_empty()).unwrap_or_else(|| ix.to_string());
            let is_open = open.contains(&key);
            let paused = job.get("paused").and_then(Value::as_bool).unwrap_or(false);
            let error = text("error");
            let last_status = text("lastStatus");
            let failing = !error.is_empty() || ["error", "delivery_failed", "blocked_config"].contains(&last_status.as_str());
            let next = text("nextRunAt");
            let last = text("lastRunAt");
            let details: Vec<(&str, String)> = [
                ("Next run", absolute(&next)),
                ("Last run", if last.is_empty() { "Never".into() } else { format!("{} · {}", relative(&last), absolute(&last)) }),
                ("Result", status_text(&last_status)),
                ("Delivers to", text("deliver")),
                ("Schedule", text("schedule")),
            ]
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .collect();
            let toggle = expanded.clone();
            let toggle_key = key.clone();
            let hover = c.raised;
            ident(id("job", &key))
                .mx(px(12.))
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .rounded(c.radius)
                .bg(if is_open { c.raised } else { c.raised.alpha(0.45) })
                .when(is_open, |d| d.border_1().border_color(c.border))
                .hover(move |s| s.bg(hover))
                .when(paused, |d| d.opacity(0.6))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    toggle.update(cx, |set, cx| {
                        if !set.remove(&toggle_key) {
                            set.insert(toggle_key.clone());
                        }
                        cx.notify();
                    })
                })
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().mt(px(6.)).size(px(8.)).flex_none().rounded_full().bg(if paused {
                            c.muted
                        } else if failing {
                            c.danger
                        } else {
                            c.green
                        }))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .line_clamp(if is_open { 6 } else { 2 })
                                .child(Some(text("title")).filter(|t| !t.is_empty()).unwrap_or_else(|| "Untitled job".into())),
                        ),
                )
                .child(widgets::muted(schedule_label(&text("schedule")), 12., &c).pl(px(16.)).truncate())
                .child(
                    div()
                        .pl(px(16.))
                        .text_size(px(12.))
                        .text_color(if paused { c.muted } else { c.accent })
                        .child(if paused {
                            "Paused".to_string()
                        } else if next.is_empty() {
                            "Not scheduled".to_string()
                        } else {
                            format!("Next {}", relative(&next))
                        }),
                )
                .when(is_open, |d| {
                    d.child(div().pl(px(16.)).pt(px(8.)).flex().flex_col().gap(px(4.)).children(details.into_iter().map(|(k, v)| {
                        div()
                            .flex()
                            .gap(px(10.))
                            .text_size(px(12.))
                            .child(div().w(px(76.)).flex_none().text_color(c.muted).child(k))
                            .child(div().flex_1().child(v))
                    })))
                    .when(!error.is_empty(), |d| d.child(div().pl(px(16.)).pt(px(4.)).text_size(px(12.)).text_color(c.danger).child(error.clone())))
                    .when(!text("prompt").is_empty(), |d| {
                        d.child(div().pl(px(16.)).pt(px(6.)).text_size(px(12.)).italic().text_color(c.muted).child(text("prompt")))
                    })
                })
                .into_any_element()
        })
        .collect();
    let empty = jobs.is_empty() && !s.jobs_loading && s.jobs_error.is_empty();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(header(
            "Scheduled jobs",
            s.jobs_loading,
            move |_, cx| refresh.update(cx, |s, cx| s.refresh_jobs(cx)),
            move |_, cx| close.update(cx, |s, cx| s.close_side_panel(cx)),
            &c,
        ))
        .when(!s.jobs_error.is_empty(), |d| {
            d.child(div().m(px(16.)).text_size(px(13.)).text_color(c.danger).child(format!("Couldn't load jobs: {}", s.jobs_error)))
        })
        .child(
            ident("jobs-list")
                .flex_1()
                .overflow_y_scroll()
                .py(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(jobs)
                .when(empty, |d| {
                    d.child(
                        div()
                            .px(px(24.))
                            .pt(px(40.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(6.))
                            .child(div().text_size(px(14.)).child("No scheduled jobs"))
                            .child(
                                widgets::muted("Ask the bot to do something on a schedule, and it will show up here.", 12., &c)
                                    .text_center(),
                            ),
                    )
                }),
        )
        .into_any_element()
}

/// What the screen panel says when there's no running screen to show.
fn screen_message(status: &Value, connected: bool) -> String {
    if !connected {
        return "Not connected.".into();
    }
    if status.is_null() {
        return "Checking the screen…".into();
    }
    let b = |k: &str| status.get(k).and_then(Value::as_bool);
    let text = |k: &str| status.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    if b("unavailable") == Some(true) {
        return "This Hermes server doesn't have bot screens. Update Hermes to use them.".into();
    }
    if !text("error").is_empty() {
        return format!("Couldn't check the screen: {}", text("error"));
    }
    if b("supported") == Some(false) {
        return "Bot screens need a Linux server.".into();
    }
    if b("installed") == Some(false) {
        let missing: Vec<String> = status
            .get("missing")
            .and_then(Value::as_array)
            .map(|m| m.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut out = "The screen isn't installed on the server yet.".to_string();
        if !missing.is_empty() {
            out.push_str(&format!(" Missing: {}.", missing.join(", ")));
        }
        return out;
    }
    if !text("blocker").is_empty() {
        return text("blocker");
    }
    if b("running") != Some(true) {
        return "The screen is off. The bot turns it on when it needs a browser or desktop app, or you can start it now.".into();
    }
    String::new()
}

/// `live` is the screen view to place in the panel, or `None` while it shows
/// over the chat.
pub fn screen_panel(
    state: &Entity<AppState>,
    screen: &Entity<ScreenView>,
    expanded: bool,
    on_expand: impl Fn(bool, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> AnyElement {
    let c = colors(cx).clone();
    let s = state.read(cx);
    let view = screen.read(cx);
    let status = s.screen.clone();
    let b = |k: &str| status.get(k).and_then(Value::as_bool);
    let text = |k: &str| status.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let running = b("running") == Some(true);
    let lease = status.get("lease").filter(|l| l.is_object());
    let bot_in_control = lease.is_none_or(|l| l.get("holder").and_then(Value::as_str) != Some("human"));
    let message = screen_message(&status, s.connected);
    let busy = s.screen_busy;
    let install_command = text("install_command");
    let not_installed = b("installed") == Some(false);
    let can_install = not_installed && b("supported") != Some(false) && {
        let placement = text("placement");
        placement.is_empty() || placement == "gateway"
    };
    let can_start = !status.is_null()
        && b("installed") != Some(false)
        && b("supported") != Some(false)
        && !running
        && b("unavailable") != Some(true)
        && text("error").is_empty();
    let (refresh, close) = (state.clone(), state.clone());
    let aspect = if view.frame_width > 0 { view.frame_height as f32 / view.frame_width as f32 } else { 0.625 };
    let state_line = match view.state {
        ScreenState::Connecting => "Connecting to the screen…".to_string(),
        ScreenState::Ended => if view.message.is_empty() { "Disconnected.".to_string() } else { view.message.clone() },
        _ if view.in_control => "You're in control. The bot waits until you give it back.".to_string(),
        _ if bot_in_control => "Watching. The bot is in control.".to_string(),
        _ => "Someone else is in control. Watching.".to_string(),
    };
    let in_control = view.in_control;
    let live = view.state == ScreenState::Live;
    let ended = view.state == ScreenState::Ended;
    let extra_message = (running && !ended && !view.message.is_empty()).then(|| view.message.clone());

    let body = ident("screen-body")
        .flex_1()
        .overflow_y_scroll()
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .when(!message.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(13.))
                    .text_color(if text("error").is_empty() { c.muted } else { c.danger })
                    .child(message.clone()),
            )
        })
        .when(not_installed && !install_command.is_empty(), |d| {
            d.child(
                div()
                    .p(px(8.))
                    .rounded(c.radius)
                    .bg(c.background)
                    .border_1()
                    .border_color(c.border)
                    .font_family(mono(cx))
                    .text_size(px(12.))
                    .child(install_command.clone()),
            )
        })
        .when(can_install, |d| {
            let st = state.clone();
            d.child(
                widgets::button("screen-install", if busy { "Install on the server…" } else { "Install on the server" }, true, !busy, &c)
                    .when(!busy, |b| b.on_click(move |_, _, cx| st.update(cx, |s, cx| s.install_screen(cx)))),
            )
        })
        .when(!s.screen_log.is_empty(), |d| {
            let tail: Vec<&str> = s.screen_log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
            d.child(div().font_family(mono(cx)).text_size(px(11.)).text_color(c.muted).child(tail.join("\n")))
        })
        .when(can_start, |d| {
            let st = state.clone();
            d.child(
                widgets::button("screen-start", if busy { "Starting…" } else { "Start screen" }, true, !busy, &c)
                    .when(!busy, |b| b.on_click(move |_, _, cx| st.update(cx, |s, cx| s.start_screen(cx)))),
            )
        })
        .when(running, |d| {
            let expand = on_expand.clone();
            let holder = ident("screen-holder")
                .w_full()
                .flex_none()
                .rounded(c.radius)
                .overflow_hidden()
                .bg(black())
                .map(|h| h.h(px(288. * aspect)));
            let holder = if expanded {
                holder
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(widgets::muted("Showing over the chat", 12., &c))
            } else {
                holder.child(screen.clone()).when(!in_control, |h| {
                    h.on_click(move |e, w, cx| {
                        if e.click_count() >= 2 {
                            expand(true, w, cx)
                        }
                    })
                })
            };
            let (take, reconnect, stop) = (screen.clone(), screen.clone(), state.clone());
            let expand = on_expand.clone();
            d.child(holder)
                .child(div().text_size(px(12.)).text_color(if in_control { c.accent } else { c.muted }).child(state_line.clone()))
                .children(extra_message.clone().map(|m| widgets::muted(m, 12., &c)))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            widgets::button("screen-expand", if expanded { "Collapse" } else { "Expand" }, false, true, &c)
                                .on_click(move |_, w, cx| expand(!expanded, w, cx)),
                        )
                        .child(
                            widgets::button("screen-control", if in_control { "Give back" } else { "Take control" }, !in_control, live, &c)
                                .when(live, |b| {
                                    b.on_click(move |_, window, cx| {
                                        take.update(cx, |v, cx| if v.in_control { v.give_back(cx) } else { v.take_control(window, cx) })
                                    })
                                }),
                        )
                        .when(ended, |d| {
                            d.child(
                                widgets::button("screen-reconnect", "Reconnect", false, true, &c)
                                    .on_click(move |_, _, cx| reconnect.update(cx, |v, cx| v.reconnect(cx))),
                            )
                        })
                        .child(
                            widgets::button("screen-stop", if busy { "Stop…" } else { "Stop" }, false, !busy, &c).when(!busy, |b| {
                                // The server refuses a stop while a human holds the screen;
                                // when that human is us, stopping is our call to make.
                                b.on_click(move |_, _, cx| stop.update(cx, |s, cx| s.stop_screen(in_control, cx)))
                            }),
                        ),
                )
        });

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(header(
            "Bot screen",
            busy,
            move |_, cx| refresh.update(cx, |s, cx| s.refresh_screen(cx)),
            move |_, cx| close.update(cx, |s, cx| s.close_side_panel(cx)),
            &c,
        ))
        .child(body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not a glob: GPUI's prelude brings its own `test` attribute.
    use super::schedule_label;

    #[test]
    fn spells_out_schedules() {
        assert_eq!(schedule_label("every 60m"), "Hourly");
        assert_eq!(schedule_label("every 1440m"), "Daily");
        assert_eq!(schedule_label("every 90m"), "Every 90 minutes");
        assert_eq!(schedule_label("every 120m"), "Every 2 hours");
        assert_eq!(schedule_label("once in 2h"), "Once, in 2h");
        assert_eq!(schedule_label("30m"), "Once, in 30m");
        assert_eq!(schedule_label("0 9 * * *"), "Daily at 09:00");
        assert_eq!(schedule_label("30 8 * * 1-5"), "Weekdays at 08:30");
        assert_eq!(schedule_label("0 18 * * 1,3"), "Mon, Wed at 18:00");
        assert_eq!(schedule_label("15 * * * *"), "Hourly at :15");
        assert_eq!(schedule_label("*/5 * * * *"), "*/5 * * * *");
    }
}
