//! Edit Bot: what the official desktop's Edit Profile dialog does without its
//! Advanced section. A bot's title and look live in the profile's
//! `ui_meta["hermes-bots"]`, its description in the profile metadata, and its
//! picture in the profile's avatar asset.
//!
//! The editor works on local files: every picture it shows (a drawn face, an
//! upload, a generated image, a pet) is a PNG in the cache, and Save sends
//! the chosen one to the server.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::face;
use super::gateway::Gateway;
use super::media;
use super::roster::{self, META_NAMESPACE};

/// The size the desktop rasterizes a drawn face at before uploading it. A
/// 160×160 PNG avatar is how both apps recognize "this is only a copy of the
/// face" rather than a real picture.
pub const FACE_PNG: u32 = 160;
/// Uploads and generated images are cropped to this square.
pub const PHOTO_EDGE: u32 = 256;
/// How many times a save re-reads after another writer got in first.
const CAS_ATTEMPTS: usize = 3;
/// Image backends routinely take 40–60 s.
const GENERATE_DEADLINE: Duration = Duration::from_secs(90);

fn edit_dir() -> PathBuf {
    media::cache_dir().join("edit")
}

/// Writes `bytes` under a name derived from their content and returns the path.
fn keep(bytes: &[u8], ext: &str) -> Result<PathBuf, String> {
    let hash: String = Sha256::digest(bytes).iter().take(10).map(|b| format!("{b:02x}")).collect();
    let dir = edit_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{hash}.{ext}"));
    if !file.exists() {
        std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    }
    Ok(file)
}

/// A PNG of the face for `shape` and `color`, at `px` pixels.
pub fn face_file(shape: &str, color: &str, name: &str, px: u32) -> Result<PathBuf, String> {
    let svg = face::svg(shape, color, name, 40);
    let png = face::rasterize(&svg, px).ok_or("Couldn't draw the face.")?;
    keep(&png, "png")
}

/// True when the avatar file is the 160×160 PNG copy of a drawn face, not a
/// picture someone chose.
pub fn is_face_copy(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return false;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    width == FACE_PNG && height == FACE_PNG
}

/// A picture from this machine, cropped to the square the desktop stores.
pub fn upload(path: &Path) -> Result<PathBuf, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > 15_000_000 {
        return Err("Image too large (max 15 MB).".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let png = face::normalize_image(&bytes, PHOTO_EDGE).ok_or("That file isn't an image this app can read.")?;
    keep(&png, "png")
}

/// What the dialog shows for a bot, as it opened and as the user leaves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub shape: String,
    /// `None` follows the name's hue.
    pub color: Option<String>,
    /// A picture (local file) that replaces the drawn face.
    pub image: Option<PathBuf>,
    pub title: String,
    pub description: String,
}

/// The look the dialog opens with for a roster row and its cached avatar.
pub fn opening_look(row: &Value, avatar: Option<&Path>) -> Look {
    let name = row.get("name").and_then(Value::as_str).unwrap_or("default");
    let meta = roster::meta(row);
    let appearance = face::appearance(name, &meta);
    let image = avatar.filter(|p| !is_face_copy(p)).map(Path::to_path_buf);
    Look {
        shape: appearance.shape,
        color: appearance.color,
        image,
        title: meta.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        description: row.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
    }
}

/// The `ui_meta` fields the user actually changed (the desktop's
/// `editedLook`): untouched fields stay out so a save never reverts what
/// another client wrote since the dialog opened.
pub fn meta_patch(opened: &Look, current: &Look) -> Map<String, Value> {
    let mut patch = Map::new();
    if current.shape != opened.shape {
        patch.insert("shape".into(), json!(current.shape));
    }
    if current.color != opened.color {
        patch.insert("color".into(), current.color.as_ref().map_or(Value::Null, |c| json!(c)));
    }
    if current.image != opened.image {
        patch.insert("imageKind".into(), json!(if current.image.is_some() { "photo" } else { "shape" }));
    }
    if !patch.is_empty() {
        patch.insert("custom".into(), json!(true));
    }
    if current.title.trim() != opened.title.trim() {
        patch.insert("title".into(), json!(current.title.trim()));
    }
    patch
}

