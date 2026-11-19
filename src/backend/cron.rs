//! A bot's scheduled jobs (Hermes cron) from `cron.manage {action: "list"}`.
//!
//! Servers that honor `profile` answer with `scoped` set and only that bot's
//! jobs. Older ones return one shared store; there, jobs belong to a bot by
//! their `[bot:<name>]` name prefix and untagged jobs to `default`, the same
//! rule the official desktop app uses.

use serde_json::{Value, json};

/// Splits `[bot:<name>] title` into (`Some(name)`, `title`).
pub fn split_bot_tag(name: &str) -> (Option<String>, String) {
    let trimmed = name.trim_start();
    if let Some(rest) = trimmed.strip_prefix("[bot:").or_else(|| trimmed.strip_prefix("[BOT:")) {
        if let Some(end) = rest.find(']') {
            let tag = &rest[..end];
            let valid = !tag.is_empty()
                && tag.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
                && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if valid {
                return (Some(tag.to_ascii_lowercase()), rest[end + 1..].trim().to_string());
            }
        }
    }
    (None, name.trim().to_string())
}

/// The jobs to show for `bot`, reduced to what the panel draws.
pub fn jobs_for_bot(result: &Value, bot: &str) -> Vec<Value> {
    let scoped = result
        .get("scoped")
        .and_then(Value::as_str)
        .is_some_and(|s| s.trim().eq_ignore_ascii_case(bot.trim()));
    let text = |job: &Value, key: &str| job.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    result
        .get("jobs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|job| {
            let (tag, title) = split_bot_tag(&text(job, "name"));
            if !scoped && tag.as_deref().unwrap_or("default") != bot.to_ascii_lowercase() {
                return None;
            }
            let paused = job.get("enabled").and_then(Value::as_bool) == Some(false) || text(job, "state") == "paused";
            let error = ["last_error", "last_fire_error", "last_delivery_error"]
                .iter()
                .map(|key| text(job, key))
                .find(|e| !e.is_empty())
                .unwrap_or_default();
            Some(json!({
                "id": text(job, "job_id"),
                "title": if title.is_empty() { text(job, "prompt_preview") } else { title },
                "schedule": text(job, "schedule"),
                "prompt": text(job, "prompt_preview"),
                "nextRunAt": text(job, "next_run_at"),
                "lastRunAt": text(job, "last_run_at"),
                "lastStatus": text(job, "last_status"),
                "deliver": text(job, "deliver"),
                "error": error,
                "paused": paused,
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_bot_tags() {
        assert_eq!(split_bot_tag("[bot:work] Daily brief"), (Some("work".into()), "Daily brief".into()));
        assert_eq!(split_bot_tag("Plain job"), (None, "Plain job".into()));
        assert_eq!(split_bot_tag("[bot:] x"), (None, "[bot:] x".into()));
    }

    #[test]
    fn scoped_replies_show_every_job() {
        let reply = json!({"scoped": "work", "jobs": [
            {"job_id": "1", "name": "Untagged", "schedule": "every 60m"},
            {"job_id": "2", "name": "[bot:other] Tagged", "schedule": "0 9 * * *", "enabled": false},
        ]});
        let jobs = jobs_for_bot(&reply, "work");
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[1]["title"], "Tagged");
        assert_eq!(jobs[1]["paused"], true);
    }

    #[test]
    fn unscoped_replies_filter_by_tag() {
        let reply = json!({"jobs": [
            {"job_id": "1", "name": "Untagged"},
            {"job_id": "2", "name": "[bot:work] Brief", "last_error": "boom"},
            {"job_id": "3", "name": "[bot:finance] Bills"},
        ]});
        let work = jobs_for_bot(&reply, "work");
        assert_eq!(work.len(), 1);
        assert_eq!(work[0]["error"], "boom");
        let default: Vec<_> = jobs_for_bot(&reply, "default").iter().map(|j| j["id"].clone()).collect();
        assert_eq!(default, vec![json!("1")]);
    }
}
