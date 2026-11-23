//! Edit Bot: the official desktop's Edit Profile dialog without its Advanced
//! section. A preview, the avatar picker (Bot / Generate / Upload / Pet),
//! the title and the description. The logic lives in Rust
//! (backend/botedit.rs).

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::state::editor::{self as faces, pet_fields};
use super::state::{AppEvent, AppState};
use super::theme::{colors, hex};
use super::widgets::{self, id, modal, muted};
use crate::backend::botedit::Look;

const PET_COLUMNS: usize = 3;
const PET_ROWS: usize = 4;
const PET_PAGE: usize = PET_COLUMNS * PET_ROWS;
/// Pages of thumbnails fetched ahead, so turning the page shows pictures.
const PRELOAD_PAGES: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Bot,
    Generate,
    Upload,
    Pet,
}

pub struct EditBotDialog {
    state: Entity<AppState>,
    pub open: bool,
    bot_name: String,
    display_name: String,
    opened: Look,
    shapes: Vec<String>,
    kinds: Vec<String>,
    swatches: Vec<String>,
    // The look being edited.
    shape: String,
    picked_color: String,
    image: Option<PathBuf>,
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    describe: Entity<TextareaState>,

    tab: Tab,
    busy: bool,
    error: String,
    /// `None` until probed.
    can_generate: Option<bool>,
    generating: bool,

    // ── the Pet tab ──
    pets: Vec<(String, String, String)>,
    pets_loading: bool,
    pets_loaded: bool,
    pet_error: String,
    selected_slug: String,
    page: usize,
    pet_search: Entity<InputState>,
    /// slug → thumbnail (`None` while loading).
    thumbs: HashMap<String, Option<PathBuf>>,
    _subscriptions: Vec<Subscription>,
}

