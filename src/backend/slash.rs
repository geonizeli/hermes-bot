//! Slash commands. The server owns the command set: `commands.catalog`
//! answers, per session, every command, alias, subcommand and skill it can
//! run, and whether a desktop client may use each one (`desktop` is null
//! when it may). Commands run on the server with `slash.exec`, falling back
//! to `command.dispatch`, and answer a directive (output, alias, a message
//! to send, a draft for the composer).
//!
//! A few commands are handled here instead: ones whose server side is a
//! terminal action (`/new` resets the TUI, `/compress` outlives the slash
//! worker's timeout) and the official desktop's own client actions, which
//! hermes-bot either maps onto its UI or doesn't have.
//! See hermes-agent `apps/desktop/src/AGENTS.md`, "Slash commands", and
//! `apps/shared/src/slash.ts`.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::gateway::Gateway;

/// A typed `/name arg…` line.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// Lower-cased, without the slash.
    pub name: String,
    /// The rest, outer whitespace trimmed, inner whitespace (newlines too) kept.
    pub arg: String,
}

/// A command is `/` at the very start, a bare name, then whitespace or the
/// end. `/usr/local` (a second slash) and `/ words` (no name) are prose.
pub fn parse(text: &str) -> Option<Parsed> {
    let rest = text.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..end];
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(Parsed { name: name.to_lowercase(), arg: rest[end..].trim().to_string() })
}

/// Commands this app runs itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Local {
    Stop,
    /// Steers a running turn; when idle the server turns it into a message.
    Steer,
    /// Opens the model picker.
    Model,
    /// `/reasoning <level>` sets the effort; anything else goes to the server.
    Reasoning,
    /// `session.compress`. In a Bot Chat, `/new`, `/reset` and `/clear` land
    /// here too: the chat is the bot's only one, so starting over means
    /// compacting it (the official Bot Mode does the same).
    Compress,
    Help,
    /// `/skin [theme]` shows or switches hermes-bot's theme.
    Skin,
    /// `/queue <message>` sends after the current answer; `/queue` lists.
    Queue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Local(Local),
    Server,
    /// Not runnable here; the text says why.
    Unavailable(String),
}

/// The official desktop's client-side commands (`DESKTOP_COMMAND_SPECS`) and
/// the TUI's display toggles that hermes-bot doesn't have, by canonical name.
const NOT_HERE: &[(&str, &str)] = &[
    ("/title", "The Bot Chat's title is how the bot finds its chat, so it can't be renamed."),
    ("/undo", "Use Edit on your last message instead."),
    ("/retry", "Use Regenerate under the bot's last answer instead."),
    ("/branch", "A bot has one chat, so it can't be branched."),
    ("/resume", "A bot has one chat; pick another bot in the sidebar."),
    ("/sessions", "A bot has one chat; pick another bot in the sidebar."),
    ("/profile", "Pick another bot in the sidebar."),
    ("/prompt", "Write the prompt in the message box instead."),
    ("/btw", ""),
    ("/bg", ""),
    ("/pet", ""),
    ("/hatch", ""),
    ("/palette", ""),
    ("/wake", ""),
    ("/handoff", ""),
    ("/save", ""),
    ("/export", ""),
    ("/import", ""),
    ("/timestamps", ""),
    ("/journey", ""),
    ("/yolo", ""),
    ("/density", ""),
    ("/details", ""),
    ("/logs", ""),
    ("/mouse", ""),
    ("/pets", ""),
];

fn local(canonical: &str) -> Option<Local> {
    Some(match canonical {
        "/stop" => Local::Stop,
        "/steer" => Local::Steer,
        "/model" => Local::Model,
        "/reasoning" => Local::Reasoning,
        "/compress" | "/new" | "/clear" => Local::Compress,
        "/help" | "/commands" => Local::Help,
        "/skin" => Local::Skin,
        "/queue" => Local::Queue,
        _ => return None,
    })
}

/// What a command does here, when that differs from the server's wording.
fn local_description(canonical: &str) -> Option<&'static str> {
    Some(match canonical {
        "/new" | "/clear" => "Start over: compress this Bot Chat (it's the bot's only chat)",
        "/stop" => "Stop the bot's answer",
        "/model" => "Pick the model for this chat",
        "/help" => "Show the commands you can use here",
        "/queue" => "Send a message after the bot's current answer (or list the queue)",
        "/skin" => "Show or switch the app's theme (system, dark, light, omarchy)",
        _ => return None,
    })
}

