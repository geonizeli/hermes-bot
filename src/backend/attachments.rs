//! Files and images attached to a message: picking (xdg portal), pasting
//! (the clipboard, read by the bridge), uploading to the Bot Chat, and the
//! `@image:` / `@file:` references Hermes keeps in the transcript.
//!
//! A remote dashboard has no multipart upload route, so everything travels as
//! bytes over the chat socket: images through `image.attach_bytes` (queued
//! for the next prompt), other files through `file.attach` with a data URL,
//! whose `@file:` reference goes at the top of the prompt.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use super::gateway::Gateway;

/// Images the server accepts (25 MiB).
pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
/// Other files travel base64 inside one WebSocket frame; keep them modest.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Pastes longer than this become a text file, like the official app.
pub const LARGE_PASTE_CHARS: usize = 3000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    File,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub id: u64,
    pub kind: Kind,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

impl Attachment {
    pub fn to_view(&self) -> Value {
        json!({
            "id": self.id,
            "kind": if self.kind == Kind::Image { "image" } else { "file" },
            "name": self.name,
            "url": url::Url::from_file_path(&self.path).map(|u| u.to_string()).unwrap_or_default(),
            "size": self.size,
        })
    }
}

/// Image formats by their first bytes, as the server sniffs them.
pub fn image_extension(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("webp")
    } else if head.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

/// A local path from a `file://` URL (drops, portal results) or a plain path.
pub fn path_from_url(text: &str) -> Option<PathBuf> {
    if let Ok(url) = url::Url::parse(text) {
        if url.scheme() == "file" {
            return url.to_file_path().ok();
        }
        return None;
    }
    let path = PathBuf::from(text);
    path.is_absolute().then_some(path)
}

pub fn from_path(id: u64, path: &Path) -> Result<Attachment, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let meta = std::fs::metadata(path).map_err(|e| format!("Can't read {name}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{name} is a folder; attach the files inside it."));
    }
    let mut head = [0u8; 16];
    let read = {
        use std::io::Read;
        std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0)
    };
    let kind = if image_extension(&head[..read]).is_some() { Kind::Image } else { Kind::File };
    let limit = if kind == Kind::Image { MAX_IMAGE_BYTES } else { MAX_FILE_BYTES };
    if meta.len() > limit {
        return Err(format!("{name} is too large ({} MB; the limit is {} MB).", meta.len() / 1_048_576, limit / 1_048_576));
    }
    Ok(Attachment { id, kind, name, path: path.to_path_buf(), size: meta.len() })
}

fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "md" | "log" | "csv" | "json" | "yaml" | "yml" | "toml" | "rs" | "py" | "js" | "ts" | "sh" | "html"
        | "css" | "xml" | "sql" => "text/plain",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Uploads one attachment into the session. Returns the `@file:` reference
/// for files; images are queued server-side for the next prompt.
pub async fn upload(gateway: &Gateway, session_id: &str, attachment: &Attachment) -> Result<Option<String>, String> {
    let bytes = tokio::fs::read(&attachment.path)
        .await
        .map_err(|e| format!("Can't read {}: {e}", attachment.name))?;
    match attachment.kind {
        Kind::Image => {
            let ext = image_extension(&bytes).unwrap_or("png");
            gateway
                .request(
                    "image.attach_bytes",
                    json!({
                        "session_id": session_id,
                        "content_base64": BASE64.encode(&bytes),
                        "filename": attachment.name,
                        "ext": ext,
                    }),
                )
                .await
                .map_err(|e| format!("Couldn't attach {}: {e}", attachment.name))?;
            Ok(None)
        }
        Kind::File => {
            let data_url = format!("data:{};base64,{}", mime_for(&attachment.name), BASE64.encode(&bytes));
            let result = gateway
                .request("file.attach", json!({ "session_id": session_id, "name": attachment.name, "data_url": data_url }))
                .await
                .map_err(|e| format!("Couldn't attach {}: {e}", attachment.name))?;
            Ok(result.get("ref_text").and_then(Value::as_str).map(str::to_string))
        }
    }
}

/// The prompt as Hermes expects it: file references first, then the text.
pub fn compose_prompt(text: &str, refs: &[String], has_images: bool) -> String {
    let text = text.trim();
    let body = if text.is_empty() && has_images && refs.is_empty() {
        "What do you see in this image?".to_string()
    } else {
        text.to_string()
    };
    if refs.is_empty() {
        body
    } else if body.is_empty() {
        refs.join("\n")
    } else {
        format!("{}\n\n{body}", refs.join("\n"))
    }
}

/// Splits a stored user message into its text and its `@image:`/`@file:`
/// references (`caption\n\n@image:<path>` / leading `@file:` lines).
pub fn split_refs(text: &str) -> (String, Vec<Value>) {
    let mut refs = Vec::new();
    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let found = [("@image:", "image"), ("@file:", "file")].iter().find_map(|(prefix, kind)| {
            trimmed.strip_prefix(prefix).filter(|p| !p.trim().is_empty()).map(|p| (p.trim(), *kind))
        });
        match found {
            Some((path, kind)) => {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                refs.push(json!({ "kind": kind, "name": name, "path": path }));
            }
            None => kept.push(line),
        }
    }
    (kept.join("\n").trim().to_string(), refs)
}

/// Where pasted images and long pastes are written before sending.
pub fn scratch_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("hermes-bot");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub enum Pasted {
    /// A file was made from the clipboard (an image, or a long text).
    File(PathBuf),
    /// Ordinary text: let the text box paste it.
    Text,
    Nothing,
}

