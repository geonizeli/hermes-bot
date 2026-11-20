//! Bot answers as display blocks: Markdown parsed in Rust (pulldown-cmark),
//! code highlighted (syntect), and the rest turned into the HTML subset Qt's
//! rich text understands, styled with the Omarchy theme colours.
//!
//! Each block is drawn by its own QML item: text and tables as rich text,
//! code with a header and a copy button, images fetched from the server,
//! GitHub-style alerts with a coloured edge.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Deserialize;
use serde_json::{Value, json};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::SyntaxSet;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Colors {
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub surface: String,
    pub border: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub orange: String,
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            foreground: "#e6e6e6".into(),
            muted: "#8a8a8d".into(),
            accent: "#FFD60A".into(),
            surface: "#151517".into(),
            border: "#2a2a2e".into(),
            red: "#ff6b6b".into(),
            green: "#3fb950".into(),
            yellow: "#e3b341".into(),
            blue: "#4493f8".into(),
            magenta: "#bc8cff".into(),
            cyan: "#39c5cf".into(),
            orange: "#f0883e".into(),
        }
    }
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn parse_color(hex: &str) -> Color {
    let channel = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(200);
    Color { r: channel(1), g: channel(3), b: channel(5), a: 0xff }
}

/// A syntax theme made from the Omarchy palette, so code takes the same
/// colours as the terminal. Built once per palette.
fn code_theme(c: &Colors) -> Arc<Theme> {
    static THEMES: OnceLock<Mutex<HashMap<String, Arc<Theme>>>> = OnceLock::new();
    let key = [&c.foreground, &c.muted, &c.red, &c.green, &c.yellow, &c.blue, &c.magenta, &c.cyan, &c.orange]
        .map(String::as_str)
        .join(",");
    let mut themes = THEMES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    themes
        .entry(key)
        .or_insert_with(|| {
            let item = |scope: &str, color: &str, font_style: Option<FontStyle>| ThemeItem {
                scope: ScopeSelectors::from_str(scope).unwrap_or_default(),
                style: StyleModifier { foreground: Some(parse_color(color)), background: None, font_style },
            };
            Arc::new(Theme {
                settings: ThemeSettings { foreground: Some(parse_color(&c.foreground)), ..Default::default() },
                scopes: vec![
                    item("comment, punctuation.definition.comment", &c.muted, Some(FontStyle::ITALIC)),
                    item("string, punctuation.definition.string, constant.character.escape", &c.green, None),
                    item("constant.numeric, constant.language, constant.other, support.constant", &c.orange, None),
                    item("keyword, storage, keyword.control", &c.magenta, None),
                    item("keyword.operator, punctuation.separator.key-value", &c.cyan, None),
                    item("entity.name.function, support.function, meta.function-call.generic", &c.blue, None),
                    item("entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, support.type, support.class", &c.yellow, None),
                    item("variable.language, entity.name.tag, markup.deleted", &c.red, None),
                    item("entity.other.attribute-name, variable.parameter, entity.name.label", &c.orange, None),
                    item("markup.heading, entity.name.section", &c.blue, Some(FontStyle::BOLD)),
                    item("markup.inserted", &c.green, None),
                    item("entity.name.namespace, entity.name.module, support.module", &c.cyan, None),
                ],
                ..Default::default()
            })
        })
        .clone()
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Code as HTML spans, one colour per token; plain escaped text when the
/// language is unknown.
pub fn highlight(code: &str, lang: &str, colors: &Colors) -> String {
    let set = syntaxes();
    let token = lang.split(|c: char| c.is_whitespace() || c == ',').next().unwrap_or_default();
    let syntax = (!token.is_empty())
        .then(|| set.find_syntax_by_token(token).or_else(|| set.find_syntax_by_extension(token)))
        .flatten();
    let Some(syntax) = syntax else {
        return escape(code).replace('\n', "<br>");
    };
    let theme = code_theme(colors);
    let mut highlighter = HighlightLines::new(syntax, &theme);
    let mut out = String::new();
    for line in syntect::util::LinesWithEndings::from(code) {
        let Ok(ranges) = highlighter.highlight_line(line, set) else {
            out.push_str(&escape(line));
            continue;
        };
        for (style, piece) in ranges {
            let c = style.foreground;
            let piece = piece.trim_end_matches('\n');
            if piece.is_empty() {
                continue;
            }
            let (open, close) = match (
                style.font_style.contains(FontStyle::ITALIC),
                style.font_style.contains(FontStyle::BOLD),
            ) {
                (true, _) => ("<i>", "</i>"),
                (_, true) => ("<b>", "</b>"),
                _ => ("", ""),
            };
            out.push_str(&format!(
                "<span style=\"color:#{:02x}{:02x}{:02x}\">{open}{}{close}</span>",
                c.r,
                c.g,
                c.b,
                escape(piece).replace(' ', "&nbsp;")
            ));
        }
        if line.ends_with('\n') {
            out.push_str("<br>");
        }
    }
    out.trim_end_matches("<br>").to_string()
}

fn is_image_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"].iter().any(|e| lower.ends_with(e))
}