/// Reasoning levels `/reasoning` sets directly.
pub const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// One row of the command menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// With the slash: `/goal`.
    pub name: String,
    pub description: String,
    pub category: String,
    pub skill: bool,
}

/// A session's command set, from `commands.catalog`.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// Alias → canonical name, both with the slash.
    canon: HashMap<String, String>,
    /// Canonical name → why a desktop client can't use it (terminal, advanced…).
    desktop: HashMap<String, String>,
    /// Canonical name → its subcommands.
    sub: HashMap<String, Vec<String>>,
}

fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|p| {
                    let name = p.get(0)?.as_str()?.to_string();
                    let description = p.get(1).and_then(Value::as_str).unwrap_or_default().to_string();
                    Some((name, description))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Catalog {
    pub fn from_json(result: &Value) -> Catalog {
        let skills: HashSet<String> = result
            .get("skills")
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        for category in result.get("categories").and_then(Value::as_array).cloned().unwrap_or_default() {
            let label = category.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            for (name, description) in pairs(category.get("pairs")) {
                if seen.insert(name.clone()) {
                    let skill = skills.contains(&name);
                    entries.push(Entry { name, description, category: label.clone(), skill });
                }
            }
        }
        // Skills and user quick commands ride only the flat list.
        for (name, description) in pairs(result.get("pairs")) {
            if seen.insert(name.clone()) {
                let skill = skills.contains(&name);
                let category = if skill { "Skills" } else { "Other" }.to_string();
                entries.push(Entry { name, description, category, skill });
            }
        }
        let strings = |key: &str| -> HashMap<String, String> {
            result
                .get(key)
                .and_then(Value::as_object)
                .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default()
        };
        let desktop = result
            .get("commands")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, meta)| {
                        let reason = meta.get("desktop")?.as_str()?;
                        Some((k.clone(), reason.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sub = result
            .get("sub")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        let options = v
                            .as_array()
                            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                            .unwrap_or_default();
                        (k.clone(), options)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let canon = strings("canon");
        for entry in &mut entries {
            let canonical = canon.get(&entry.name).unwrap_or(&entry.name);
            if let Some(text) = local_description(canonical) {
                entry.description = text.to_string();
            }
        }
        Catalog { entries, canon, desktop, sub }
    }

    /// The canonical name for `name` (no slash), with the slash.
    pub fn canonical(&self, name: &str) -> String {
        let key = format!("/{name}");
        self.canon.get(&key).cloned().unwrap_or(key)
    }

    pub fn route(&self, name: &str) -> Route {
        let typed = format!("/{name}");
        let canonical = self.canonical(name);
        if let Some(l) = local(&canonical).or_else(|| local(&typed)) {
            return Route::Local(l);
        }
        if let Some((_, hint)) = NOT_HERE.iter().find(|(n, _)| *n == canonical || *n == typed) {
            let base = format!("{typed} isn't available in hermes-bot.");
            return Route::Unavailable(if hint.is_empty() { base } else { format!("{base} {hint}") });
        }
        if let Some(reason) = self.desktop.get(&canonical) {
            let place = match reason.as_str() {
                "terminal" => "the Hermes terminal app",
                "advanced" | "settings" => "Hermes' own settings",
                "composer-voice" => "voice mode",
                _ => "",
            };
            return Route::Unavailable(if place.is_empty() {
                format!("{typed} isn't available in hermes-bot.")
            } else {
                format!("{typed} only works in {place}.")
            });
        }
        // Skills, quick commands and anything else: the server decides.
        Route::Server
    }

    fn shown(&self, entry: &Entry) -> bool {
        !matches!(self.route(entry.name.trim_start_matches('/')), Route::Unavailable(_))
    }

    /// Menu rows for the composer's text, best match first. While the name
    /// is being typed it lists commands; after `/name ` it lists that
    /// command's subcommands.
    pub fn suggestions(&self, text: &str, limit: usize) -> Vec<Value> {
        let Some(rest) = text.strip_prefix('/') else { return Vec::new() };
        match rest.split_once(char::is_whitespace) {
            None => self.name_suggestions(&rest.to_lowercase(), limit),
            Some((name, partial)) => {
                // Only the first word of the argument is offered.
                if partial.contains(char::is_whitespace) {
                    return Vec::new();
                }
                let name = name.to_lowercase();
                if matches!(self.route(&name), Route::Unavailable(_)) {
                    return Vec::new();
                }
                let canonical = self.canonical(&name);
                let options: Vec<String> = match self.sub.get(&canonical) {
                    Some(options) => options.clone(),
                    None if canonical == "/reasoning" => EFFORTS.iter().map(|s| s.to_string()).collect(),
                    None => Vec::new(),
                };
                let partial = partial.to_lowercase();
                options
                    .into_iter()
                    .filter(|o| o.to_lowercase().starts_with(&partial) && o.to_lowercase() != partial)
                    .take(limit)
                    .map(|o| {
                        json!({
                            "insert": format!("/{name} {o}"),
                            "display": o,
                            "meta": "",
                            "skill": false,
                        })
                    })
                    .collect()
            }
        }
    }

    fn name_suggestions(&self, prefix: &str, limit: usize) -> Vec<Value> {
        let name_of = |e: &Entry| e.name.trim_start_matches('/').to_lowercase();
        let mut ranked: Vec<(u8, usize, &Entry)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.shown(e))
            .filter_map(|(i, e)| {
                let name = name_of(e);
                let rank = if name.starts_with(prefix) {
                    0
                } else if prefix.len() >= 2 && name.contains(prefix) {
                    1
                } else if prefix.len() >= 3 && e.description.to_lowercase().contains(prefix) {
                    2
                } else {
                    return None;
                };
                Some((rank, i, e))
            })
            .collect();
        ranked.sort_by_key(|(rank, i, _)| (*rank, *i));
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, _, e)| {
                json!({
                    "insert": format!("{} ", e.name),
                    "display": e.name,
                    "meta": e.description,
                    "skill": e.skill,
                })
            })
            .collect()
    }

    /// `/help`: what can be typed here, by category.
    pub fn help(&self) -> String {
        if self.entries.is_empty() {
            return "The command list hasn't loaded yet. Try again in a moment.".into();
        }
        let mut out = String::new();
        let mut category = "";
        let mut skills = 0;
        for entry in self.entries.iter().filter(|e| self.shown(e)) {
            if entry.skill {
                skills += 1;
                continue;
            }
            if entry.category != category {
                category = &entry.category;
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("{category}\n"));
            }
            out.push_str(&format!("  {:<16} {}\n", entry.name, entry.description));
        }
        if skills > 0 {
            out.push_str(&format!("\n{skills} skills: type / and a name to find one.\n"));
        }
        out
    }
}

