//! Edit Bot on `HermesApp`: the editor's QML calls, backed by
//! `backend::botedit` and `backend::face`.

use core::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use rand::RngExt;
use serde_json::{Value, json};

use super::{file_url, q, qobject};
use crate::backend::botedit::{self, Look};
use crate::backend::face::{self, BlobKind};
use crate::backend::{attachments, media, runtime};

fn look_json(look: &Look) -> Value {
    json!({
        "shape": look.shape,
        "color": look.color.clone().unwrap_or_default(),
        "image": look.image.as_deref().map(file_url).unwrap_or_default(),
        "title": look.title,
        "description": look.description,
    })
}

impl qobject::HermesApp {
    pub fn edit_bot_open(self: Pin<&mut Self>, name: QString) -> QString {
        let name = name.to_string();
        let row = self.rust().roster_rows.get(&name).cloned().unwrap_or_else(|| json!({ "name": name }));
        let server_id = self.rust().config.active_server.clone().unwrap_or_default();
        let avatar = media::cached_avatar(&server_id, &name);
        let look = botedit::opening_look(&row, avatar.as_deref());
        q(json!({
            "name": name,
            "look": look_json(&look),
            "shapes": face::PICKER_SHAPES,
            "kinds": BlobKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>(),
            "swatches": face::swatches(),
        })
        .to_string())
    }

    pub fn face_url(mut self: Pin<&mut Self>, shape: QString, color: QString, name: QString, px: i32) -> QString {
        let (shape, name) = (shape.to_string(), name.to_string());
        let color = face::avatar_color(Some(&color.to_string()), &name);
        let px = px.clamp(16, 1024) as u32;
        let key = (shape.clone(), color.clone(), name.clone(), px);
        if let Some(url) = self.rust().face_cache.get(&key) {
            return q(url);
        }
        let url = botedit::face_file(&shape, &color, &name, px).map(|f| file_url(&f)).unwrap_or_default();
        self.as_mut().rust_mut().face_cache.insert(key, url.clone());
        q(url)
    }

    pub fn face_color(&self, color: QString, name: QString) -> QString {
        q(face::css_to_hex(&face::avatar_color(Some(&color.to_string()), &name.to_string())))
    }

    pub fn blob_parts(&self, shape: QString) -> QString {
        let shape = shape.to_string();
        let (seed, kind) = face::parse_blob_shape(&shape);
        q(json!({ "blob": face::is_blob_shape(&shape), "seed": seed, "kind": kind.map(|k| k.name()).unwrap_or("") }).to_string())
    }

    pub fn blob_shape(&self, seed: QString, kind: QString) -> QString {
        q(face::blob_shape_string(&seed.to_string(), BlobKind::parse(&kind.to_string())))
    }

    /// Eight base-36 characters, like the desktop's `Math.random().toString(36)`.
    pub fn random_seed(&self) -> QString {
        const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let mut rng = rand::rng();
        q((0..8).map(|_| DIGITS[rng.random_range(0..DIGITS.len())] as char).collect::<String>())
    }

    pub fn default_shape(&self, name: QString) -> QString {
        q(face::default_shape_for(&name.to_string()))
    }

    pub fn edit_pick_image(self: Pin<&mut Self>) {
        let qt = self.qt_thread();
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
            let _ = qt.queue(move |app| match result {
                Ok(Some(file)) => app.edit_image_ready(q(file_url(&file)), q("")),
                Ok(None) => {}
                Err(e) => app.edit_image_ready(q(""), q(e)),
            });
        });
    }

    pub fn edit_probe_generate(self: Pin<&mut Self>) {
        let Some(gateway) = self.rust().gateway.clone() else { return };
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let available = botedit::can_generate(&gateway).await;
            let _ = qt.queue(move |app| app.edit_generate_available(available));
        });
    }

    pub fn edit_generate(self: Pin<&mut Self>, describe: QString, name: QString, title: QString, description: QString) {
        let Some(gateway) = self.rust().gateway.clone() else {
            self.edit_image_ready(q(""), q("Not connected."));
            return;
        };
        let prompt = botedit::generate_prompt(&describe.to_string(), &name.to_string(), &title.to_string(), &description.to_string());
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = botedit::generate(&gateway, &prompt).await;
            let _ = qt.queue(move |app| match result {
                Ok(file) => app.edit_image_ready(q(file_url(&file)), q("")),
                Err(e) => app.edit_image_ready(q(""), q(e)),
            });
        });
    }

    pub fn edit_load_pets(self: Pin<&mut Self>) {
        let Some(gateway) = self.rust().gateway.clone() else {
            self.edit_pets_loaded(q("[]"), q("Not connected."));
            return;
        };
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = botedit::pet_gallery(&gateway).await;
            let _ = qt.queue(move |app| match result {
                Ok(pets) => app.edit_pets_loaded(q(Value::Array(pets).to_string()), q("")),
                Err(e) => app.edit_pets_loaded(q("[]"), q(e)),
            });
        });
    }

    pub fn edit_pet_thumb(self: Pin<&mut Self>, slug: QString, url: QString) {
        let Some(gateway) = self.rust().gateway.clone() else { return };
        let (slug, url) = (slug.to_string(), url.to_string());
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = botedit::pet_thumb(&gateway, &slug, &url).await;
            let _ = qt.queue(move |app| {
                app.edit_pet_ready(q(&slug), q(result.map(|f| file_url(&f)).unwrap_or_default()));
            });
        });
    }

    pub fn edit_save(self: Pin<&mut Self>, name: QString, opened: QString, current: QString) {
        let Some(gateway) = self.rust().gateway.clone() else {
            self.edit_saved(false, q("Not connected."));
            return;
        };
        let name = name.to_string();
        let parse = |s: QString| Look::from_json(&serde_json::from_str(&s.to_string()).unwrap_or(Value::Null));
        let (opened, current) = (parse(opened), parse(current));
        let generation = self.rust().generation;
        let qt = self.qt_thread();
        runtime().spawn(async move {
            let result = botedit::save(&gateway, &name, &opened, &current).await;
            let _ = qt.queue(move |mut app| {
                if app.rust().generation != generation {
                    return;
                }
                match result {
                    Ok(()) => app.as_mut().edit_saved(true, q("")),
                    Err(e) => app.as_mut().edit_saved(false, q(e)),
                }
                // Even a partial save changes what the roster shows.
                app.as_mut().refresh_bots();
            });
        });
    }
}