/// `MEDIA:<path>` lines (how Hermes tools hand back files) become Markdown
/// images or links the renderer understands.
fn expand_media_tags(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            match trimmed.strip_prefix("MEDIA:") {
                Some(path) if !path.trim().is_empty() => {
                    let path = path.trim();
                    let name = path.rsplit('/').next().unwrap_or(path);
                    if is_image_path(path) {
                        format!("![{name}](hermes-media:{path})")
                    } else {
                        format!("[📄 {name}](hermes-media:{path})")
                    }
                }
                _ => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Builder<'c> {
    colors: &'c Colors,
    blocks: Vec<Value>,
    html: String,
    // Container stack so a flush can reopen what's still open (lists, quotes).
    in_table: bool,
    table: String,
    in_code: Option<String>,
    code: String,
    alert: Option<(String, String)>,
    image: Option<(String, String)>,
    list_stack: Vec<Option<u64>>,
    // The colour text is drawn in, innermost last. Text is always wrapped in
    // its colour so list markers can take the accent without their items.
    ink: Vec<String>,
}

impl Builder<'_> {
    fn text(&mut self, text: &str) {
        let html = escape(text);
        match self.ink.last() {
            Some(color) => {
                let span = format!("<span style=\"color:{color}\">{html}</span>");
                self.push(&span)
            }
            None => self.push(&html),
        }
    }

    fn push(&mut self, s: &str) {
        if let Some((_, html)) = self.alert.as_mut() {
            html.push_str(s);
        } else if self.in_table {
            self.table.push_str(s);
        } else {
            self.html.push_str(s);
        }
    }

    fn flush_text(&mut self) {
        let html = std::mem::take(&mut self.html);
        if !html.trim().is_empty() {
            self.blocks.push(json!({ "type": "text", "html": html }));
        }
    }

    fn event(&mut self, event: Event) {
        let c = self.colors.clone();
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.push("<p>"),
                Tag::Heading { level, .. } => {
                    let (size, color) = match level {
                        HeadingLevel::H1 => (22, &c.accent),
                        HeadingLevel::H2 => (19, &c.blue),
                        HeadingLevel::H3 => (17, &c.magenta),
                        _ => (15, &c.cyan),
                    };
                    self.ink.push(color.clone());
                    self.push(&format!("<p style=\"font-size:{size}px; font-weight:600; margin-top:10px\">"));
                }
                Tag::BlockQuote(Some(kind)) => {
                    self.flush_text();
                    let name = match kind {
                        BlockQuoteKind::Note => "note",
                        BlockQuoteKind::Tip => "tip",
                        BlockQuoteKind::Important => "important",
                        BlockQuoteKind::Warning => "warning",
                        BlockQuoteKind::Caution => "caution",
                    };
                    self.alert = Some((name.into(), String::new()));
                }
                Tag::BlockQuote(None) => {
                    self.ink.push(c.muted.clone());
                    self.push(&format!(
                        "<table cellspacing=\"0\" cellpadding=\"0\" style=\"margin-top:4px; margin-bottom:4px\"><tr><td width=\"3\" style=\"background-color:{}\"></td><td style=\"padding-left:12px\"><i>",
                        c.magenta
                    ));
                }
                Tag::CodeBlock(kind) => {
                    self.flush_text();
                    self.in_code = Some(match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    });
                    self.code.clear();
                }
                Tag::List(start) => {
                    self.list_stack.push(start);
                    match start {
                        Some(n) => self.push(&format!("<ol start=\"{n}\">")),
                        None => self.push("<ul>"),
                    }
                }
                Tag::Item => self.push(&format!("<li style=\"color:{}\">", c.accent)),
                Tag::Table(_) => {
                    self.flush_text();
                    self.in_table = true;
                    self.table = format!(
                        "<table border=\"1\" cellspacing=\"0\" cellpadding=\"6\" style=\"border-color:{}; border-style:solid\">",
                        c.border
                    );
                }
                Tag::TableHead => {
                    self.ink.push(c.accent.clone());
                    self.push(&format!("<tr style=\"background-color:{}; font-weight:600\">", c.surface))
                }
                Tag::TableRow => self.push("<tr>"),
                Tag::TableCell => self.push("<td>"),
                Tag::Emphasis => self.push("<i>"),
                Tag::Strong => {
                    self.ink.push(c.accent.clone());
                    self.push("<b>")
                }
                Tag::Strikethrough => self.push("<s>"),
                Tag::Link { dest_url, .. } => {
                    self.ink.push(c.blue.clone());
                    self.push(&format!("<a href=\"{}\" style=\"color:{}\">", escape(&dest_url), c.blue));
                }
                Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => self.push("</p>"),
                TagEnd::Heading(_) => {
                    self.ink.pop();
                    self.push("</p>")
                }
                TagEnd::BlockQuote(Some(_)) => {
                    if let Some((kind, html)) = self.alert.take() {
                        self.blocks.push(json!({ "type": "alert", "kind": kind, "html": html }));
                    }
                }
                TagEnd::BlockQuote(None) => {
                    self.ink.pop();
                    self.push("</i></td></tr></table>")
                }
                TagEnd::CodeBlock => {
                    let lang = self.in_code.take().unwrap_or_default();
                    let raw = std::mem::take(&mut self.code);
                    let raw = raw.strip_suffix('\n').unwrap_or(&raw).to_string();
                    self.blocks.push(json!({
                        "type": "code",
                        "lang": lang,
                        "html": highlight(&raw, &lang, self.colors),
                        "raw": raw,
                    }));
                }
                TagEnd::List(_) => {
                    match self.list_stack.pop().flatten() {
                        Some(_) => self.push("</ol>"),
                        None => self.push("</ul>"),
                    }
                }
                TagEnd::Item => self.push("</li>"),
                TagEnd::Table => {
                    self.in_table = false;
                    let mut html = std::mem::take(&mut self.table);
                    html.push_str("</table>");
                    self.blocks.push(json!({ "type": "table", "html": html }));
                }
                TagEnd::TableHead => {
                    self.ink.pop();
                    self.push("</tr>")
                }
                TagEnd::TableRow => self.push("</tr>"),
                TagEnd::TableCell => self.push("</td>"),
                TagEnd::Emphasis => self.push("</i>"),
                TagEnd::Strong => {
                    self.ink.pop();
                    self.push("</b>")
                }
                TagEnd::Strikethrough => self.push("</s>"),
                TagEnd::Link => {
                    self.ink.pop();
                    self.push("</a>")
                }
                TagEnd::Image => {
                    if let Some((dest, alt)) = self.image.take() {
                        // Images stand on their own line.
                        self.flush_text();
                        let (path, url) = match dest.strip_prefix("hermes-media:") {
                            Some(p) => (p.to_string(), String::new()),
                            None if dest.starts_with('/') => (dest.clone(), String::new()),
                            None => (String::new(), dest.clone()),
                        };
                        self.blocks.push(json!({ "type": "image", "path": path, "url": url, "alt": alt }));
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if self.in_code.is_some() {
                    self.code.push_str(&text);
                } else if let Some((_, alt)) = self.image.as_mut() {
                    alt.push_str(&text);
                } else {
                    self.text(&text);
                }
            }
            Event::Code(code) => self.push(&format!(
                "<code style=\"background-color:{}; color:{}; font-family:monospace\">&nbsp;{}&nbsp;</code>",
                c.surface,
                c.green,
                escape(&code)
            )),
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                self.push(&format!("<code style=\"font-family:monospace\">{}</code>", escape(&math)))
            }
            Event::Html(html) | Event::InlineHtml(html) => self.push(&escape(&html)),
            // Bot answers use single newlines as line breaks.
            Event::SoftBreak | Event::HardBreak => {
                if self.in_code.is_some() {
                    self.code.push('\n');
                } else {
                    self.push("<br>");
                }
            }
            Event::Rule => {
                self.flush_text();
                self.blocks.push(json!({ "type": "rule" }));
            }
            Event::TaskListMarker(done) => {
                let (mark, color) = if done { ("☑", &c.green) } else { ("☐", &c.muted) };
                self.push(&format!("<span style=\"color:{color}\">{mark}</span> "))
            }
            Event::FootnoteReference(name) => self.push(&format!("<sup>{}</sup>", escape(&name))),
        }
    }
}

