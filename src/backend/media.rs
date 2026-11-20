//! Files that live on the server (images in the transcript, images the bot
//! made), fetched over REST and cached on disk so QML can show them.

use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::auth;
use super::session::Session;

fn cache_dir() -> PathBuf {
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

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_data_urls() {
        let (mime, bytes) = super::decode_data_url("data:image/png;base64,aGk=").unwrap();
        assert_eq!((mime.as_str(), bytes.as_slice()), ("image/png", &b"hi"[..]));
        assert!(super::decode_data_url("nope").is_none());
    }
}