/// What the avatar asset becomes: a picture, the drawn face, or unchanged.
#[derive(Debug, PartialEq)]
pub enum AvatarChange {
    Keep,
    Picture(PathBuf),
    Face,
}

pub fn avatar_change(opened: &Look, current: &Look) -> AvatarChange {
    match &current.image {
        Some(path) if current.image != opened.image => AvatarChange::Picture(path.clone()),
        Some(_) => AvatarChange::Keep,
        None if opened.image.is_some() || current.shape != opened.shape || current.color != opened.color => AvatarChange::Face,
        None => AvatarChange::Keep,
    }
}

fn png_data_url(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mime = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8") {
        "image/jpeg"
    } else if bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        // The server takes PNG, JPEG and WebP only.
        let png = face::normalize_image(&bytes, PHOTO_EDGE).ok_or("Unsupported image.")?;
        return Ok(format!("data:image/png;base64,{}", BASE64.encode(png)));
    };
    Ok(format!("data:{mime};base64,{}", BASE64.encode(bytes)))
}

/// The roster row for `name`, freshly read.
async fn read_row(gateway: &Gateway, name: &str) -> Result<Value, String> {
    let list = gateway.request("profiles.list", json!({ "include_sessions": false })).await?;
    list.get("profiles")
        .and_then(Value::as_array)
        .and_then(|rows| rows.iter().find(|r| r.get("name").and_then(Value::as_str) == Some(name)))
        .cloned()
        .ok_or_else(|| format!("The bot \"{name}\" is gone from the server."))
}

/// Saves the dialog: title, look and description through
/// `profiles.configure`, then the avatar asset when what it shows changed.
pub async fn save(gateway: &Gateway, name: &str, opened: &Look, current: &Look) -> Result<(), String> {
    let patch = meta_patch(opened, current);
    let description = current.description.trim();
    let description_changed = description != opened.description.trim();

    if !patch.is_empty() || description_changed {
        let mut done = false;
        for _ in 0..CAS_ATTEMPTS {
            let mut params = json!({ "name": name });
            if description_changed {
                params["description"] = json!(description);
            }
            if !patch.is_empty() {
                let row = read_row(gateway, name).await?;
                let mut merged = roster::meta(&row).as_object().cloned().unwrap_or_default();
                merged.remove("chat");
                merged.extend(patch.clone());
                params["ui_meta"] = json!({ META_NAMESPACE: merged });
                if let Some(revision) = roster::meta_revision(&row) {
                    params["ui_meta_expected_revisions"] = json!({ META_NAMESPACE: revision });
                }
            }
            let reply = gateway.request("profiles.configure", params).await?;
            let applied = reply.get("applied").cloned().unwrap_or(Value::Null);
            if applied.get("ui_meta_conflicts").is_some_and(|c| !c.is_null()) {
                continue;
            }
            if !patch.is_empty() && applied.get("ui_meta") == Some(&json!(false)) {
                return Err("The server didn't save the title and look.".into());
            }
            if description_changed && applied.get("description") == Some(&json!(false)) {
                return Err("The server didn't save the description.".into());
            }
            done = true;
            break;
        }
        if !done {
            return Err("Someone else kept changing this bot; try again.".into());
        }
    }

    let data = match avatar_change(opened, current) {
        AvatarChange::Keep => return Ok(()),
        AvatarChange::Picture(path) => png_data_url(&path)?,
        AvatarChange::Face => {
            let color = face::avatar_color(current.color.as_deref(), name);
            let file = face_file(&current.shape, &color, name, FACE_PNG)?;
            png_data_url(&file)?
        }
    };
    gateway
        .request("profiles.set_asset", json!({ "name": name, "asset": "avatar", "data": data }))
        .await
        .map(|_| ())
        .map_err(|e| format!("The look was saved, but the picture wasn't: {e}"))
}

