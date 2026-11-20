//! Bot answers as display blocks: Markdown parsed in Rust (pulldown-cmark),
//! code highlighted (syntect), and the rest turned into the HTML subset Qt's
//! rich text understands, styled with the Omarchy theme colours.
//!
//! Each block is drawn by its own QML item: text and tables as rich text,
//! code with a header and a copy button, images fetched from the server,
//! GitHub-style alerts with a coloured edge.

use std::sync::OnceLock;

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Deserialize;
use serde_json::{Value, json};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

#[derive(Deserialize, Clone, Debug)]
pub struct Colors {
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub surface: String,
    pub border: String,
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            foreground: "#e6e6e6".into(),
            muted: "#8a8a8d".into(),
            accent: "#FFD60A".into(),
            surface: "#151517".into(),
            border: "#2a2a2e".into(),
        }
    }
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn code_theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let mut set = ThemeSet::load_defaults();
        set.themes.remove("base16-ocean.dark").or_else(|| set.themes.into_values().next()).expect("a theme")
    })
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
pub fn highlight(code: &str, lang: &str) -> String {
    let set = syntaxes();
    let token = lang.split(|c: char| c.is_whitespace() || c == ',').next().unwrap_or_default();
    let syntax = (!token.is_empty())
        .then(|| set.find_syntax_by_token(token).or_else(|| set.find_syntax_by_extension(token)))
        .flatten();
    let Some(syntax) = syntax else {
        return escape(code).replace('\n', "<br>");
    };
    let mut highlighter = HighlightLines::new(syntax, code_theme());
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
            out.push_str(&format!(
                "<span style=\"color:#{:02x}{:02x}{:02x}\">{}</span>",
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
}

impl Builder<'_> {
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
                    let size = match level {
                        HeadingLevel::H1 => 22,
                        HeadingLevel::H2 => 19,
                        HeadingLevel::H3 => 17,
                        _ => 15,
                    };
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
                    self.push(&format!("<blockquote style=\"color:{}; margin-left:12px\">", c.muted));
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
                Tag::Item => self.push("<li>"),
                Tag::Table(_) => {
                    self.flush_text();
                    self.in_table = true;
                    self.table = format!(
                        "<table border=\"1\" cellspacing=\"0\" cellpadding=\"6\" style=\"border-color:{}; border-style:solid\">",
                        c.border
                    );
                }
                Tag::TableHead => self.push(&format!("<tr style=\"background-color:{}\">", c.surface)),
                Tag::TableRow => self.push("<tr>"),
                Tag::TableCell => self.push("<td>"),
                Tag::Emphasis => self.push("<i>"),
                Tag::Strong => self.push("<b>"),
                Tag::Strikethrough => self.push("<s>"),
                Tag::Link { dest_url, .. } => {
                    self.push(&format!("<a href=\"{}\" style=\"color:{}\">", escape(&dest_url), c.accent));
                }
                Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) => self.push("</p>"),
                TagEnd::BlockQuote(Some(_)) => {
                    if let Some((kind, html)) = self.alert.take() {
                        self.blocks.push(json!({ "type": "alert", "kind": kind, "html": html }));
                    }
                }
                TagEnd::BlockQuote(None) => self.push("</blockquote>"),
                TagEnd::CodeBlock => {
                    let lang = self.in_code.take().unwrap_or_default();
                    let raw = std::mem::take(&mut self.code);
                    let raw = raw.strip_suffix('\n').unwrap_or(&raw).to_string();
                    self.blocks.push(json!({
                        "type": "code",
                        "lang": lang,
                        "html": highlight(&raw, &lang),
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
                TagEnd::TableHead | TagEnd::TableRow => self.push("</tr>"),
                TagEnd::TableCell => self.push("</td>"),
                TagEnd::Emphasis => self.push("</i>"),
                TagEnd::Strong => self.push("</b>"),
                TagEnd::Strikethrough => self.push("</s>"),
                TagEnd::Link => self.push("</a>"),
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
                    self.push(&escape(&text));
                }
            }
            Event::Code(code) => self.push(&format!(
                "<code style=\"background-color:{}; font-family:monospace\">&nbsp;{}&nbsp;</code>",
                c.surface,
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
            Event::TaskListMarker(done) => self.push(if done { "☑ " } else { "☐ " }),
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
    };
    for event in Parser::new_ext(&text, options) {
        builder.event(event);
    }
    // A code block still streaming in has no closing fence yet.
    if let Some(lang) = builder.in_code.take() {
        let raw = std::mem::take(&mut builder.code);
        builder.blocks.push(json!({ "type": "code", "lang": lang, "html": highlight(&raw, &lang), "raw": raw }));
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
        assert!(blocks[0]["html"].as_str().unwrap().contains("<b>world</b>"));
        assert_eq!(blocks[1]["lang"], "rust");
        assert_eq!(blocks[1]["raw"], "fn main() {}");
        assert!(blocks[1]["html"].as_str().unwrap().contains("<span style=\"color:#"));
        assert!(blocks[2]["html"].as_str().unwrap().contains("<td>1</td>"));
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