impl EditBotDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx));
        let description = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 8).placeholder("What should this bot help with?"));
        let describe = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 6).placeholder("Describe your avatar…"));
        let pet_search = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&pet_search, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.page = 0;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&describe, window, |_, _, _: &InputEvent, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, |this, _, event: &AppEvent, _, cx| {
                if !this.open {
                    return;
                }
                match event {
                    AppEvent::EditSaved(result) => {
                        this.busy = false;
                        match result {
                            Ok(()) => this.open = false,
                            Err(e) => this.error = e.clone(),
                        }
                    }
                    AppEvent::EditImageReady(result) => {
                        this.generating = false;
                        match result {
                            Ok(path) => this.image = Some(path.clone()),
                            Err(e) => this.error = e.clone(),
                        }
                    }
                    AppEvent::EditGenerateAvailable(available) => this.can_generate = Some(*available),
                    AppEvent::EditPetsLoaded(result) => {
                        this.pets_loading = false;
                        match result {
                            Ok(pets) => {
                                this.pets = pets.iter().map(pet_fields).collect();
                                this.pets_loaded = true;
                                this.pet_error.clear();
                            }
                            Err(e) => this.pet_error = e.clone(),
                        }
                    }
                    AppEvent::EditPetReady(slug, path) => match path {
                        Some(path) => {
                            this.thumbs.insert(slug.clone(), Some(path.clone()));
                            if *slug == this.selected_slug {
                                this.image = Some(path.clone());
                            }
                        }
                        None => {
                            // Never keep a failure: a later visit retries.
                            this.thumbs.remove(slug);
                            if *slug == this.selected_slug {
                                this.selected_slug.clear();
                                this.pet_error = "Could not load that pet — try another.".into();
                            }
                        }
                    },
                    _ => return,
                }
                cx.notify();
            }),
        ];
        EditBotDialog {
            state,
            open: false,
            bot_name: String::new(),
            display_name: String::new(),
            opened: Look::default(),
            shapes: Vec::new(),
            kinds: Vec::new(),
            swatches: Vec::new(),
            shape: String::new(),
            picked_color: String::new(),
            image: None,
            title,
            description,
            describe,
            tab: Tab::Bot,
            busy: false,
            error: String::new(),
            can_generate: None,
            generating: false,
            pets: Vec::new(),
            pets_loading: false,
            pets_loaded: false,
            pet_error: String::new(),
            selected_slug: String::new(),
            page: 0,
            pet_search,
            thumbs: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, bot: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (info, display_name) = {
            let s = self.state.read(cx);
            let display = s.bots.iter().find(|b| b.name == bot).map(|b| b.display_name.clone()).unwrap_or_else(|| bot.to_string());
            (s.edit_bot_open(bot), display)
        };
        self.bot_name = info.name;
        self.display_name = display_name.clone();
        self.shapes = info.shapes;
        self.kinds = info.kinds;
        self.swatches = info.swatches;
        self.shape = info.look.shape.clone();
        self.picked_color = info.look.color.clone().unwrap_or_default();
        self.image = info.look.image.clone();
        let (title, description) = (info.look.title.clone(), info.look.description.clone());
        self.title.update(cx, |i, cx| {
            i.set_value(title, window, cx);
            i.set_placeholder(display_name, window, cx);
        });
        self.description.update(cx, |i, cx| i.set_value(description, window, cx));
        self.describe.update(cx, |i, cx| i.set_value("", window, cx));
        self.pet_search.update(cx, |i, cx| i.set_value("", window, cx));
        self.opened = info.look;
        self.tab = Tab::Bot;
        self.busy = false;
        self.error.clear();
        self.generating = false;
        self.selected_slug.clear();
        self.page = 0;
        self.pet_error.clear();
        self.open = true;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            self.open = false;
            cx.notify();
        }
    }

    fn current(&self, cx: &App) -> Look {
        Look {
            shape: self.shape.clone(),
            color: Some(self.picked_color.clone()).filter(|c| !c.is_empty()),
            image: self.image.clone(),
            title: self.title.read(cx).value().to_string(),
            description: self.description.read(cx).value().to_string(),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error.clear();
        let (name, opened, current) = (self.bot_name.clone(), self.opened.clone(), self.current(cx));
        self.state.update(cx, |s, cx| s.edit_save(&name, opened, current, cx));
        cx.notify();
    }

    fn pick_shape(&mut self, shape: String, cx: &mut Context<Self>) {
        self.image = None;
        self.shape = shape;
        cx.notify();
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        // A stale "unavailable" is re-checked on every visit.
        if tab == Tab::Generate && self.can_generate != Some(true) {
            self.can_generate = None;
            self.state.update(cx, |s, _| s.edit_probe_generate());
        }
        if tab == Tab::Pet && !self.pets_loaded && !self.pets_loading {
            self.pets_loading = true;
            self.state.update(cx, |s, cx| s.edit_load_pets(cx));
        }
        cx.notify();
    }

    fn face(&self, shape: &str, size: f32, window: &Window) -> Option<PathBuf> {
        faces::face_file(shape, &self.picked_color, &self.bot_name, (size * window.scale_factor()).round() as u32)
    }

    fn ranked_pets(&self, cx: &App) -> Vec<(String, String, String)> {
        let q = self.pet_search.read(cx).value().trim().to_lowercase();
        if q.is_empty() {
            return self.pets.clone();
        }
        self.pets.iter().filter(|(slug, name, _)| name.to_lowercase().contains(&q) || slug.contains(&q)).cloned().collect()
    }

    fn thumb(&mut self, slug: &str, url: &str, cx: &mut Context<Self>) {
        if !self.thumbs.contains_key(slug) {
            self.thumbs.insert(slug.to_string(), None);
            self.state.update(cx, |s, _| s.edit_pet_thumb(slug, url));
        }
    }

    fn face_tile(&self, key: ElementId, picture: Option<PathBuf>, label: &str, tip: String, selected: bool, cx: &App) -> Stateful<Div> {
        let c = colors(cx);
        let hover = c.raised;
        div()
            .id(key)
            .size(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(c.radius)
            .hover(move |s| s.bg(hover))
            .when(selected, |d| d.border_1().border_color(c.accent))
            .cursor_pointer()
            .when_some(picture, |d, p| d.child(img(p).size(px(32.))))
            .when(!label.is_empty(), |d| d.child(div().text_size(px(10.)).text_color(c.muted).child(label.to_string())))
            .when(!tip.is_empty(), |d| d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)))
    }

    fn render_bot_tab(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let c = colors(cx).clone();
        let (is_blob, seed, kind) = faces::blob_parts(&self.shape);
        let no_image = self.image.is_none();
        if is_blob {
            let mut kinds = vec![String::new()];
            kinds.extend(self.kinds.iter().cloned());
            let tiles = kinds.into_iter().enumerate().map(|(ix, k)| {
                let shape = faces::blob_shape(&seed, &k);
                let picture = (!k.is_empty()).then(|| self.face(&shape, 32., window)).flatten();
                let tip = if k.is_empty() { "Auto — the name decides".to_string() } else { k.clone() };
                self.face_tile(id("kind", ix), picture, if k.is_empty() { "Auto" } else { "" }, tip, k == kind && no_image, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_shape(shape.clone(), cx)))
            });
            let locked = !seed.is_empty();
            let (kind_r, kind_l) = (kind.clone(), kind.clone());
            let name = self.bot_name.clone();
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.))
                .child(div().grid().grid_cols(4).gap(px(6.)).children(tiles))
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(widgets::text_link("randomize", "Randomize", &c).on_click(cx.listener(move |this, _, _, cx| {
                            this.pick_shape(faces::blob_shape(&faces::random_seed(), &kind_r), cx)
                        })))
                        .child(div().w(px(12.)))
                        .child(widgets::text_link("lock-face", if locked { "Unlock" } else { "Lock face" }, &c).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.shape = faces::blob_shape(if locked { "" } else { &name }, &kind_l);
                                cx.notify();
                            },
                        ))),
                )
                .child(muted(if locked { "Face locked — renaming won’t change it." } else { "Face follows the name." }, 11., &c))
                .child(widgets::text_link("classic-shapes", "Classic shapes", &c).on_click(cx.listener(|this, _, _, cx| {
                    this.shape = faces::default_shape(&this.bot_name);
                    cx.notify();
                })))
                .into_any_element()
        } else {
            let mut shapes = vec!["blobatar".to_string()];
            shapes.extend(self.shapes.iter().cloned());
            let tiles = shapes.into_iter().enumerate().map(|(ix, shape)| {
                let tip = if shape == "blobatar" { "Blob face — drawn from the bot’s name".to_string() } else { shape.clone() };
                let picture = self.face(&shape, 32., window);
                let selected = shape == self.shape && no_image;
                self.face_tile(id("shape", ix), picture, "", tip, selected, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_shape(shape.clone(), cx)))
            });
            let swatches = self.swatches.iter().enumerate().map(|(ix, sw)| {
                let on = *sw == self.picked_color;
                let color = hex(&faces::face_color(sw, &self.bot_name));
                let sw = sw.clone();
                div()
                    .id(id("swatch", ix))
                    .size(px(22.))
                    .rounded_full()
                    .bg(color)
                    .when(on, |d| d.border_2().border_color(c.foreground))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.picked_color = sw.clone();
                        cx.notify();
                    }))
            });
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .child(div().grid().grid_cols(4).gap(px(6.)).children(tiles))
                .child(div().grid().grid_cols(6).gap(px(8.)).children(swatches))
                .child(widgets::text_link("match-name", "Match the name", &c).on_click(cx.listener(|this, _, _, cx| {
                    this.picked_color.clear();
                    cx.notify();
                })))
                .into_any_element()
        }
    }

    fn render_generate_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = colors(cx).clone();
        let describe_empty = self.describe.read(cx).value().trim().is_empty();
        match self.can_generate {
            Some(true) => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(Textarea::new(&self.describe))
                .child(
                    widgets::button("generate", if self.generating { "Generating…" } else { "Generate" }, true, !self.generating, &c)
                        .w_full()
                        .when(!self.generating, |b| {
                            b.on_click(cx.listener(|this, _, _, cx| {
                                this.generating = true;
                                this.error.clear();
                                let describe = this.describe.read(cx).value().to_string();
                                let title = this.title.read(cx).value().to_string();
                                let description = this.description.read(cx).value().to_string();
                                let name = this.bot_name.clone();
                                this.state.update(cx, |s, cx| s.edit_generate(&describe, &name, &title, &description, cx));
                                cx.notify();
                            }))
                        }),
                )
                .when(describe_empty, |d| d.child(muted("Leave blank to generate from the bot’s name and description.", 11., &c).text_center()))
                .into_any_element(),
            Some(false) => muted("No image model available. If you just enabled one (or updated Hermes), restart the gateway.", 12., &c)
                .text_center()
                .into_any_element(),
            None => muted("Checking image backend…", 12., &c).text_center().into_any_element(),
        }
    }

    fn render_pet_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let c = colors(cx).clone();
        if self.pets_loading {
            return div().flex().justify_center().child(widgets::spinner(24.)).into_any_element();
        }
        let mut col = div().flex().flex_col().gap(px(8.));
        if !self.pet_error.is_empty() {
            col = col.child(div().text_size(px(12.)).text_color(c.danger).text_center().child(self.pet_error.clone()));
        }
        if self.pets_loaded && self.pets.is_empty() {
            return col.child(muted("No pets in the petdex gallery. Run `hermes pets` to explore.", 12., &c).text_center()).into_any_element();
        }
        if !self.pets_loaded {
            return col.into_any_element();
        }
        let ranked = self.ranked_pets(cx);
        let pages = ranked.len().div_ceil(PET_PAGE).max(1);
        self.page = self.page.min(pages - 1);
        let start = self.page * PET_PAGE;
        let visible: Vec<_> = ranked.iter().skip(start).take(PET_PAGE).cloned().collect();
        // This page's thumbnails, and the next pages' ahead of time.
        for (slug, _, url) in ranked.iter().skip(start).take(PET_PAGE * (1 + PRELOAD_PAGES)) {
            self.thumb(slug, url, cx);
        }
        let total = self.pets.len();
        let cells = visible.into_iter().map(|(slug, name, _)| {
            let selected = self.selected_slug == slug;
            let picture = self.thumbs.get(&slug).cloned().flatten();
            let hover = c.raised;
            let pick_slug = slug.clone();
            div()
                .id(id("pet", &slug))
                .h(px(72.))
                .p(px(4.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded(c.radius)
                .hover(move |s| s.bg(hover))
                .when(selected, |d| d.border_1().border_color(c.accent))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pet_error.clear();
                    this.selected_slug = pick_slug.clone();
                    match this.thumbs.get(&pick_slug).cloned().flatten() {
                        Some(path) => this.image = Some(path),
                        None => {
                            if let Some((_, _, url)) = this.pets.iter().find(|p| p.0 == pick_slug).cloned() {
                                this.thumb(&pick_slug, &url, cx);
                            }
                        }
                    }
                    cx.notify();
                }))
                .child(match picture {
                    Some(p) => img(p).size(px(40.)).object_fit(ObjectFit::Contain).into_any_element(),
                    None => div().size(px(40.)).rounded(c.radius).bg(c.surface).into_any_element(),
                })
                .child(div().w_full().truncate().text_center().text_size(px(10.)).text_color(c.muted).child(name))
        });
        col.child(muted("Pick a pet as this bot’s profile picture.", 11., &c).text_center())
            .child(div().child(Input::new(&self.pet_search)).child(muted(format!("{total} pets"), 10., &c)))
            .when(self.image.is_some() && !self.selected_slug.is_empty(), |d| {
                d.child(div().flex().justify_center().child(widgets::text_link("pet-remove", "Remove — back to shape avatar", &c).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.selected_slug.clear();
                        this.image = None;
                        cx.notify();
                    }),
                )))
            })
            .when(ranked.is_empty(), |d| d.child(muted("No pets match.", 12., &c).text_center()))
            .child(div().grid().grid_cols(PET_COLUMNS as u16).children(cells))
            .when(pages > 1, |d| {
                let page = self.page;
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(widgets::button("pet-prev", "‹ Prev", false, page > 0, &c).when(page > 0, |b| {
                            b.on_click(cx.listener(|this, _, _, cx| {
                                this.page = this.page.saturating_sub(1);
                                cx.notify();
                            }))
                        }))
                        .child(muted(format!("Page {} of {pages}", page + 1), 11., &c).flex_1().text_center())
                        .child(widgets::button("pet-next", "Next ›", false, page + 1 < pages, &c).when(page + 1 < pages, |b| {
                            b.on_click(cx.listener(|this, _, _, cx| {
                                this.page += 1;
                                cx.notify();
                            }))
                        })),
                )
            })
            .into_any_element()
    }
}

