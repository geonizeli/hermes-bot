//! Bot answers as display blocks: Markdown parsed in Rust (pulldown-cmark),
//! code highlighted (syntect), and prose turned into paragraphs of styled
//! spans, coloured with the app's theme.
//!
//! The UI draws each block on its own: paragraphs as styled text, code with a
//! header and a copy button, tables as a grid, images fetched from the
//! server, GitHub-style alerts with a coloured edge.

use std::collections::HashMap;
use std::ops::Range;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::SyntaxSet;

/// The palette answers are drawn with, as `#rrggbb` strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

/// How a run of text looks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    /// Byte range in the paragraph's text.
    pub range: Range<usize>,
    /// `#rrggbb`.
    pub color: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    /// Inline code: drawn on the surface colour.
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ParaKind {
    #[default]
    Normal,
    /// 1 to 4 (deeper levels draw like 4).
    Heading(u8),
    Quote,
}

/// One paragraph (or list item, heading, quote line) of styled text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Para {
    pub text: String,
    pub spans: Vec<Span>,
    pub kind: ParaKind,
    /// List nesting depth.
    pub indent: u8,
    /// A list marker ("•", "3.") drawn before the text in the accent colour.
    pub marker: Option<String>,
}

impl Para {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Text(Vec<Para>),
    Code { lang: String, raw: String, spans: Vec<Span> },
    /// Rows of cells; the first row is the header.
    Table { rows: Vec<Vec<Para>> },
    /// `path` is a server-side file, `url` anything else.
    Image { path: String, url: String, alt: String },
    Alert { kind: String, paras: Vec<Para> },
    Rule,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn parse_color(hex: &str) -> Color {
    let channel = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(200);
    Color { r: channel(1), g: channel(3), b: channel(5), a: 0xff }
}

/// A syntax theme made from the app's palette, so code takes the same
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

/// Code as coloured token spans over `code`; one plain span when the
/// language is unknown.
pub fn highlight(code: &str, lang: &str, colors: &Colors) -> Vec<Span> {
    let plain = || vec![Span { range: 0..code.len(), color: colors.foreground.clone(), ..Default::default() }];
    let set = syntaxes();
    let token = lang.split(|c: char| c.is_whitespace() || c == ',').next().unwrap_or_default();
    let syntax = (!token.is_empty())
        .then(|| set.find_syntax_by_token(token).or_else(|| set.find_syntax_by_extension(token)))
        .flatten();
    let Some(syntax) = syntax else { return plain() };
    let theme = code_theme(colors);
    let mut highlighter = HighlightLines::new(syntax, &theme);
    let mut spans = Vec::new();
    let mut offset = 0;
    for line in syntect::util::LinesWithEndings::from(code) {
        match highlighter.highlight_line(line, set) {
            Ok(ranges) => {
                for (style, piece) in ranges {
                    let c = style.foreground;
                    spans.push(Span {
                        range: offset..offset + piece.len(),
                        color: format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b),
                        bold: style.font_style.contains(FontStyle::BOLD),
                        italic: style.font_style.contains(FontStyle::ITALIC),
                        ..Default::default()
                    });
                    offset += piece.len();
                }
            }
            Err(_) => {
                spans.push(Span { range: offset..offset + line.len(), color: colors.foreground.clone(), ..Default::default() });
                offset += line.len();
            }
        }
    }
    spans
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

/// Where paragraphs are being collected.
#[derive(PartialEq)]
enum Sink {
    Text,
    Alert,
    Table,
}

struct Builder<'c> {
    colors: &'c Colors,
    blocks: Vec<Block>,
    /// Paragraphs of the text block being built.
    paras: Vec<Para>,
    alert: Option<(String, Vec<Para>)>,
    table: Option<Vec<Vec<Para>>>,
    in_code: Option<String>,
    code: String,
    image: Option<(String, String)>,
    list_stack: Vec<Option<u64>>,
    quote_depth: u32,
    /// The colour text is drawn in, innermost last.
    ink: Vec<String>,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<String>,
    /// The paragraph just opened by a list item, which its first paragraph reuses.
    fresh_item: bool,
}

