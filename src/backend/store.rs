//! The server list (a JSON file) and each server's tokens (the keyring).
//!
//! Tokens go to the desktop's Secret Service keyring (GNOME Keyring,
//! KWallet, KeePassXC…). Without one, they're kept in memory instead: signing
//! in still works, and lasts until the app quits.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub url: String,
    /// The dashboard auth provider signed in with (`basic`, `nous`, ...).
    #[serde(default)]
    pub provider: String,
    /// Username for password providers, so re-signing in only asks for the password.
    #[serde(default)]
    pub username: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub active_server: Option<String>,
    /// Last bot (Hermes profile) used, per server id.
    #[serde(default)]
    pub last_bot: HashMap<String, String>,
    /// Window state remembered between runs.
    #[serde(default)]
    pub ui: UiPrefs,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UiPrefs {
    /// The chat's side panel is open.
    #[serde(default = "yes")]
    pub side_panel_open: bool,
    /// "jobs" or "screen".
    #[serde(default = "jobs_tab")]
    pub side_panel_tab: String,
    /// "system", "dark", "light" or "omarchy"; unset means the default
    /// (Omarchy's theme where it's installed, otherwise the system's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

fn yes() -> bool {
    true
}

fn jobs_tab() -> String {
    "jobs".into()
}

impl Default for UiPrefs {
    fn default() -> Self {
        UiPrefs { side_panel_open: true, side_panel_tab: jobs_tab(), theme: None }
    }
}

impl Config {
    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("hermes-bot/servers.json")
}

/// A missing or unreadable file is an empty list; a corrupt one is reported
/// so the user's servers aren't silently dropped on the next save.
pub fn load(path: &Path) -> Result<Config, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{} is not valid: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("Can't read {}: {e}", path.display())),
    }
}

/// Writes beside the target and renames over it, so a crash never leaves a
/// half-written file.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    let dir = path.parent().ok_or("config path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(serde_json::to_string_pretty(config).unwrap_or_default().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    };
    write().map_err(|e| format!("Can't save {}: {e}", path.display()))
}

/// Accepts `host`, `host:port` or a full URL; returns `scheme://host[:port][/prefix]`
/// without a trailing slash. Defaults to https.
pub fn normalize_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter the server address.".into());
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = url::Url::parse(&with_scheme).map_err(|_| format!("\"{input}\" isn't a valid address."))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The address must start with https:// or http://.".into());
    }
    if url.host_str().is_none() {
        return Err(format!("\"{input}\" has no host."));
    }
    let mut out = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        out.push_str(&format!(":{port}"));
    }
    out.push_str(url.path().trim_end_matches('/'));
    Ok(out)
}

/// A readable default name for a server: its host.
pub fn default_name(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.to_string())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub provider: String,
}

impl Tokens {
    pub fn expires_soon(&self) -> bool {
        self.expires_at - 60 <= now()
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const KEYRING_SERVICE: &str = "hermes-bot";

/// Sign-ins the keyring couldn't take, by server id, for this run only.
static IN_MEMORY: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);

fn in_memory<R>(f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
    let mut guard = IN_MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

fn entry(server_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, server_id).map_err(|e| format!("Keyring unavailable: {e}"))
}

pub fn load_tokens(server_id: &str) -> Option<Tokens> {
    let secret = in_memory(|m| m.get(server_id).cloned())
        .or_else(|| entry(server_id).ok()?.get_password().ok())?;
    serde_json::from_str(&secret).ok()
}

/// Stores a sign-in in the keyring, or in memory when there's no keyring
/// (see `is_in_memory`).
pub fn save_tokens(server_id: &str, tokens: &Tokens) -> Result<(), String> {
    let json = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
    save_secret(server_id, json, |json| entry(server_id)?.set_password(json).map_err(|e| e.to_string()));
    Ok(())
}

/// `keyring` stores the secret for good; when it fails the secret is kept
/// for this run. Once a server's sign-in is in memory it stays there, so a
/// refreshed token doesn't land in a keyring that holds an older one.
fn save_secret(server_id: &str, json: String, keyring: impl FnOnce(&str) -> Result<(), String>) {
    let held = in_memory(|m| m.contains_key(server_id));
    if held || keyring(&json).is_err() {
        in_memory(|m| m.insert(server_id.to_string(), json));
    }
}

/// The server's sign-in lasts only until the app quits: there was no
/// keyring to keep it in.
pub fn is_in_memory(server_id: &str) -> bool {
    in_memory(|m| m.contains_key(server_id))
}

pub fn delete_tokens(server_id: &str) {
    in_memory(|m| m.remove(server_id));
    if let Ok(entry) = entry(server_id) {
        let _ = entry.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_sign_ins_in_memory_without_a_keyring() {
        let mut stored = None;
        save_secret("with-keyring", "a".into(), |json| {
            stored = Some(json.to_string());
            Ok(())
        });
        assert_eq!(stored.as_deref(), Some("a"));
        assert!(!is_in_memory("with-keyring"));

        save_secret("no-keyring", "b".into(), |_| Err("no Secret Service".into()));
        assert!(is_in_memory("no-keyring"));
        // Later refreshes stay in memory too, even if a keyring appears.
        save_secret("no-keyring", "c".into(), |_| panic!("the keyring isn't asked again"));
        assert_eq!(in_memory(|m| m.get("no-keyring").cloned()).as_deref(), Some("c"));

        delete_tokens("no-keyring");
        assert!(!is_in_memory("no-keyring"));
    }

    #[test]
    fn normalizes_addresses() {
        assert_eq!(normalize_url("hermes.example.com").unwrap(), "https://hermes.example.com");
        assert_eq!(normalize_url(" https://h.example/ ").unwrap(), "https://h.example");
        assert_eq!(normalize_url("http://10.0.0.2:9119/hermes/").unwrap(), "http://10.0.0.2:9119/hermes");
        assert!(normalize_url("").is_err());
        assert!(normalize_url("ftp://x").is_err());
    }

    #[test]
    fn saves_and_loads_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/servers.json");
        assert_eq!(load(&path).unwrap(), Config::default());
        let mut config = Config::default();
        config.servers.push(Server {
            id: "a".into(),
            name: "A".into(),
            url: "https://a".into(),
            provider: "basic".into(),
            username: "me".into(),
        });
        config.active_server = Some("a".into());
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn reports_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("servers.json");
        std::fs::write(&path, "{nope").unwrap();
        assert!(load(&path).is_err());
    }
}
