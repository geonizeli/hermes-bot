//! One tool call: a compact chip (status, name, what it's doing, duration)
//! that expands into its arguments, result, diff and to-do list.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::markdown_view;
use super::motion;
use super::selectable::RichText;
use super::state::{AppState, Row, Todo};
use super::theme::{Colors, colors};
use super::widgets::{self, ident, id, mono};

/// Each kind of tool gets its own colour, so a run of calls reads at a glance.
pub fn tint(name: &str, c: &Colors) -> Hsla {
    let n = name.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if any(&["terminal", "shell", "exec", "code", "process", "bash", "run"]) {
        c.green
    } else if any(&["patch", "write", "edit", "file", "read", "search_files", "fs"]) {
        c.blue
    } else if any(&["web", "browser", "fetch", "url", "search"]) {
        c.cyan
    } else if any(&["image", "vision", "media", "screen", "display"]) {
        c.magenta
    } else if any(&["todo", "memory", "skill", "cron", "schedule"]) {
        c.yellow
    } else if any(&["delegate", "agent", "session", "send", "message"]) {
        c.orange
    } else {
        c.accent
    }
}

pub fn todo_list(todos: &[Todo], c: &Colors) -> impl IntoElement {
    div().pl(px(12.)).flex().flex_col().gap(px(2.)).children(todos.iter().map(|t| {
        let (mark, color) = match t.status.as_str() {
            "completed" => ("☑", c.green),
            "in_progress" => ("◐", c.yellow),
            "cancelled" => ("☒", c.red),
            _ => ("☐", c.muted),
        };
        let finished = t.status == "completed" || t.status == "cancelled";
        div()
            .flex()
            .gap(px(6.))
            .text_size(px(13.))
            .child(div().flex_none().text_color(color).child(mark))
            .child(
                div()
                    .text_color(if finished { c.muted } else { c.foreground })
                    .when(t.status == "cancelled", |d| d.line_through())
                    .child(t.content.clone()),
            )
    }))
}

/// A unified diff with added lines green and removed lines red.
fn diff_text(key: ElementId, order: u64, text: &str, c: &Colors) -> RichText {
    let mut highlights = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let color = if line.starts_with("+++") || line.starts_with("---") {
            Some(c.muted)
        } else if line.starts_with('+') {
            Some(c.green)
        } else if line.starts_with('-') {
            Some(c.red)
        } else if line.starts_with("@@") {
            Some(c.blue)
        } else {
            None
        };
        if let Some(color) = color {
            highlights.push((offset..offset + line.len(), HighlightStyle { color: Some(color), ..Default::default() }));
        }
        offset += line.len();
    }
    RichText::new(key, text.to_string(), markdown_view::selection_color(c)).order(order).styled(|t| t.with_highlights(highlights))
}

fn section(title: &'static str, body: AnyElement, heading: Hsla, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(heading).child(title))
        .child(div().font_family(mono(cx)).text_size(px(12.)).child(body))
}