impl Builder<'_> {
    fn sink(&self) -> Sink {
        if self.table.is_some() {
            Sink::Table
        } else if self.alert.is_some() {
            Sink::Alert
        } else {
            Sink::Text
        }
    }

    fn paras_mut(&mut self) -> &mut Vec<Para> {
        match self.sink() {
            Sink::Table => self
                .table
                .as_mut()
                .and_then(|rows| rows.last_mut())
                .expect("a table row is open"),
            Sink::Alert => &mut self.alert.as_mut().expect("an alert is open").1,
            Sink::Text => &mut self.paras,
        }
    }

    fn new_para(&mut self, para: Para) {
        self.fresh_item = false;
        self.paras_mut().push(para);
    }

    fn current(&mut self) -> &mut Para {
        if self.paras_mut().is_empty() {
            let kind = if self.quote_depth > 0 { ParaKind::Quote } else { ParaKind::Normal };
            let indent = self.list_stack.len() as u8;
            self.paras_mut().push(Para { kind, indent, ..Default::default() });
        }
        self.paras_mut().last_mut().expect("a paragraph")
    }

    fn style(&self, code: bool) -> Span {
        Span {
            range: 0..0,
            color: if code { self.colors.green.clone() } else { self.ink.last().cloned().unwrap_or_default() },
            bold: self.bold > 0,
            italic: self.italic > 0 || self.quote_depth > 0,
            strike: self.strike > 0,
            code,
            link: self.links.last().cloned(),
        }
    }

    fn text(&mut self, text: &str, code: bool) {
        if text.is_empty() {
            return;
        }
        let mut span = self.style(code);
        let para = self.current();
        let start = para.text.len();
        para.text.push_str(text);
        span.range = start..para.text.len();
        // Runs with the same look merge, so a paragraph stays a few spans.
        if let Some(last) = para.spans.last_mut() {
            let same_look = Span { range: last.range.clone(), ..span.clone() } == *last;
            if last.range.end == start && same_look {
                last.range.end = span.range.end;
                return;
            }
        }
        para.spans.push(span);
    }

    fn flush_text(&mut self) {
        let paras: Vec<Para> = std::mem::take(&mut self.paras).into_iter().filter(|p| !p.is_empty()).collect();
        if !paras.is_empty() {
            self.blocks.push(Block::Text(paras));
        }
    }

    fn event(&mut self, event: Event) {
        let c = self.colors.clone();
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if !self.fresh_item {
                        let kind = if self.quote_depth > 0 { ParaKind::Quote } else { ParaKind::Normal };
                        let indent = self.list_stack.len() as u8;
                        self.new_para(Para { kind, indent, ..Default::default() });
                    }
                    self.fresh_item = false;
                }
                Tag::Heading { level, .. } => {
                    let (n, color) = match level {
                        HeadingLevel::H1 => (1, &c.accent),
                        HeadingLevel::H2 => (2, &c.blue),
                        HeadingLevel::H3 => (3, &c.magenta),
                        _ => (4, &c.cyan),
                    };
                    self.ink.push(color.clone());
                    self.bold += 1;
                    self.new_para(Para { kind: ParaKind::Heading(n), ..Default::default() });
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
                    self.alert = Some((name.into(), Vec::new()));
                }
                Tag::BlockQuote(None) => {
                    self.ink.push(c.muted.clone());
                    self.quote_depth += 1;
                }
                Tag::CodeBlock(kind) => {
                    self.flush_text();
                    self.in_code = Some(match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    });
                    self.code.clear();
                }
                Tag::List(start) => self.list_stack.push(start),
                Tag::Item => {
                    let depth = self.list_stack.len();
                    let marker = match self.list_stack.last_mut() {
                        Some(Some(n)) => {
                            let marker = format!("{n}.");
                            *n += 1;
                            marker
                        }
                        _ => if depth > 1 { "◦" } else { "•" }.to_string(),
                    };
                    let kind = if self.quote_depth > 0 { ParaKind::Quote } else { ParaKind::Normal };
                    self.new_para(Para { kind, indent: depth.saturating_sub(1) as u8, marker: Some(marker), ..Default::default() });
                    self.fresh_item = true;
                }
                Tag::Table(_) => {
                    self.flush_text();
                    self.table = Some(Vec::new());
                }
                Tag::TableHead => {
                    self.ink.push(c.accent.clone());
                    self.bold += 1;
                    if let Some(rows) = self.table.as_mut() {
                        rows.push(Vec::new());
                    }
                }
                Tag::TableRow => {
                    if let Some(rows) = self.table.as_mut() {
                        rows.push(Vec::new());
                    }
                }
                Tag::TableCell => {
                    if let Some(row) = self.table.as_mut().and_then(|rows| rows.last_mut()) {
                        row.push(Para::default());
                    }
                }
                Tag::Emphasis => self.italic += 1,
                Tag::Strong => {
                    self.ink.push(c.accent.clone());
                    self.bold += 1;
                }
                Tag::Strikethrough => self.strike += 1,
                Tag::Link { dest_url, .. } => {
                    self.ink.push(c.blue.clone());
                    self.links.push(dest_url.to_string());
                }
                Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {}
                TagEnd::Heading(_) => {
                    self.ink.pop();
                    self.bold = self.bold.saturating_sub(1);
                    self.new_para(Para::default());
                }
                TagEnd::BlockQuote(Some(_)) => {
                    if let Some((kind, paras)) = self.alert.take() {
                        let paras = paras.into_iter().filter(|p| !p.is_empty()).collect();
                        self.blocks.push(Block::Alert { kind, paras });
                    }
                }
                TagEnd::BlockQuote(None) => {
                    self.ink.pop();
                    self.quote_depth = self.quote_depth.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    let lang = self.in_code.take().unwrap_or_default();
                    let raw = std::mem::take(&mut self.code);
                    let raw = raw.strip_suffix('\n').unwrap_or(&raw).to_string();
                    let spans = highlight(&raw, &lang, self.colors);
                    self.blocks.push(Block::Code { lang, raw, spans });
                }
                TagEnd::List(_) => {
                    self.list_stack.pop();
                }
                TagEnd::Item => self.fresh_item = false,
                TagEnd::Table => {
                    if let Some(rows) = self.table.take() {
                        let rows: Vec<Vec<Para>> = rows.into_iter().filter(|r| !r.is_empty()).collect();
                        self.blocks.push(Block::Table { rows });
                    }
                }
                TagEnd::TableHead => {
                    self.ink.pop();
                    self.bold = self.bold.saturating_sub(1);
                }
                TagEnd::TableRow | TagEnd::TableCell => {}
                TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
                TagEnd::Strong => {
                    self.ink.pop();
                    self.bold = self.bold.saturating_sub(1);
                }
                TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
                TagEnd::Link => {
                    self.ink.pop();
                    self.links.pop();
                }
                TagEnd::Image => {
                    if let Some((dest, alt)) = self.image.take() {
                        // Images stand on their own line.
                        if self.sink() == Sink::Text {
                            self.flush_text();
                        }
                        let (path, url) = match dest.strip_prefix("hermes-media:") {
                            Some(p) => (p.to_string(), String::new()),
                            None if dest.starts_with('/') => (dest.clone(), String::new()),
                            None => (String::new(), dest.clone()),
                        };
                        self.blocks.push(Block::Image { path, url, alt });
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
                    self.text(&text, false);
                }
            }
            Event::Code(code) => self.text(&format!("\u{a0}{code}\u{a0}"), true),
            Event::InlineMath(math) | Event::DisplayMath(math) => self.text(&math, true),
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html, false),
            // Bot answers use single newlines as line breaks.
            Event::SoftBreak | Event::HardBreak => {
                if self.in_code.is_some() {
                    self.code.push('\n');
                } else {
                    self.text("\n", false);
                }
            }
            Event::Rule => {
                self.flush_text();
                self.blocks.push(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                let (mark, color) = if done { ("☑ ", c.green.clone()) } else { ("☐ ", c.muted.clone()) };
                self.ink.push(color);
                self.text(mark, false);
                self.ink.pop();
            }
            Event::FootnoteReference(name) => self.text(&format!("[{name}]"), false),
        }
    }
}