impl Render for EditBotDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx).clone();
        let preview = match &self.image {
            Some(path) => img(path.clone()).size(px(64.)).object_fit(ObjectFit::Cover).into_any_element(),
            None => match self.face(&self.shape, 64., window) {
                Some(path) => img(path).size(px(64.)).into_any_element(),
                None => div().size(px(64.)).into_any_element(),
            },
        };
        let tabs = [(Tab::Bot, "Bot"), (Tab::Generate, "Generate"), (Tab::Upload, "Upload"), (Tab::Pet, "Pet")].map(|(tab, label)| {
            let on = self.tab == tab;
            let hover = c.raised.alpha(0.5);
            div()
                .id(id("edit-tab", label))
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(c.radius)
                .when(on, |d| d.bg(c.raised).font_weight(FontWeight::SEMIBOLD))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if on { c.foreground } else { c.muted })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
        });
        let tab_body = match self.tab {
            Tab::Bot => self.render_bot_tab(window, cx),
            Tab::Generate => self.render_generate_tab(cx),
            Tab::Upload => widgets::button("upload", "Choose an image…", false, true, &c)
                .w_full()
                .on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |s, _| s.edit_pick_image())))
                .into_any_element(),
            Tab::Pet => self.render_pet_tab(cx),
        };
        let busy = self.busy;
        let content = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(20.))
                    .pt(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("Edit profile"))
                    .child(muted(format!("Appearance and role for {} ({}).", self.display_name, self.bot_name), 13., &c)),
            )
            .child(
                div()
                    .id("edit-body")
                    .px(px(20.))
                    .py(px(14.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(div().flex().justify_center().child(preview))
                    .child(
                        div().flex().justify_center().child(
                            div()
                                .p(px(3.))
                                .flex()
                                .gap(px(2.))
                                .rounded(c.radius)
                                .bg(c.background)
                                .border_1()
                                .border_color(c.border)
                                .children(tabs),
                        ),
                    )
                    .when(self.image.is_some() && self.tab != Tab::Generate, |d| {
                        d.child(div().flex().justify_center().child(widgets::text_link("remove-image", "Remove image — use shape", &c).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.image = None;
                                cx.notify();
                            }),
                        )))
                    })
                    .child(tab_body)
                    .child(div().flex().flex_col().gap(px(4.)).child(muted("Title", 12., &c)).child(Input::new(&self.title)))
                    .child(div().flex().flex_col().gap(px(4.)).child(muted("Description", 12., &c)).child(Textarea::new(&self.description))),
            )
            .when(!self.error.is_empty(), |d| d.child(div().px(px(20.)).text_size(px(13.)).text_color(c.danger).child(self.error.clone())))
            .child(
                div()
                    .px(px(20.))
                    .py(px(16.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        widgets::button("edit-cancel", "Cancel", false, !busy, &c)
                            .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.close(cx)))),
                    )
                    .child(
                        widgets::button("edit-save", if busy { "Saving…" } else { "Save" }, true, !busy, &c)
                            .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.save(cx)))),
                    ),
            );
        modal("edit-bot-dialog", 440., content, &c).on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
            if e.keystroke.key == "escape" {
                this.close(cx);
            }
        }))
    }
}