/// Whether the server can generate images.
pub async fn can_generate(gateway: &Gateway) -> bool {
    gateway
        .request("image.generate", json!({ "probe": true }))
        .await
        .ok()
        .and_then(|r| r.get("available").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// The prompt the desktop sends: the user's own words, or a mascot for the bot.
pub fn generate_prompt(describe: &str, name: &str, title: &str, description: &str) -> String {
    let describe = describe.trim();
    if !describe.is_empty() {
        return format!("{describe}. Avatar for an AI agent: centered, bold flat vector style, solid color background, no text.");
    }
    let who = [if title.trim().is_empty() { name } else { title.trim() }, description.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" — ");
    format!(
        "Cute minimal robot avatar for an AI agent named \"{who}\". \
         Friendly simple mascot face, bold flat vector style, solid color background, centered, no text."
    )
}

/// Generates an avatar on the server and returns it as a local square PNG.
pub async fn generate(gateway: &Gateway, prompt: &str) -> Result<PathBuf, String> {
    let reply = gateway
        .request_within("image.generate", json!({ "prompt": prompt, "aspect_ratio": "square" }), GENERATE_DEADLINE)
        .await?;
    if reply.get("success").and_then(Value::as_bool) != Some(true) {
        let why = reply.get("error").and_then(Value::as_str).unwrap_or("generation failed");
        return Err(format!("Couldn't generate an image: {why}"));
    }
    let data = reply
        .get("image_data")
        .and_then(Value::as_str)
        .filter(|d| d.starts_with("data:"))
        .ok_or("The server sent no image.")?;
    let (_, bytes) = media::decode_data_url(data).ok_or("The server sent a broken image.")?;
    let png = face::normalize_image(&bytes, PHOTO_EDGE).ok_or("The server sent an image this app can't read.")?;
    keep(&png, "png")
}

/// The petdex gallery, installed pets first, then curated ones.
pub async fn pet_gallery(gateway: &Gateway) -> Result<Vec<Value>, String> {
    let reply = gateway.request("pet.gallery", json!({})).await?;
    let mut pets: Vec<Value> = reply
        .get("pets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.get("slug").and_then(Value::as_str).is_some_and(|s| !s.is_empty()))
        .map(|p| {
            let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            json!({
                "slug": text("slug"),
                "name": Some(text("displayName")).filter(|n| !n.is_empty()).unwrap_or_else(|| text("slug")),
                "url": text("spritesheetUrl"),
                "rank": if p["installed"] == true { 0 } else if p["curated"] == true { 1 } else { 2 },
            })
        })
        .collect();
    pets.sort_by_key(|p| p["rank"].as_i64().unwrap_or(2));
    Ok(pets)
}

/// A pet's first frame as a local PNG, cropped by the server.
pub async fn pet_thumb(gateway: &Gateway, slug: &str, url: &str) -> Result<PathBuf, String> {
    let reply = gateway
        .request_within("pet.thumb", json!({ "slug": slug, "url": url }), Duration::from_secs(15))
        .await?;
    let data = reply
        .get("dataUri")
        .and_then(Value::as_str)
        .filter(|_| reply["ok"] == true)
        .ok_or("Couldn't load that pet.")?;
    let (_, bytes) = media::decode_data_url(data).ok_or("Couldn't load that pet.")?;
    keep(&bytes, "png")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(shape: &str, color: Option<&str>, image: Option<&str>, title: &str) -> Look {
        Look {
            shape: shape.into(),
            color: color.map(str::to_string),
            image: image.map(PathBuf::from),
            title: title.into(),
            description: String::new(),
        }
    }

    #[test]
    fn patches_only_what_changed() {
        let opened = look("circle", None, None, "");
        assert!(meta_patch(&opened, &opened).is_empty());

        let renamed = meta_patch(&opened, &look("circle", None, None, " Jarvis "));
        assert_eq!(Value::Object(renamed), json!({ "title": "Jarvis" }));

        let restyled = meta_patch(&opened, &look("pill", Some("#fff"), None, ""));
        assert_eq!(Value::Object(restyled), json!({ "shape": "pill", "color": "#fff", "custom": true }));

        let photo = meta_patch(&opened, &look("circle", None, Some("/a.png"), ""));
        assert_eq!(Value::Object(photo), json!({ "imageKind": "photo", "custom": true }));

        let back = meta_patch(&look("circle", Some("#fff"), Some("/a.png"), ""), &look("circle", None, None, ""));
        assert_eq!(Value::Object(back), json!({ "color": null, "imageKind": "shape", "custom": true }));
    }

    #[test]
    fn decides_the_avatar() {
        let face = look("circle", None, None, "");
        let photo = look("circle", None, Some("/a.png"), "");
        assert_eq!(avatar_change(&face, &face), AvatarChange::Keep);
        assert_eq!(avatar_change(&face, &photo), AvatarChange::Picture("/a.png".into()));
        assert_eq!(avatar_change(&photo, &photo), AvatarChange::Keep);
        assert_eq!(avatar_change(&photo, &face), AvatarChange::Face);
        assert_eq!(avatar_change(&face, &look("pill", None, None, "")), AvatarChange::Face);
        // A new color under a picture changes nothing the server shows.
        assert_eq!(avatar_change(&photo, &look("circle", Some("#fff"), Some("/a.png"), "")), AvatarChange::Keep);
    }

    #[test]
    fn tells_face_copies_from_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let face = dir.path().join("face.png");
        std::fs::write(&face, face::rasterize(&face::svg("circle", "#8b5cf6", "x", 40), FACE_PNG).unwrap()).unwrap();
        assert!(is_face_copy(&face));
        let photo = dir.path().join("photo.png");
        std::fs::write(&photo, face::rasterize(&face::svg("circle", "#8b5cf6", "x", 40), PHOTO_EDGE).unwrap()).unwrap();
        assert!(!is_face_copy(&photo));
    }

    #[test]
    fn prompts_match_the_desktop() {
        assert!(generate_prompt("a fox", "work", "", "").starts_with("a fox. Avatar for an AI agent"));
        assert!(generate_prompt("", "work", "Jarvis", "Mail only").contains("named \"Jarvis — Mail only\""));
        assert!(generate_prompt("", "work", "", "").contains("named \"work\""));
    }
}

/// Renames the `default` bot and back, against the real server:
/// `HERMES_BOT_EDIT_LIVE=1 cargo test edit_live -- --ignored --nocapture`.
#[cfg(test)]
mod live {
    use tokio::sync::mpsc;

    use super::*;
    use crate::backend::store;

    #[tokio::test]
    #[ignore]
    async fn edit_live() {
        if std::env::var("HERMES_BOT_EDIT_LIVE").is_err() {
            return;
        }
        let config = store::load(&store::config_path()).unwrap();
        let server = config.server(config.active_server.as_deref().unwrap()).unwrap().clone();
        let mut tokens = store::load_tokens(&server.id).expect("signed in");
        if tokens.expires_soon() {
            tokens = crate::backend::auth::refresh(&server.url, &tokens).await.ok().expect("refresh");
            store::save_tokens(&server.id, &tokens).unwrap();
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let gateway = Gateway::connect(&server.url, &tokens.access_token, tx).await.unwrap();

        let row = read_row(&gateway, "default").await.unwrap();
        println!("before: title={:?} revision={:?}", roster::meta(&row).get("title"), roster::meta_revision(&row));
        let opened = opening_look(&row, None);
        let renamed = Look { title: "Hermes Live Test".into(), ..opened.clone() };
        save(&gateway, "default", &opened, &renamed).await.unwrap();
        let row = read_row(&gateway, "default").await.unwrap();
        println!("after: name={} revision={:?}", roster::display_name(&row), roster::meta_revision(&row));
        assert_eq!(roster::display_name(&row), "Hermes Live Test");

        save(&gateway, "default", &renamed, &opened).await.unwrap();
        let row = read_row(&gateway, "default").await.unwrap();
        println!("restored: name={} meta={}", roster::display_name(&row), roster::meta(&row));
        assert_eq!(roster::display_name(&row), "Hermes");
    }
}