/// What the server wants done with a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    /// Text to show.
    Output(String),
    /// Run another command instead (with the same argument).
    Alias(String),
    /// Send `message` as a prompt; the bubble shows `display`, never a skill's
    /// expanded body. `notice` is a line to show first.
    Send { message: String, display: String, notice: String },
    /// Put `message` in the composer.
    Prefill { message: String, notice: String },
}

fn field(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A `command.dispatch` (or rerouted `slash.exec`) answer; `None` when it's
/// malformed. Mirrors `parseCommandDispatch` in apps/shared/src/slash.ts.
pub fn directive(row: &Value) -> Option<Directive> {
    let display_for = |message: &str| {
        Some(field(row, "display"))
            .filter(|d| !d.trim().is_empty())
            .or_else(|| skill_invocation_text(message))
            .unwrap_or_else(|| message.to_string())
    };
    match row.get("type").and_then(Value::as_str)? {
        "exec" | "plugin" => Some(Directive::Output(field(row, "output"))),
        "alias" => Some(Directive::Alias(row.get("target")?.as_str()?.to_string())),
        "prefill" => Some(Directive::Prefill { message: row.get("message")?.as_str()?.to_string(), notice: field(row, "notice") }),
        "send" => {
            let message = row.get("message")?.as_str()?.to_string();
            Some(Directive::Send { display: display_for(&message), message, notice: field(row, "notice") })
        }
        "skill" => {
            row.get("name")?.as_str()?;
            let message = field(row, "message");
            Some(Directive::Send { display: display_for(&message), message, notice: String::new() })
        }
        _ => None,
    }
}

/// A plain `slash.exec` answer: the worker's text, with any warning first.
fn exec_output(row: &Value, name: &str) -> Directive {
    if row.get("type").is_some()
        && let Some(d) = directive(row)
    {
        return d;
    }
    let output = Some(field(row, "output")).filter(|o| !o.trim().is_empty()).unwrap_or_else(|| format!("/{name}: no output"));
    let warning = field(row, "warning");
    Directive::Output(if warning.is_empty() { output } else { format!("warning: {warning}\n{output}") })
}

/// Runs `/name arg` on the server: `slash.exec`, then `command.dispatch`
/// when the worker can't. Aliases are followed here.
pub async fn run(gateway: &Gateway, live: &str, profile: &str, name: &str, arg: &str) -> Result<Directive, String> {
    let mut name = name.to_string();
    for _ in 0..4 {
        let command = if arg.is_empty() { name.clone() } else { format!("{name} {arg}") };
        let exec = gateway
            .request("slash.exec", json!({ "session_id": live, "command": command, "profile": profile }))
            .await;
        let directive = match exec {
            Ok(row) => exec_output(&row, &name),
            Err(exec_error) => {
                let params = json!({
                    "session_id": live,
                    "name": name,
                    "arg": if arg.is_empty() { Value::Null } else { json!(arg) },
                    "profile": profile,
                });
                match gateway.request("command.dispatch", params).await {
                    Ok(row) => directive(&row).ok_or_else(|| "The server's answer to the command was malformed.".to_string())?,
                    // "not a quick/plugin/skill command" only says the fallback
                    // had nothing; the worker's failure is the real error.
                    Err(e) if e.to_lowercase().contains("not a quick/plugin") => {
                        return Err(format!("/{name} failed: {exec_error}"));
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        match directive {
            Directive::Alias(target) => name = target.trim_start_matches('/').to_lowercase(),
            other => return Ok(other),
        }
    }
    Err(format!("/{name} points to itself."))
}

/// `session.compress`'s answer as one line.
pub fn compress_summary(reply: &Value) -> String {
    let n = |k: &str| reply.get(k).and_then(Value::as_i64);
    match (n("before_messages"), n("after_messages")) {
        (Some(before), Some(after)) => {
            let mut text = format!("Compressed the chat: {before} → {after} messages");
            if let (Some(bt), Some(at)) = (n("before_tokens"), n("after_tokens")) {
                text.push_str(&format!(", ~{bt} → ~{at} tokens"));
            }
            text.push('.');
            text
        }
        _ => match reply.get("status").and_then(Value::as_str) {
            Some(status) if !status.is_empty() => format!("Compression: {status}."),
            _ => "Compressed the chat.".into(),
        },
    }
}

// A `/skill` expands into a model-facing message embedding the whole skill;
// the bubble shows the invocation instead. The markers mirror
// `agent/skill_commands.py` (via apps/shared/src/skill-scaffold.ts).
const INVOCATION_PREFIX: &str = "[IMPORTANT: The user has invoked the ";
const SINGLE_MARKER: &str = "The full skill content is loaded below.]";
const SINGLE_INSTRUCTION: &str = "The user has provided the following instruction alongside the skill invocation: ";
const RUNTIME_NOTE: &str = "\n\n[Runtime note:";
const BUNDLE_MARKER: &str = " skill bundle,";
const BUNDLE_INSTRUCTION: &str = "\nUser instruction: ";
const BUNDLE_SKILL_BLOCK: &str = "\n\n[Loaded as part of the ";

/// `/work fix the leak` for a scaffolded skill message; `None` for prose.
pub fn skill_invocation_text(text: &str) -> Option<String> {
    let rest = text.strip_prefix(INVOCATION_PREFIX)?.strip_prefix('"')?;
    let name = rest[..rest.find('"')?].trim();
    if name.is_empty() {
        return None;
    }
    let label = if name.starts_with('/') { name.to_string() } else { format!("/{name}") };
    let between = |marker: &str, end: &str, from_end: bool| -> String {
        let index = if from_end { text.rfind(marker) } else { text.find(marker) };
        let Some(index) = index else { return String::new() };
        let tail = &text[index + marker.len()..];
        tail[..tail.find(end).unwrap_or(tail.len())].trim().to_string()
    };
    let instruction = if text.contains(BUNDLE_MARKER) {
        between(BUNDLE_INSTRUCTION, BUNDLE_SKILL_BLOCK, false)
    } else if text.contains(SINGLE_MARKER) {
        between(SINGLE_INSTRUCTION, RUNTIME_NOTE, true)
    } else {
        String::new()
    };
    Some(if instruction.is_empty() {
        label
    } else {
        format!("{label} {}", instruction.split_whitespace().collect::<Vec<_>>().join(" "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_json(&json!({
            "canon": {"/compact": "/compress", "/reset": "/new", "/s": "/steer", "/ctx": "/context", "/fork": "/branch"},
            "categories": [
                {"name": "Session", "pairs": [["/new", "Start a new session"], ["/clear", "Clear screen"], ["/goal", "Set a standing goal"], ["/branch", "Branch"]]},
                {"name": "Info", "pairs": [["/context", "Context window view"], ["/usage", "Token usage"], ["/help", "Help"]]},
                {"name": "Configuration", "pairs": [["/model", "Switch model"], ["/reasoning", "Effort"], ["/fast", "Fast mode"]]},
            ],
            "pairs": [["/goal", "Set a standing goal"], ["/github", "GitHub via gh CLI."], ["/arxiv", "Search arXiv papers."]],
            "commands": {
                "/clear": {"desktop": "terminal"}, "/fast": {"desktop": "advanced"},
                "/model": {"desktop": "hidden"}, "/goal": {"desktop": null}, "/usage": {},
            },
            "skills": {"/github": {"usage": 3}, "/arxiv": {"usage": 0}},
            "sub": {"/goal": ["status", "pause", "resume", "clear"]},
        }))
    }

    #[test]
    fn parses_commands_and_leaves_prose() {
        assert_eq!(parse("/Goal  ship it \n now "), Some(Parsed { name: "goal".into(), arg: "ship it \n now".into() }));
        assert_eq!(parse("/usage"), Some(Parsed { name: "usage".into(), arg: "".into() }));
        assert_eq!(parse("/goal\nline two"), Some(Parsed { name: "goal".into(), arg: "line two".into() }));
        assert_eq!(parse("/usr/local is a path"), None);
        assert_eq!(parse("/ hello"), None);
        assert_eq!(parse("/"), None);
        assert_eq!(parse("run /usage"), None);
    }

    #[test]
    fn routes_by_the_server_catalog() {
        let c = catalog();
        assert_eq!(c.route("usage"), Route::Server);
        assert_eq!(c.route("github"), Route::Server);
        assert_eq!(c.route("some-quick-command"), Route::Server);
        assert_eq!(c.route("ctx"), Route::Server);
        assert!(matches!(c.route("fast"), Route::Unavailable(t) if t.contains("settings")));
        assert_eq!(c.route("model"), Route::Local(Local::Model));
        assert_eq!(c.route("s"), Route::Local(Local::Steer));
        // In a Bot Chat, starting over compacts the one chat.
        for name in ["new", "reset", "clear", "compact", "compress"] {
            assert_eq!(c.route(name), Route::Local(Local::Compress), "{name}");
        }
        assert!(matches!(c.route("fork"), Route::Unavailable(t) if t.starts_with("/fork ")));
        assert!(matches!(c.route("title"), Route::Unavailable(_)));
    }

    #[test]
    fn suggests_names_then_subcommands() {
        let c = catalog();
        let names = |text: &str| -> Vec<String> {
            c.suggestions(text, 20).iter().map(|s| s["display"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(names("/g"), vec!["/goal", "/github"]);
        // Name matches before description matches; hidden ones never show.
        assert_eq!(names("/arx"), vec!["/arxiv"]);
        assert!(!names("/").contains(&"/fast".to_string()));
        assert!(!names("/").contains(&"/branch".to_string()));
        // /clear is terminal-only on the server but handled here.
        assert!(names("/cl").contains(&"/clear".to_string()));
        assert_eq!(c.suggestions("/gi", 5)[0]["skill"], true);
        assert_eq!(c.suggestions("/goal", 5)[0]["insert"], "/goal ");
        assert_eq!(names("/goal p"), vec!["pause"]);
        assert_eq!(c.suggestions("/goal p", 5)[0]["insert"], "/goal pause");
        assert_eq!(names("/reasoning hi"), vec!["high"]);
        assert!(names("/goal pause now").is_empty());
        assert!(names("/fast o").is_empty());
        assert!(names("hello").is_empty());
    }

    #[test]
    fn help_lists_usable_commands() {
        let help = catalog().help();
        assert!(help.contains("/usage"));
        assert!(help.contains("2 skills"));
        assert!(help.contains("compress this Bot Chat"));
        assert!(!help.contains("/fast"));
        assert!(!help.contains("/branch"));
    }

    #[test]
    fn reads_directives() {
        assert_eq!(directive(&json!({"type": "exec", "output": "ok"})), Some(Directive::Output("ok".into())));
        assert_eq!(directive(&json!({"type": "alias", "target": "/usage"})), Some(Directive::Alias("/usage".into())));
        assert_eq!(
            directive(&json!({"type": "send", "message": "hi", "notice": "No agent running; sent as next turn: hi"})),
            Some(Directive::Send { message: "hi".into(), display: "hi".into(), notice: "No agent running; sent as next turn: hi".into() })
        );
        assert_eq!(
            directive(&json!({"type": "skill", "name": "work", "message": "BODY", "display": "/work fix it"})),
            Some(Directive::Send { message: "BODY".into(), display: "/work fix it".into(), notice: "".into() })
        );
        assert_eq!(
            directive(&json!({"type": "prefill", "message": "draft"})),
            Some(Directive::Prefill { message: "draft".into(), notice: "".into() })
        );
        assert_eq!(directive(&json!({"type": "send"})), None);
        assert_eq!(directive(&json!({"type": "wat"})), None);
        assert_eq!(exec_output(&json!({"output": "Tokens: 5", "warning": "old"}), "usage"), Directive::Output("warning: old\nTokens: 5".into()));
        assert_eq!(exec_output(&json!({}), "usage"), Directive::Output("/usage: no output".into()));
        assert_eq!(exec_output(&json!({"output": "x", "type": "exec"}), "goal"), Directive::Output("x".into()));
    }

    #[test]
    fn shows_skill_invocations_not_their_bodies() {
        let single = format!(
            "{INVOCATION_PREFIX}\"work\" skill. {SINGLE_MARKER}\n\nBODY\n\n{SINGLE_INSTRUCTION}fix   the\nleak{RUNTIME_NOTE} x]"
        );
        assert_eq!(skill_invocation_text(&single).as_deref(), Some("/work fix the leak"));
        let bare = format!("{INVOCATION_PREFIX}\"arxiv\" skill. {SINGLE_MARKER}\n\nBODY");
        assert_eq!(skill_invocation_text(&bare).as_deref(), Some("/arxiv"));
        assert_eq!(skill_invocation_text("just text"), None);
        assert_eq!(
            directive(&json!({"type": "skill", "name": "work", "message": single})),
            Some(Directive::Send { message: single.clone(), display: "/work fix the leak".into(), notice: "".into() })
        );
    }

    #[test]
    fn summarizes_compression() {
        assert_eq!(
            compress_summary(&json!({"before_messages": 240, "after_messages": 12, "before_tokens": 160000, "after_tokens": 9000})),
            "Compressed the chat: 240 → 12 messages, ~160000 → ~9000 tokens."
        );
        assert_eq!(compress_summary(&json!({"status": "skipped"})), "Compression: skipped.");
    }

    /// `HERMES_BOT_SLASH='/goal st' cargo test slash_live -- --ignored --nocapture`
    /// prints the default bot's suggestions for a text, and its /help.
    #[tokio::test]
    #[ignore]
    async fn slash_live() {
        use crate::backend::store;
        let text = std::env::var("HERMES_BOT_SLASH").unwrap_or("/".into());
        let config = store::load(&store::config_path()).unwrap();
        let server = config.server(config.active_server.as_deref().unwrap()).unwrap().clone();
        let mut tokens = store::load_tokens(&server.id).expect("signed in");
        if tokens.expires_soon() {
            tokens = crate::backend::auth::refresh(&server.url, &tokens).await.ok().expect("refresh");
            store::save_tokens(&server.id, &tokens).unwrap();
        }
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let gateway = Gateway::connect(&server.url, &tokens.access_token, tx).await.unwrap();
        let result = gateway.request("commands.catalog", json!({ "profile": "default" })).await.unwrap();
        let c = Catalog::from_json(&result);
        for s in c.suggestions(&text, 12) {
            println!("{}  {}  {}", s["display"], if s["skill"] == true { "[skill]" } else { "" }, s["meta"]);
        }
        println!("---\n{}", c.help());
    }
}