/// What the clipboard offers, read by the caller (the bridge, through Qt).
pub trait Clipboard {
    fn formats(&self) -> Vec<String>;
    fn data(&self, format: &str) -> Vec<u8>;
    fn text(&self) -> String;
}

/// Turns images and long texts on the clipboard into files to attach.
pub fn paste_clipboard(clipboard: &impl Clipboard, dir: &Path) -> Result<Pasted, String> {
    let types = clipboard.formats();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    for (mime, ext) in [("image/png", "png"), ("image/jpeg", "jpg"), ("image/webp", "webp"), ("image/gif", "gif")] {
        if types.iter().any(|t| t.trim() == mime) {
            let bytes = clipboard.data(mime);
            if bytes.is_empty() {
                break;
            }
            let path = dir.join(format!("pasted-image-{stamp}.{ext}"));
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            return Ok(Pasted::File(path));
        }
    }
    let text = clipboard.text();
    if text.is_empty() {
        return Ok(Pasted::Nothing);
    }
    if text.chars().count() > LARGE_PASTE_CHARS {
        let path = dir.join(format!("pasted_content_{stamp}.txt"));
        std::fs::write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
        return Ok(Pasted::File(path));
    }
    Ok(Pasted::Text)
}

/// Opens the desktop's file chooser through the xdg portal.
pub async fn pick_files() -> Result<Vec<PathBuf>, String> {
    use ashpd::desktop::file_chooser::SelectedFiles;
    let request = SelectedFiles::open_file()
        .title("Attach files")
        .modal(true)
        .multiple(true)
        .send()
        .await
        .map_err(|e| format!("Couldn't open the file chooser: {e}"))?;
    match request.response() {
        Ok(files) => Ok(files.uris().iter().filter_map(|u| path_from_url(u.as_str())).collect()),
        // Cancelled.
        Err(ashpd::Error::Response(_)) => Ok(Vec::new()),
        Err(e) => Err(format!("The file chooser failed: {e}")),
    }
}

/// One picture through the xdg file chooser; `None` when cancelled.
pub async fn pick_image() -> Result<Option<PathBuf>, String> {
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    let images = ["image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp"]
        .into_iter()
        .fold(FileFilter::new("Images"), |f, mime| f.mimetype(mime));
    let request = SelectedFiles::open_file()
        .title("Choose a picture")
        .modal(true)
        .multiple(false)
        .filter(images)
        .send()
        .await
        .map_err(|e| format!("Couldn't open the file chooser: {e}"))?;
    match request.response() {
        Ok(files) => Ok(files.uris().iter().find_map(|u| path_from_url(u.as_str()))),
        Err(ashpd::Error::Response(_)) => Ok(None),
        Err(e) => Err(format!("The file chooser failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_images() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(image_extension(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(image_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_extension(b"hello"), None);
    }

    #[test]
    fn reads_drop_urls() {
        assert_eq!(path_from_url("file:///tmp/a%20b.png"), Some(PathBuf::from("/tmp/a b.png")));
        assert_eq!(path_from_url("/tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(path_from_url("https://example.com/x"), None);
    }

    #[test]
    fn classifies_and_limits_files() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("shot.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n0000").unwrap();
        let txt = dir.path().join("notes.txt");
        std::fs::write(&txt, b"hello").unwrap();
        assert_eq!(from_path(1, &png).unwrap().kind, Kind::Image);
        assert_eq!(from_path(2, &txt).unwrap().kind, Kind::File);
        assert!(from_path(3, dir.path()).is_err());
    }

    #[test]
    fn composes_prompts() {
        assert_eq!(compose_prompt("hi", &[], false), "hi");
        assert_eq!(compose_prompt("", &[], true), "What do you see in this image?");
        assert_eq!(compose_prompt("read", &["@file:a.txt".into()], false), "@file:a.txt\n\nread");
    }

    #[test]
    fn splits_stored_references() {
        let (text, refs) = split_refs("@file:/srv/notes.txt\nlook at these\n\n@image:/home/u/.hermes/images/a.png");
        assert_eq!(text, "look at these");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0]["kind"], "file");
        assert_eq!(refs[1]["name"], "a.png");
    }

    struct FakeClipboard {
        formats: Vec<&'static str>,
        data: Vec<u8>,
        text: String,
    }

    impl Clipboard for FakeClipboard {
        fn formats(&self) -> Vec<String> {
            self.formats.iter().map(|f| f.to_string()).collect()
        }
        fn data(&self, _format: &str) -> Vec<u8> {
            self.data.clone()
        }
        fn text(&self) -> String {
            self.text.clone()
        }
    }

    #[test]
    fn pastes_images_long_texts_and_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let image = FakeClipboard { formats: vec!["image/png"], data: b"\x89PNG".to_vec(), text: String::new() };
        let Ok(Pasted::File(path)) = paste_clipboard(&image, dir.path()) else { panic!("image not pasted") };
        assert_eq!(path.extension().unwrap(), "png");
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");

        let long = FakeClipboard { formats: vec!["text/plain"], data: vec![], text: "x".repeat(LARGE_PASTE_CHARS + 1) };
        let Ok(Pasted::File(path)) = paste_clipboard(&long, dir.path()) else { panic!("long text not pasted") };
        assert_eq!(path.extension().unwrap(), "txt");

        let short = FakeClipboard { formats: vec!["text/plain"], data: vec![], text: "hi".into() };
        assert!(matches!(paste_clipboard(&short, dir.path()), Ok(Pasted::Text)));

        let empty = FakeClipboard { formats: vec![], data: vec![], text: String::new() };
        assert!(matches!(paste_clipboard(&empty, dir.path()), Ok(Pasted::Nothing)));
    }
}
