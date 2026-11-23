//! A bot answer, drawn from the Markdown renderer's blocks: styled text with
//! clickable links, highlighted code with a copy button, tables, images and
//! alerts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::motion;
use super::state::{AppState, MediaState};
use super::theme::{Colors, colors, hex};
use super::widgets::{self, ident, id, mono};
use crate::backend::markdown::{self, Block, Para, ParaKind, Span};

thread_local! {
    /// Rendered answers by (text, palette); answers re-render on every
    /// streamed delta, finished ones shouldn't.
    static CACHE: RefCell<HashMap<u64, Rc<Vec<Block>>>> = RefCell::new(HashMap::new());
}

pub fn blocks_for(text: &str, palette: &markdown::Colors) -> Rc<Vec<Block>> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (text, palette).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let blocks = Rc::new(markdown::render(text, palette));
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() > 400 {
            cache.clear();
        }
        cache.insert(key, blocks.clone());
    });
    blocks
}

fn highlights(spans: &[Span], c: &Colors) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    spans
        .iter()
        .map(|s| {
            let color = hex(&s.color);
            (
                s.range.clone(),
                HighlightStyle {
                    color: Some(color),
                    font_weight: s.bold.then_some(FontWeight::BOLD),
                    font_style: s.italic.then_some(FontStyle::Italic),
                    background_color: s.code.then_some(c.surface),
                    underline: s.link.is_some().then_some(UnderlineStyle { thickness: px(1.), color: Some(color.alpha(0.5)), wavy: false }),
                    strikethrough: s.strike.then_some(StrikethroughStyle { thickness: px(1.), color: None }),
                    ..Default::default()
                },
            )
        })
        .collect()
}

/// Opens a link: web links in the browser, server files fetched first.
fn open_link(link: &str, state: &Entity<AppState>, cx: &mut App) {
    match link.strip_prefix("hermes-media:") {
        Some(path) => {
            let path = path.to_string();
            state.update(cx, |s, cx| s.open_media(&path, cx));
        }
        None => cx.open_url(link),
    }
}

/// Styled text, clickable where it links.
pub fn styled_text(key: ElementId, para_text: &str, spans: &[Span], state: &Entity<AppState>, cx: &App) -> AnyElement {
    let c = colors(cx);
    let code_font = mono(cx);
    let overrides: Vec<_> = spans.iter().filter(|s| s.code).map(|s| (s.range.clone(), code_font.clone())).collect();
    let styled = StyledText::new(SharedString::from(para_text.to_string()))
        .with_highlights(highlights(spans, c))
        .with_font_family_overrides(overrides);
    let links: Vec<(std::ops::Range<usize>, String)> =
        spans.iter().filter_map(|s| s.link.clone().map(|l| (s.range.clone(), l))).collect();
    if links.is_empty() {
        return styled.into_any_element();
    }
    let ranges = links.iter().map(|(r, _)| r.clone()).collect();
    let targets: Vec<String> = links.into_iter().map(|(_, l)| l).collect();
    let state = state.clone();
    InteractiveText::new(key, styled)
        .on_click(ranges, move |ix, _, cx| {
            if let Some(link) = targets.get(ix) {
                open_link(link, &state, cx);
            }
        })
        .into_any_element()
}

fn paragraph(key: String, para: &Para, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let c = colors(cx);
    let (size, top) = match para.kind {
        ParaKind::Heading(1) => (22., 10.),
        ParaKind::Heading(2) => (19., 10.),
        ParaKind::Heading(3) => (17., 8.),
        ParaKind::Heading(_) => (15., 6.),
        _ => (15., 0.),
    };
    let text = styled_text(id("para", &key), &para.text, &para.spans, state, cx);
    div()
        .flex()
        .gap(px(8.))
        .pl(px(para.indent as f32 * 18. + if para.marker.is_some() { 4. } else { 0. }))
        .pt(px(top))
        .text_size(px(size))
        .line_height(relative(1.45))
        .when(para.kind == ParaKind::Quote, |d| d.border_l_2().border_color(c.magenta.alpha(0.7)).pl(px(12.)))
        .when_some(para.marker.clone(), |d, marker| d.child(div().flex_none().text_color(c.accent).child(marker)))
        .child(div().flex_1().min_w_0().child(text))
        .into_any_element()
}