pub fn render(ix: usize, row: &Row, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let c = colors(cx).clone();
    let tint = tint(&row.name, &c);
    let has_body = !row.args.is_empty() || !row.result.is_empty() || !row.diff.is_empty() || !row.error.is_empty();
    let failed = !row.error.is_empty();
    let duration = (row.duration > 0.0).then(|| {
        if row.duration < 10.0 { format!("{:.1}s", row.duration) } else { format!("{}s", row.duration.round()) }
    });
    let hover_bg = tint.alpha(0.16);
    let toggle_state = state.clone();
    let chip = ident(id("tool-chip", row.id))
        .h(px(30.))
        .px(px(10.))
        .max_w_full()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .rounded(c.radius)
        .bg(tint.alpha(0.07))
        .border_1()
        .border_color(if failed { c.danger.alpha(0.6) } else { tint.alpha(0.35) })
        .overflow_hidden()
        .when(has_body, |d| {
            d.cursor_pointer().hover(move |s| s.bg(hover_bg)).on_click(move |_, _, cx| {
                toggle_state.update(cx, |s, cx| {
                    s.toggle_expanded(ix);
                    cx.notify();
                })
            })
        })
        .child(if row.done {
            // The mark lands as the call finishes.
            let settled = motion::fresh(row.settled_at, motion::SLOW);
            div()
                .text_size(px(12.))
                .text_color(if failed { c.danger } else { tint })
                .child(if failed { "✕" } else { "✓" })
                .with_animation(id("tool-mark", row.id), Animation::new(motion::BASE).with_easing(motion::ease_out), move |d, t| {
                    if settled { d.opacity(t).text_size(px(6. + 6. * t)) } else { d }
                })
                .into_any_element()
        } else {
            widgets::spinner(14.).into_any_element()
        })
        .child(
            div()
                .flex_none()
                .text_size(px(13.))
                .font_family(mono(cx))
                .text_color(tint)
                .child(if row.name.is_empty() { "tool".to_string() } else { row.name.clone() }),
        )
        .when(!row.detail.is_empty(), |d| {
            d.child(div().min_w_0().max_w(px(420.)).truncate().text_size(px(12.)).text_color(c.muted).child(row.detail.clone()))
        })
        .when_some(duration, |d, t| d.child(div().flex_none().text_size(px(11.)).text_color(c.muted).child(t)))
        .when(has_body, |d| {
            d.child(div().flex_none().text_size(px(11.)).text_color(c.muted).child(if row.expanded { "▾" } else { "▸" }))
        });

    let body = (row.expanded && has_body).then(|| {
        let order = super::chat::reading_order(row.id);
        let mut sections = Vec::new();
        if !row.error.is_empty() {
            sections.push(section("Error", div().text_color(c.danger).child(markdown_view::plain_text(id("tool-error", row.id), order, row.error.clone(), cx)).into_any_element(), c.danger, cx).into_any_element());
        }
        if !row.args.is_empty() {
            sections.push(section("Arguments", markdown_view::plain_text(id("tool-args", row.id), order + 1, row.args.clone(), cx), tint, cx).into_any_element());
        }
        if !row.diff.is_empty() {
            sections.push(section("Changes", diff_text(id("tool-diff", row.id), order + 2, &row.diff, &c).into_any_element(), tint, cx).into_any_element());
        }
        if !row.result.is_empty() {
            sections.push(section("Result", markdown_view::plain_text(id("tool-result", row.id), order + 3, row.result.clone(), cx), tint, cx).into_any_element());
        }
        let body = ident(id("tool-body", row.id))
            .w_full()
            .max_h(px(520.))
            .overflow_y_scroll()
            .p(px(10.))
            .rounded(c.radius)
            .bg(c.surface)
            .border_1()
            .border_color(c.border)
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(sections);
        motion::rise_in_when(motion::fresh(row.toggled_at, motion::SLOW), body, id("tool-body-in", row.id), -6., motion::BASE)
    });

    div()
        .w_full()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(6.))
        .child(chip)
        // To-dos show without expanding: they're the point of the call.
        .when(!row.todos.is_empty(), |d| d.child(todo_list(&row.todos, &c)))
        .children(body)
        .into_any_element()
}

/// The turn's to-do card (`todo.updated`).
pub fn render_todos(row: &Row, cx: &App) -> AnyElement {
    let c = colors(cx);
    let tint = c.yellow;
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(6.))
        .child(
            div()
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(c.radius)
                .bg(tint.alpha(0.07))
                .border_1()
                .border_color(tint.alpha(0.35))
                .child(div().text_size(px(13.)).font_family(mono(cx)).text_color(tint).child("todo"))
                .child(div().text_size(px(12.)).text_color(c.muted).child("Plan")),
        )
        .child(todo_list(&row.todos, c))
        .into_any_element()
}
