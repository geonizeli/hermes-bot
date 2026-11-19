//! Hermes Bot Mode: each bot (a Hermes profile) has ONE forever chat, the
//! session titled exactly "Bot Chat" on that profile. The title is the only
//! identity (the server keeps titles unique); there is no stored session-id
//! pointer, and recency never decides which chat a bot opens.
//! See hermes-agent `apps/desktop/src/AGENTS.md`, "Bot Mode".

use serde_json::{Value, json};

use super::gateway::Gateway;

pub const BOT_CHAT_TITLE: &str = "Bot Chat";

/// The session to resume for a bot: the Bot Chat row in a `session.list`
/// reply, followed to its live compression tip.
pub fn find_in_listing(result: &Value) -> Option<String> {
    result.get("sessions")?.as_array()?.iter().find_map(|row| {
        let title = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default();
        if title("title") != BOT_CHAT_TITLE && title("root_title") != BOT_CHAT_TITLE {
            return None;
        }
        let id = Some(title("resolved_id")).filter(|s| !s.is_empty()).unwrap_or_else(|| title("id"));
        (!id.is_empty()).then(|| id.to_string())
    })
}

/// The `canonical_session` that `profiles.list {include_sessions}` reports
/// for one profile row.
pub fn canonical_from_roster(profile_row: &Value) -> Option<String> {
    let canonical = profile_row.get("canonical_session")?;
    let id = |key: &str| canonical.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    id("resolved_id").or_else(|| id("id"))
}

/// Looks the bot's chat up by name. Fails closed: an error is returned as an
/// error, never as "no chat", because minting a second Bot Chat on a transient
/// failure is how a bot loses its history. An empty listing falls back to the
/// roster's `canonical_session`, for a backend that answers empty mid-restart.
pub async fn lookup(gateway: &Gateway, profile: &str, roster: Option<&str>) -> Result<Option<String>, String> {
    let result = gateway
        .request(
            "session.list",
            json!({ "profile": profile, "title": BOT_CHAT_TITLE, "limit": 200, "include_hidden": true }),
        )
        .await
        .map_err(|e| format!("Couldn't look up this bot's chat ({e}). Try again."))?;
    Ok(find_in_listing(&result).or_else(|| roster.map(str::to_string)))
}

/// An open conversation: the live id events carry, the stored id, and the
/// transcript when an existing chat was resumed.
pub struct Opened {
    pub live: String,
    pub stored: String,
    pub messages: Vec<Value>,
    pub running: bool,
}

pub async fn resume(gateway: &Gateway, profile: &str, stored: &str) -> Result<Opened, String> {
    let result = gateway
        .request("session.resume", json!({ "session_id": stored, "profile": profile }))
        .await?;
    Ok(Opened {
        live: result.get("session_id").and_then(Value::as_str).unwrap_or_default().to_string(),
        stored: stored.to_string(),
        messages: result.get("messages").and_then(Value::as_array).cloned().unwrap_or_default(),
        running: result.get("running").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Opens the bot's chat for sending, creating it on first use. Re-runs the
/// lookup before creating, and if another client claims the title between
/// our lookup and our write, adopts that chat instead of forking.
pub async fn open_or_create(gateway: &Gateway, profile: &str, roster: Option<&str>) -> Result<Opened, String> {
    if let Some(stored) = lookup(gateway, profile, roster).await? {
        return resume(gateway, profile, &stored).await;
    }
    let created = gateway
        .request(
            "session.create",
            json!({
                "profile": profile,
                "title": BOT_CHAT_TITLE,
                // Bot chats stay out of every session list; the bot is the only door.
                "hidden": true,
                // The chat always follows the profile's current model and provider.
                "follow_profile_config": true,
            }),
        )
        .await?;
    let live = created.get("session_id").and_then(Value::as_str).unwrap_or_default().to_string();
    let stored = created.get("stored_session_id").and_then(Value::as_str).unwrap_or_default().to_string();
    // session.create is lazy; titling now materializes the row and claims the
    // name before the first prompt, so a second open can't mint a duplicate.
    match gateway.request("session.title", json!({ "session_id": live, "title": BOT_CHAT_TITLE })).await {
        Ok(_) => Ok(Opened { live, stored, messages: Vec::new(), running: false }),
        Err(e) if e.to_lowercase().contains("already in use") => {
            let winner = lookup(gateway, profile, None)
                .await?
                .ok_or_else(|| "This bot's chat is being created elsewhere. Try again.".to_string())?;
            resume(gateway, profile, &winner).await
        }
        // Older servers may not title eagerly; the deferred title from
        // session.create still names the row on the first prompt.
        Err(_) => Ok(Opened { live, stored, messages: Vec::new(), running: false }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_bot_chat_by_title_not_recency() {
        let listing = json!({"sessions": [
            {"id": "newest", "title": "Something else"},
            {"id": "root", "resolved_id": "tip", "title": "Bot Chat"},
        ]});
        assert_eq!(find_in_listing(&listing).as_deref(), Some("tip"));
        assert_eq!(find_in_listing(&json!({"sessions": [{"id": "x", "title": "Bot Chat 2"}]})), None);
        assert_eq!(
            find_in_listing(&json!({"sessions": [{"id": "seg", "title": "", "root_title": "Bot Chat"}]})).as_deref(),
            Some("seg")
        );
    }

    #[test]
    fn reads_the_roster_canonical_session() {
        assert_eq!(
            canonical_from_roster(&json!({"canonical_session": {"id": "a", "resolved_id": "b"}})).as_deref(),
            Some("b")
        );
        assert_eq!(canonical_from_roster(&json!({"canonical_session": null})), None);
    }
}