/// The blocks of one answer.
pub fn render(text: &str, colors: &Colors) -> Vec<Block> {
    let text = expand_media_tags(text);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_FOOTNOTES;
    let mut builder = Builder {
        colors,
        blocks: Vec::new(),
        paras: Vec::new(),
        alert: None,
        table: None,
        in_code: None,
        code: String::new(),
        image: None,
        list_stack: Vec::new(),
        quote_depth: 0,
        ink: vec![colors.foreground.clone()],
        bold: 0,
        italic: 0,
        strike: 0,
        links: Vec::new(),
        fresh_item: false,
    };
    for event in Parser::new_ext(&text, options) {
        builder.event(event);
    }
    // A code block still streaming in has no closing fence yet.
    if let Some(lang) = builder.in_code.take() {
        let raw = std::mem::take(&mut builder.code);
        let spans = highlight(&raw, &lang, colors);
        builder.blocks.push(Block::Code { lang, raw, spans });
    }
    builder.flush_text();
    builder.blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(blocks: &[Block]) -> Vec<&'static str> {
        blocks
            .iter()
            .map(|b| match b {
                Block::Text(_) => "text",
                Block::Code { .. } => "code",
                Block::Table { .. } => "table",
                Block::Image { .. } => "image",
                Block::Alert { .. } => "alert",
                Block::Rule => "rule",
            })
            .collect()
    }

    fn span_text<'a>(para: &'a Para, span: &Span) -> &'a str {
        &para.text[span.range.clone()]
    }

    #[test]
    fn splits_text_code_and_tables() {
        let md = "Hello **world**\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nbye";
        let colors = Colors::default();
        let blocks = render(md, &colors);
        assert_eq!(kinds(&blocks), vec!["text", "code", "table", "text"]);
        let Block::Text(paras) = &blocks[0] else { panic!() };
        assert_eq!(paras[0].text, "Hello world");
        let strong = paras[0].spans.iter().find(|s| s.bold).unwrap();
        assert_eq!(span_text(&paras[0], strong), "world");
        assert_eq!(strong.color, colors.accent);
        let Block::Code { lang, raw, spans } = &blocks[1] else { panic!() };
        assert_eq!(lang, "rust");
        assert_eq!(raw, "fn main() {}");
        assert!(spans.len() > 1);
        assert_eq!(spans.last().unwrap().range.end, raw.len());
        let Block::Table { rows } = &blocks[2] else { panic!() };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0].text, "1");
        assert!(rows[0][0].spans[0].bold);
    }

    #[test]
    fn colours_code_and_text_from_the_palette() {
        let colors = Colors { magenta: "#aa00aa".into(), blue: "#0000ee".into(), ..Colors::default() };
        let code = highlight("fn main() {}", "rust", &colors);
        assert!(code.iter().any(|s| s.color == "#aa00aa"), "{code:?}");
        let blocks = render("## Title\n\n- item with [a link](https://e.com)", &colors);
        let Block::Text(paras) = &blocks[0] else { panic!() };
        assert_eq!(paras[0].kind, ParaKind::Heading(2));
        assert_eq!(paras[0].spans[0].color, "#0000ee");
        let item = &paras[1];
        assert_eq!(item.marker.as_deref(), Some("•"));
        let link = item.spans.iter().find(|s| s.link.is_some()).unwrap();
        assert_eq!(link.link.as_deref(), Some("https://e.com"));
        assert_eq!(span_text(item, link), "a link");
    }

    #[test]
    fn numbers_and_nests_lists() {
        let blocks = render("3. three\n4. four\n   - inner", &Colors::default());
        let Block::Text(paras) = &blocks[0] else { panic!() };
        let markers: Vec<_> = paras.iter().map(|p| (p.marker.clone().unwrap_or_default(), p.indent)).collect();
        assert_eq!(markers, vec![("3.".into(), 0), ("4.".into(), 0), ("◦".into(), 1)]);
    }

    #[test]
    fn turns_media_tags_and_images_into_image_blocks() {
        let blocks = render("Here:\nMEDIA:/home/h/.hermes/images/cat.png\n\n![x](https://e.com/a.png)", &Colors::default());
        let images: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Image { path, url, .. } => Some((path.clone(), url.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].0, "/home/h/.hermes/images/cat.png");
        assert_eq!(images[1].1, "https://e.com/a.png");
    }

    #[test]
    fn turns_non_image_media_tags_into_download_links() {
        let blocks = render("Done.\nMEDIA:/srv/out/report.pdf\n", &Colors::default());
        let Block::Text(paras) = &blocks[0] else { panic!("{blocks:?}") };
        let para = paras.last().unwrap();
        let link = para.spans.iter().find(|s| s.link.is_some()).expect("file link");
        assert_eq!(link.link.as_deref(), Some("hermes-media:/srv/out/report.pdf"));
        assert!(span_text(para, link).contains("report.pdf"));
    }

    #[test]
    fn renders_alerts_and_keeps_html_as_text() {
        let blocks = render("> [!WARNING]\n> careful <b>\n\n<script>", &Colors::default());
        let Block::Alert { kind, paras } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(kind, "warning");
        assert!(paras[0].text.contains("careful"));
        let Block::Text(rest) = &blocks[1] else { panic!() };
        assert_eq!(rest[0].text.trim(), "<script>");
    }

    #[test]
    fn keeps_an_unfinished_code_block_while_streaming() {
        let blocks = render("Look:\n```py\nprint(1)", &Colors::default());
        assert_eq!(kinds(&blocks), vec!["text", "code"]);
        let Block::Code { raw, .. } = &blocks[1] else { panic!() };
        assert_eq!(raw, "print(1)");
    }

    #[test]
    fn styles_inline_code_and_quotes() {
        let colors = Colors::default();
        let blocks = render("use `cargo`\n\n> quoted", &colors);
        let Block::Text(paras) = &blocks[0] else { panic!() };
        let code = paras[0].spans.iter().find(|s| s.code).unwrap();
        assert_eq!(code.color, colors.green);
        assert_eq!(paras[1].kind, ParaKind::Quote);
        assert!(paras[1].spans[0].italic);
    }
}
