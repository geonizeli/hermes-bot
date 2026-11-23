//! A Bot Chat's history as conversation rows, from the dashboard's REST
//! transcript (`/api/sessions/{id}/messages`), which, unlike the chat
//! socket's history, includes each tool's result.
//!
//! Every row carries the same keys:
//! role, text, reasoning, attachments (JSON), toolId, name, detail, args,
//! result, diff, todos (JSON), duration, error, timestamp.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::attachments;
use super::auth;
use super::session::Session;

/// Results longer than this are cut for display.
const MAX_RESULT_CHARS: usize = 6000;

pub fn row(role: &str) -> Value {
    json!({
        "role": role, "text": "", "reasoning": "", "attachments": "[]",
        "toolId": "", "name": "", "detail": "", "args": "", "result": "", "diff": "",
        "todos": "[]", "duration": 0.0, "error": "", "timestamp": 0.0, "rowId": -1,
    })
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_RESULT_CHARS).collect();
    format!("{cut}\n… (cut)")
}

/// Pretty JSON for arguments; the raw text when it isn't JSON.
pub fn pretty_args(args: &Value) -> String {
    let parsed = match args {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.clone())),
        other => other.clone(),
    };
    match parsed {
        Value::Null => String::new(),
        Value::String(s) => truncate(&s),
        Value::Object(ref map) if map.is_empty() => String::new(),
        other => truncate(&serde_json::to_string_pretty(&other).unwrap_or_default()),
    }
}

/// What a tool returned, for display: its `output`/`result`/`content`
/// field when it has one, else the whole thing. Also its diff and error.
pub fn tool_result(content: &Value) -> (String, String, String) {
    let parsed = match content {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.clone())),
        other => other.clone(),
    };
    let text_of = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    match &parsed {
        Value::Object(map) => {
            let diff = ["inline_diff", "diff", "patch"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            let error = map
                .get("error")
                .map(text_of)
                .filter(|e| !e.is_empty())
                .or_else(|| (map.get("status").and_then(Value::as_str) == Some("error")).then(|| "Failed".to_string()))
                .unwrap_or_default();
            let main = ["output", "result", "content", "message", "summary"]
                .iter()
                .find_map(|k| map.get(*k).filter(|v| !v.is_null()).map(text_of))
                .unwrap_or_else(|| text_of(&parsed));
            (truncate(&main), truncate(&diff), error)
        }
        other => (truncate(&text_of(other)), String::new(), String::new()),
    }
}