/// "Copy" that reads "Copied" for a moment after a click.
pub fn copy_link(key: ElementId, text: String, window: &mut Window, cx: &mut App) -> AnyElement {
    let copied = window.use_keyed_state(key.clone(), cx, |_, _| false);
    let shown = *copied.read(cx);
    let c = colors(cx);
    widgets::text_link(key, if shown { "Copied" } else { "Copy" }, c)
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            copied.update(cx, |v, cx| {
                *v = true;
                cx.notify();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let _ = this.update(cx, |v, cx| {
                        *v = false;
                        cx.notify();
                    });
                })
                .detach();
            });
        })
        .into_any_element()
}

fn code_block(key: String, lang: &str, raw: &str, spans: &[Span], window: &mut Window, cx: &mut App) -> AnyElement {
    let c = colors(cx).clone();
    let font = mono(cx);
    let header = div()
        .h(px(30.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_between()
        .bg(c.raised.alpha(0.6))
        .child(div().text_size(px(12.)).font_family(font.clone()).text_color(c.accent).child(if lang.is_empty() {
            "code".to_string()
        } else {
            lang.to_string()
        }))
        .child(copy_link(id("copy-code", &key), raw.to_string(), window, cx));
    let text = StyledText::new(SharedString::from(raw.to_string())).with_highlights(highlights(spans, &c));
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(c.radius)
        .bg(c.surface)
        .border_1()
        .border_color(c.border)
        .border_l_3()
        .border_color(c.accent)
        .overflow_hidden()
        .child(header)
        .child(
            // Long lines scroll sideways instead of wrapping.
            ident(id("code-scroll", &key))
                .max_h(px(560.))
                .overflow_scroll()
                .child(
                    div()
                        .p(px(12.))
                        .font_family(font)
                        .text_size(px(13.))
                        .line_height(relative(1.4))
                        .whitespace_nowrap()
                        .child(text),
                ),
        )
        .into_any_element()
}

fn table(key: String, rows: &[Vec<Para>], state: &Entity<AppState>, cx: &App) -> AnyElement {
    let c = colors(cx);
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let body = rows.iter().enumerate().map(|(r, row)| {
        let cells = (0..columns).map(|col| {
            let cell = row.get(col);
            div()
                .flex_1()
                .min_w(px(90.))
                .px(px(8.))
                .py(px(6.))
                .when(col > 0, |d| d.border_l_1().border_color(c.border))
                .text_size(px(14.))
                .when_some(cell, |d, p| d.child(styled_text(id("cell", format!("{key}-{r}-{col}")), &p.text, &p.spans, state, cx)))
        });
        div()
            .flex()
            .w_full()
            .when(r == 0, |d| d.bg(c.surface))
            .when(r > 0, |d| d.border_t_1().border_color(c.border))
            .children(cells)
    });
    ident(id("table", &key))
        .w_full()
        .overflow_x_scroll()
        .child(
            div()
                .min_w_full()
                .flex()
                .flex_col()
                .border_1()
                .border_color(c.border)
                .rounded(c.radius)
                .children(body),
        )
        .into_any_element()
}

/// A picture's size scaled to fit (never up), read from its header once.
fn fitted_size(file: &std::path::Path, max_w: f32, max_h: f32) -> (f32, f32) {
    thread_local! {
        static SIZES: RefCell<HashMap<std::path::PathBuf, Option<(u32, u32)>>> = RefCell::new(HashMap::new());
    }
    let dims = SIZES.with(|s| *s.borrow_mut().entry(file.to_path_buf()).or_insert_with(|| image::image_dimensions(file).ok()));
    match dims {
        Some((w, h)) if w > 0 && h > 0 => {
            let scale = (max_w / w as f32).min(max_h / h as f32).min(1.0);
            ((w as f32 * scale).round(), (h as f32 * scale).round())
        }
        _ => (max_w.min(320.), max_h.min(240.)),
    }
}

/// An image on the Hermes server (a path) or anywhere else (a URL).
pub fn remote_image(
    key: ElementId,
    path: &str,
    url: &str,
    max_w: f32,
    max_h: f32,
    state: &Entity<AppState>,
    cx: &mut App,
) -> AnyElement {
    let c = colors(cx).clone();
    // A breathing frame where the picture will be.
    let pulse_key = ElementId::Name(format!("{key}-loading").into());
    let placeholder = move || {
        let frame = div().w(px(160.)).h(px(100.)).rounded(c.radius).bg(c.foreground.alpha(0.07)).border_1().border_color(c.border);
        motion::breathe(frame, pulse_key.clone(), 0.4).into_any_element()
    };
    // Rows are measured once, so the picture's box is sized before it loads.
    let source: Option<(ImageSource, String, (f32, f32))> = if !path.is_empty() {
        match state.read(cx).media.get(path).cloned() {
            Some(MediaState::Ready(file)) => {
                let open = url::Url::from_file_path(&file).map(|u| u.to_string()).unwrap_or_default();
                let size = fitted_size(&file, max_w, max_h);
                Some((ImageSource::from(file), open, size))
            }
            Some(MediaState::Loading) => None,
            Some(MediaState::Failed) => return widgets::muted(format!("Couldn't load {path}"), 12., colors(cx)).into_any_element(),
            None => {
                let (state, path) = (state.clone(), path.to_string());
                cx.defer(move |cx| state.update(cx, |s, cx| s.request_media(&path, cx)));
                None
            }
        }
    } else if let Ok(parsed) = url::Url::parse(url) {
        match parsed.to_file_path() {
            Ok(file) if parsed.scheme() == "file" => {
                let size = fitted_size(&file, max_w, max_h);
                Some((ImageSource::from(file), url.to_string(), size))
            }
            _ => Some((ImageSource::from(url.to_string()), url.to_string(), (max_w.min(320.), max_h.min(240.)))),
        }
    } else {
        None
    };
    let Some((source, open, (w, h))) = source else { return placeholder() };
    let fallback = placeholder.clone();
    ident(key)
        .cursor_pointer()
        .on_click(move |_, _, cx| cx.open_url(&open))
        .child(
            img(source)
                .w(px(w))
                .h(px(h))
                .object_fit(ObjectFit::Contain)
                .with_loading(placeholder)
                .with_fallback(fallback),
        )
        .into_any_element()
}

fn alert(key: String, kind: &str, paras: &[Para], state: &Entity<AppState>, cx: &App) -> AnyElement {
    let c = colors(cx);
    let edge = match kind {
        "note" => c.blue,
        "tip" => c.green,
        "important" => c.magenta,
        "warning" => c.yellow,
        "caution" => c.red,
        _ => c.accent,
    };
    let title = kind.chars().next().map(|f| f.to_uppercase().collect::<String>() + &kind[f.len_utf8()..]).unwrap_or_default();
    div()
        .w_full()
        .rounded(c.radius)
        .bg(edge.alpha(0.08))
        .border_l_3()
        .border_color(edge)
        .px(px(14.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(edge).child(title))
        .children(paras.iter().enumerate().map(|(i, p)| paragraph(format!("{key}-{i}"), p, state, cx)))
        .into_any_element()
}

/// The answer `text`, as a column of blocks. `key` keeps element ids
/// stable across frames.
pub fn render(key: u64, text: &str, state: &Entity<AppState>, window: &mut Window, cx: &mut App) -> AnyElement {
    let palette = colors(cx).markdown.clone();
    let blocks = blocks_for(text, &palette);
    let mut children = Vec::with_capacity(blocks.len());
    for (b, block) in blocks.iter().enumerate() {
        let k = format!("{key}-{b}");
        let el = match block {
            Block::Text(paras) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(paras.iter().enumerate().map(|(i, p)| paragraph(format!("{k}-{i}"), p, state, cx)))
                .into_any_element(),
            Block::Code { lang, raw, spans } => code_block(k, lang, raw, spans, window, cx),
            Block::Table { rows } => table(k, rows, state, cx),
            Block::Image { path, url, alt } => {
                let image = remote_image(id("md-img", &k), path, url, 640., 480., state, cx);
                let c = colors(cx);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(image)
                    .when(!alt.is_empty(), |d| d.child(widgets::muted(alt.clone(), 12., c)))
                    .into_any_element()
            }
            Block::Alert { kind, paras } => alert(k, kind, paras, state, cx),
            Block::Rule => div().w_full().h(px(1.)).bg(colors(cx).border).into_any_element(),
        };
        children.push(el);
    }
    div().w_full().flex().flex_col().gap(px(10.)).children(children).into_any_element()
}