/// The blocks of one answer, as JSON for QML.
pub fn render(text: &str, colors: &Colors) -> Value {
    let text = expand_media_tags(text);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES;
    let mut builder = Builder {
        colors,
        blocks: Vec::new(),
        html: String::new(),
        in_table: false,
        table: String::new(),
        in_code: None,
        code: String::new(),
        alert: None,
        image: None,
        list_stack: Vec::new(),
        ink: vec![colors.foreground.clone()],
    };
    for event in Parser::new_ext(&text, options) {
        builder.event(event);
    }
    // A code block still streaming in has no closing fence yet.
    if let Some(lang) = builder.in_code.take() {
        let raw = std::mem::take(&mut builder.code);
        builder.blocks.push(json!({ "type": "code", "lang": lang, "html": highlight(&raw, &lang, colors), "raw": raw }));
    }
    builder.flush_text();
    Value::Array(builder.blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|b| b["type"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn splits_text_code_and_tables() {
        let md = "Hello **world**\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nbye";
        let blocks = render(md, &Colors::default());
        assert_eq!(kinds(&blocks), vec!["text", "code", "table", "text"]);
        assert!(blocks[0]["html"].as_str().unwrap().contains("world</span></b>"));
        assert_eq!(blocks[1]["lang"], "rust");
        assert_eq!(blocks[1]["raw"], "fn main() {}");
        assert!(blocks[1]["html"].as_str().unwrap().contains("<span style=\"color:#"));
        assert!(blocks[2]["html"].as_str().unwrap().contains("1</span></td>"));
    }

    #[test]
    fn colours_code_and_text_from_the_palette() {
        let colors = Colors { magenta: "#aa00aa".into(), blue: "#0000ee".into(), ..Colors::default() };
        let code = highlight("fn main() {}", "rust", &colors);
        assert!(code.contains("color:#aa00aa"), "{code}");
        let blocks = render("## Title\n\n- item with [a link](https://e.com)", &colors);
        let html = blocks.to_string();
        assert!(html.contains("color:#0000ee"));
        assert!(html.contains(&format!("<li style=\\\"color:{}\\\">", colors.accent)), "{html}");
    }

    #[test]
    fn turns_media_tags_and_images_into_image_blocks() {
        let blocks = render("Here:\nMEDIA:/home/h/.hermes/images/cat.png\n\n![x](https://e.com/a.png)", &Colors::default());
        let images: Vec<_> = blocks.as_array().unwrap().iter().filter(|b| b["type"] == "image").collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0]["path"], "/home/h/.hermes/images/cat.png");
        assert_eq!(images[1]["url"], "https://e.com/a.png");
    }

    #[test]
    fn renders_alerts_and_escapes_html() {
        let blocks = render("> [!WARNING]\n> careful <b>\n\n<script>", &Colors::default());
        assert_eq!(blocks[0]["type"], "alert");
        assert_eq!(blocks[0]["kind"], "warning");
        let all = blocks.to_string();
        assert!(!all.contains("<script>"));
    }

    #[test]
    fn keeps_an_unfinished_code_block_while_streaming() {
        let blocks = render("Look:\n```py\nprint(1)", &Colors::default());
        assert_eq!(kinds(&blocks), vec!["text", "code"]);
        assert_eq!(blocks[1]["raw"], "print(1)");
    }
}