/// Conversation rows from REST transcript rows (oldest first).
pub fn to_view(rows: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut tool_rows: HashMap<String, usize> = HashMap::new();
    for m in rows {
        let kind = m.get("display_kind").and_then(Value::as_str).unwrap_or_default();
        let meta = m.get("display_metadata").cloned().unwrap_or(Value::Null);
        if kind == "hidden" || meta.get("model_only").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let text = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let timestamp = m.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
        let row_id = m.get("id").and_then(Value::as_i64).unwrap_or(-1);
        let content = m
            .get("display_content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| match m.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            });
        match text("role").as_str() {
            "user" => {
                // A skill turn shows as its invocation, not the skill's body.
                let content = super::slash::skill_invocation_text(&content).unwrap_or(content);
                let (body, refs) = attachments::split_refs(&content);
                if body.trim().is_empty() && refs.is_empty() {
                    continue;
                }
                let mut r = row("user");
                r["text"] = json!(body);
                r["attachments"] = json!(Value::Array(refs).to_string());
                r["timestamp"] = json!(timestamp);
                r["rowId"] = json!(row_id);
                out.push(r);
            }
            "assistant" => {
                let reasoning = text("reasoning");
                if !content.trim().is_empty() || !reasoning.trim().is_empty() {
                    let mut r = row("assistant");
                    r["text"] = json!(content);
                    r["reasoning"] = json!(reasoning);
                    r["timestamp"] = json!(timestamp);
                    r["rowId"] = json!(row_id);
                    out.push(r);
                }
                let labels = m.get("tool_call_labels").cloned().unwrap_or(Value::Null);
                for call in m.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default() {
                    let id = call.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                    let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or("tool").to_string();
                    let detail = labels
                        .get(&id)
                        .and_then(|l| l.get(0))
                        .and_then(|l| l.get("text").and_then(Value::as_str))
                        .unwrap_or_default()
                        .to_string();
                    let mut r = row("tool");
                    r["toolId"] = json!(id);
                    r["name"] = json!(name);
                    r["detail"] = json!(detail);
                    r["args"] = json!(pretty_args(call.pointer("/function/arguments").unwrap_or(&Value::Null)));
                    r["timestamp"] = json!(timestamp);
                    tool_rows.insert(id, out.len());
                    out.push(r);
                }
            }
            "tool" => {
                let id = text("tool_call_id");
                let (result, diff, error) = tool_result(m.get("content").unwrap_or(&Value::Null));
                match tool_rows.get(&id) {
                    Some(&i) => {
                        out[i]["result"] = json!(result);
                        out[i]["diff"] = json!(diff);
                        out[i]["error"] = json!(error);
                    }
                    None => {
                        let mut r = row("tool");
                        r["name"] = json!(Some(text("tool_name")).filter(|n| !n.is_empty()).unwrap_or("tool".into()));
                        r["result"] = json!(result);
                        r["diff"] = json!(diff);
                        r["error"] = json!(error);
                        out.push(r);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The latest part of a stored session's transcript, oldest first.
pub async fn load(session: &Session, stored_id: &str, profile: &str) -> Result<Vec<Value>, String> {
    let mut url = url::Url::parse(&format!("{}/api/sessions/{stored_id}/messages", session.base_url))
        .map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("profile", profile)
        .append_pair("order", "latest")
        .append_pair("limit", "300")
        .append_pair("inline_images", "false");
    let response = auth::http()
        .get(url)
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|e| format!("Couldn't load the chat history: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Couldn't load the chat history (HTTP {}).", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let mut rows = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
    // "latest" pages come oldest-first already; sort to be safe across versions.
    rows.sort_by(|a, b| {
        let key = |m: &Value| m.get("id").and_then(Value::as_i64).unwrap_or(0);
        key(a).cmp(&key(b))
    });
    Ok(to_view(&rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_tool_calls_with_their_results() {
        let rows = vec![
            json!({"id":1,"role":"user","content":"look\n\n@image:/h/images/a.png","timestamp":1.0}),
            json!({"id":2,"role":"assistant","content":"Checking.","reasoning":"hmm","tool_calls":[
                {"id":"c1","function":{"name":"execute_code","arguments":"{\"code\":\"print(1)\"}"}}
            ],"tool_call_labels":{"c1":[{"text":"Running code"}]}}),
            json!({"id":3,"role":"tool","tool_call_id":"c1","content":"{\"status\":\"success\",\"output\":\"1\\n\"}"}),
            json!({"id":4,"role":"assistant","content":"Done.","display_metadata":{"model_only":false}}),
            json!({"id":5,"role":"assistant","content":"secret","display_kind":"hidden"}),
        ];
        let view = to_view(&rows);
        let roles: Vec<_> = view.iter().map(|r| r["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "assistant"]);
        assert_eq!(view[0]["text"], "look");
        assert!(view[0]["attachments"].as_str().unwrap().contains("a.png"));
        assert_eq!(view[2]["detail"], "Running code");
        assert!(view[2]["args"].as_str().unwrap().contains("print(1)"));
        assert_eq!(view[2]["result"], "1\n");
        // Every row has every key.
        assert!(view.iter().all(|r| r.as_object().unwrap().len() == row("x").as_object().unwrap().len()));
    }

    #[test]
    fn reads_errors_and_diffs_from_results() {
        let (_, diff, error) = tool_result(&json!({"status":"error","error":"boom","inline_diff":"-a\n+b"}));
        assert_eq!((diff.as_str(), error.as_str()), ("-a\n+b", "boom"));
        let (text, _, _) = tool_result(&json!("plain"));
        assert_eq!(text, "plain");
    }
}
