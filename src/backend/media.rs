//! Files that live on the server (images in the transcript, images the bot
//! made), fetched over REST and cached on disk so QML can show them.

use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::auth;
use super::session::Session;

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    base.join("hermes-bot/media")
}

fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}

/// Splits `data:<mime>;base64,<data>`.
pub fn decode_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    let rest = data_url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.split(';').next().unwrap_or_default().to_string();
    let bytes = if meta.ends_with(";base64") { BASE64.decode(data.trim()).ok()? } else { data.as_bytes().to_vec() };
    Some((mime, bytes))
}

/// A local copy of `path` from the server, fetched once and kept in the cache.
pub async fn fetch(session: &Session, path: &str, profile: &str) -> Result<PathBuf, String> {
    let key: String = Sha256::digest(format!("{}\n{profile}\n{path}", session.base_url).as_bytes())
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect();
    let dir = cache_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&key) {
                return Ok(entry.path());
            }
        }
    }
    let mut url = url::Url::parse(&format!("{}/api/fs/read-data-url", session.base_url)).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("path", path);
    if !profile.is_empty() {
        url.query_pairs_mut().append_pair("profile", profile);
    }
    let response = auth::http()
        .get(url)
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|e| format!("Couldn't fetch {path}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Couldn't fetch {path} (HTTP {}).", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let data_url = body
        .get("dataUrl")
        .or_else(|| body.get("data_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("The server sent no data for {path}."))?;
    let (mime, bytes) = decode_data_url(data_url).ok_or_else(|| format!("Bad data for {path}."))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{key}.{}", extension_for(&mime)));
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    Ok(file)
}

fn avatar_prefix(server: &str, profile: &str) -> String {
    let key: String = Sha256::digest(format!("avatar\n{server}\n{profile}").as_bytes())
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("avatar-{key}-")
}

/// The avatar last saved for a bot, so it shows before the server answers.
pub fn cached_avatar(server: &str, profile: &str) -> Option<PathBuf> {
    let prefix = avatar_prefix(server, profile);
    std::fs::read_dir(cache_dir())
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// Saves a bot's avatar (a data URL from `profiles.get_asset`). The file name
/// carries a hash of the image, so a new avatar gets a new URL and Qt's image
/// cache can't show the old one; older copies are removed.
pub fn store_avatar(server: &str, profile: &str, data_url: &str) -> Option<PathBuf> {
    let (mime, bytes) = decode_data_url(data_url)?;
    let prefix = avatar_prefix(server, profile);
    let hash: String = Sha256::digest(&bytes).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let name = format!("{prefix}{hash}.{}", extension_for(&mime));
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(&name);
    if !file.exists() {
        std::fs::write(&file, bytes).ok()?;
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let other = entry.file_name().to_string_lossy().to_string();
            if other.starts_with(&prefix) && other != name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Some(file)
}

/// Forgets a bot's avatar after the server says it has none.
pub fn clear_avatar(server: &str, profile: &str) {
    let prefix = avatar_prefix(server, profile);
    if let Ok(entries) = std::fs::read_dir(cache_dir()) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_data_urls() {
        let (mime, bytes) = super::decode_data_url("data:image/png;base64,aGk=").unwrap();
        assert_eq!((mime.as_str(), bytes.as_slice()), ("image/png", &b"hi"[..]));
        assert!(super::decode_data_url("nope").is_none());
    }
}
