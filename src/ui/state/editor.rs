//! Edit Bot: what the editor dialog needs, backed by `backend::botedit` and
//! `backend::face`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use gpui_kit::Context;
use rand::RngExt;
use serde_json::{Value, json};

use super::{AppEvent, AppState};
use crate::backend::botedit::{self, Look};
use crate::backend::face::{self, BlobKind};
use crate::backend::{attachments, media, runtime};

/// What the editor opens with.
pub struct EditorInfo {
    pub name: String,
    pub look: Look,
    pub shapes: Vec<String>,
    pub kinds: Vec<String>,
    pub swatches: Vec<String>,
}

/// A rendered face PNG for (shape, colour pick, name, pixels), cached.
pub fn face_file(shape: &str, color: &str, name: &str, px: u32) -> Option<PathBuf> {
    static CACHE: OnceLock<Mutex<HashMap<(String, String, String, u32), Option<PathBuf>>>> = OnceLock::new();
    let color = face::avatar_color(Some(color), name);
    let px = px.clamp(16, 1024);
    let key = (shape.to_string(), color.clone(), name.to_string(), px);
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(key).or_insert_with(|| botedit::face_file(shape, &color, name, px).ok()).clone()
}

/// The colour a face is drawn in: the pick, else the name's hue (`#rrggbb`).
pub fn face_color(color: &str, name: &str) -> String {
    face::css_to_hex(&face::avatar_color(Some(color), name))
}

/// (is a blob face, seed, kind name) for a shape string.
pub fn blob_parts(shape: &str) -> (bool, String, String) {
    let (seed, kind) = face::parse_blob_shape(shape);
    (face::is_blob_shape(shape), seed, kind.map(|k| k.name().to_string()).unwrap_or_default())
}

pub fn blob_shape(seed: &str, kind: &str) -> String {
    face::blob_shape_string(seed, BlobKind::parse(kind))
}

/// Eight base-36 characters, like the desktop's `Math.random().toString(36)`.
pub fn random_seed() -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::rng();
    (0..8).map(|_| DIGITS[rng.random_range(0..DIGITS.len())] as char).collect()
}

pub fn default_shape(name: &str) -> String {
    face::default_shape_for(name).to_string()
}

impl AppState {
    pub fn edit_bot_open(&self, name: &str) -> EditorInfo {
        let row = self.roster_rows.get(name).cloned().unwrap_or_else(|| json!({ "name": name }));
        let server_id = self.config.active_server.clone().unwrap_or_default();
        let avatar = media::cached_avatar(&server_id, name);
        EditorInfo {
            name: name.to_string(),
            look: botedit::opening_look(&row, avatar.as_deref()),
            shapes: face::PICKER_SHAPES.iter().map(|s| s.to_string()).collect(),
            kinds: BlobKind::ALL.iter().map(|k| k.name().to_string()).collect(),
            swatches: face::swatches(),
        }
    }

    pub fn edit_pick_image(&mut self) {
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = match attachments::pick_image().await {
                Ok(Some(path)) => tokio::task::spawn_blocking(move || botedit::upload(&path))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
                    .map(Some),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            };
            queue.queue(move |_, cx| match result {
                Ok(Some(file)) => cx.emit(AppEvent::EditImageReady(Ok(file))),
                Ok(None) => {}
                Err(e) => cx.emit(AppEvent::EditImageReady(Err(e))),
            });
        });
    }

    pub fn edit_probe_generate(&mut self) {
        let Some(gateway) = self.gateway.clone() else { return };
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let available = botedit::can_generate(&gateway).await;
            queue.queue(move |_, cx| cx.emit(AppEvent::EditGenerateAvailable(available)));
        });
    }

    pub fn edit_generate(&mut self, describe: &str, name: &str, title: &str, description: &str, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else {
            cx.emit(AppEvent::EditImageReady(Err("Not connected.".into())));
            return;
        };
        let prompt = botedit::generate_prompt(describe, name, title, description);
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = botedit::generate(&gateway, &prompt).await;
            queue.queue(move |_, cx| cx.emit(AppEvent::EditImageReady(result)));
        });
    }

    pub fn edit_load_pets(&mut self, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else {
            cx.emit(AppEvent::EditPetsLoaded(Err("Not connected.".into())));
            return;
        };
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = botedit::pet_gallery(&gateway).await;
            queue.queue(move |_, cx| cx.emit(AppEvent::EditPetsLoaded(result)));
        });
    }

    pub fn edit_pet_thumb(&mut self, slug: &str, url: &str) {
        let Some(gateway) = self.gateway.clone() else { return };
        let (slug, url) = (slug.to_string(), url.to_string());
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = botedit::pet_thumb(&gateway, &slug, &url).await;
            queue.queue(move |_, cx| cx.emit(AppEvent::EditPetReady(slug, result.ok())));
        });
    }

    pub fn edit_save(&mut self, name: &str, opened: Look, current: Look, cx: &mut Context<Self>) {
        let Some(gateway) = self.gateway.clone() else {
            cx.emit(AppEvent::EditSaved(Err("Not connected.".into())));
            return;
        };
        let name = name.to_string();
        let generation = self.generation;
        let queue = self.queue.clone();
        runtime().spawn(async move {
            let result = botedit::save(&gateway, &name, &opened, &current).await;
            queue.queue(move |app, cx| {
                if app.generation != generation {
                    return;
                }
                cx.emit(AppEvent::EditSaved(result));
                // Even a partial save changes what the roster shows.
                app.refresh_bots_with(true, cx);
            });
        });
    }
}

/// A pet gallery row.
pub fn pet_fields(v: &Value) -> (String, String, String) {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    (s("slug"), s("name"), s("url"))
}
