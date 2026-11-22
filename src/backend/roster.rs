//! A bot's identity as the official Hermes desktop presents it: the Bot Mode
//! metadata it keeps in the profile's `ui_meta["hermes-bots"]`, and the name
//! every surface shows.

use serde_json::{Value, json};

/// The `ui_meta` namespace the official desktop's bots plugin writes.
pub const META_NAMESPACE: &str = "hermes-bots";

/// The bot's `ui_meta["hermes-bots"]` object, or an empty one.
pub fn meta(row: &Value) -> Value {
    row.pointer(&format!("/ui_meta/{META_NAMESPACE}"))
        .filter(|m| m.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}))
}

/// The revision the server holds for our namespace, for compare-and-swap
/// writes; `None` when the server doesn't track revisions.
pub fn meta_revision(row: &Value) -> Option<u64> {
    let revisions = row.get("ui_meta_revisions")?.as_object()?;
    Some(revisions.get(META_NAMESPACE).and_then(Value::as_u64).unwrap_or(0))
}

/// The name a bot shows: its Bot Mode title, else the profile's display name,
/// else "Hermes" for the primary profile, else the profile name in title case.
pub fn display_name(row: &Value) -> String {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let title = text(meta(row).get("title"));
    if !title.is_empty() {
        return title;
    }
    let display = text(row.get("display_name"));
    if !display.is_empty() {
        return display;
    }
    let name = text(row.get("name"));
    if name.eq_ignore_ascii_case("default") {
        return "Hermes".into();
    }
    title_case(&name)
}

/// `my-work_bot` -> `My Work Bot`, as the official `displayName` does.
fn title_case(name: &str) -> String {
    let spaced = name.replace(['-', '_'], " ");
    let mut out = String::with_capacity(spaced.len());
    let mut at_word_start = true;
    for ch in spaced.trim().chars() {
        let word = ch.is_alphanumeric() || ch == '_';
        if word && at_word_start {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
        at_word_start = !word;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_official_rules() {
        assert_eq!(display_name(&json!({ "name": "default" })), "Hermes");
        assert_eq!(display_name(&json!({ "name": "work" })), "Work");
        assert_eq!(display_name(&json!({ "name": "my-home_lab" })), "My Home Lab");
        assert_eq!(display_name(&json!({ "name": "work", "display_name": "Office" })), "Office");
        assert_eq!(
            display_name(&json!({ "name": "default", "display_name": "Office", "ui_meta": { "hermes-bots": { "title": " Jarvis " } } })),
            "Jarvis"
        );
    }

    #[test]
    fn reads_meta_and_revision() {
        let row = json!({ "ui_meta": { "hermes-bots": { "shape": "pill" } }, "ui_meta_revisions": { "hermes-bots": 4 } });
        assert_eq!(meta(&row)["shape"], "pill");
        assert_eq!(meta_revision(&row), Some(4));
        assert_eq!(meta_revision(&json!({ "ui_meta_revisions": {} })), Some(0));
        assert_eq!(meta_revision(&json!({})), None);
        assert_eq!(meta(&json!({})), json!({}));
    }
}
