//! The open conversation as rows: what was said, tool calls, commands and
//! notes. Streaming events edit the rows in place; the list view is told
//! which rows changed so it re-measures only those.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    /// The bot's to-do list for this turn.
    Todos,
    Error,
    /// A slash command and what it printed.
    Command,
    Info,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Todo {
    pub content: String,
    /// pending, in_progress, completed or cancelled.
    pub status: String,
}

/// A file shown with a message: a local file or one on the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileRef {
    /// "image" or "file".
    pub kind: String,
    pub name: String,
    /// `file://` URL of a local copy.
    pub url: String,
    /// Path on the server.
    pub path: String,
}

impl FileRef {
    pub fn from_view(v: &Value) -> FileRef {
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        FileRef { kind: text("kind"), name: text("name"), url: text("url"), path: text("path") }
    }

    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }
}

pub fn todos_from(v: &Value) -> Vec<Todo> {
    let parsed = match v {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    parsed
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|t| Todo {
                    content: t.get("content").and_then(Value::as_str).unwrap_or_default().to_string(),
                    status: t.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn files_from(v: &Value) -> Vec<FileRef> {
    let parsed = match v {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    parsed.as_array().map(|items| items.iter().map(FileRef::from_view).collect()).unwrap_or_default()
}

#[derive(Clone, Debug)]
pub struct Row {
    /// Stable for the row's lifetime; element ids derive from it.
    pub id: u64,
    pub role: Role,
    pub text: String,
    pub reasoning: String,
    pub tool_id: String,
    pub name: String,
    pub detail: String,
    pub summary: String,
    pub done: bool,
    pub thought_ms: u64,
    pub attachments: Vec<FileRef>,
    pub args: String,
    pub result: String,
    pub diff: String,
    pub todos: Vec<Todo>,
    pub duration: f64,
    pub error: String,
    /// Seconds since the epoch; 0 when unknown.
    pub timestamp: f64,
    /// The stored row id (for edit/regenerate); negative when unknown.
    pub row_id: i64,
    // What the user opened.
    pub show_reasoning: bool,
    pub expanded: bool,
    // When things happened, for the views' transitions (see `motion::fresh`).
    /// Arrived during this session (`None` for rows loaded with the chat).
    pub born: Option<Instant>,
    /// Finished running.
    pub settled_at: Option<Instant>,
    /// The reasoning or the tool's details were last opened or closed.
    pub toggled_at: Option<Instant>,
}

impl Row {
    fn new(id: u64, role: Role) -> Row {
        Row {
            id,
            role,
            text: String::new(),
            reasoning: String::new(),
            tool_id: String::new(),
            name: String::new(),
            detail: String::new(),
            summary: String::new(),
            done: true,
            thought_ms: 0,
            attachments: Vec::new(),
            args: String::new(),
            result: String::new(),
            diff: String::new(),
            todos: Vec::new(),
            duration: 0.0,
            error: String::new(),
            timestamp: 0.0,
            row_id: -1,
            show_reasoning: false,
            expanded: false,
            born: None,
            settled_at: None,
            toggled_at: None,
        }
    }

    /// A row from the backend's view shape (`history::row` keys).
    fn from_value(id: u64, v: &Value) -> Option<Row> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let role = match text("role").as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            "todos" => Role::Todos,
            "error" => Role::Error,
            "command" => Role::Command,
            "info" => Role::Info,
            _ => return None,
        };
        let mut row = Row::new(id, role);
        row.text = text("text");
        row.reasoning = text("reasoning");
        row.tool_id = text("toolId");
        row.name = text("name");
        row.detail = text("detail");
        row.summary = text("summary");
        row.args = text("args");
        row.result = text("result");
        row.diff = text("diff");
        row.error = text("error");
        row.attachments = v.get("attachments").map(files_from).unwrap_or_default();
        row.todos = v.get("todos").map(todos_from).unwrap_or_default();
        row.duration = v.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
        row.timestamp = v.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
        row.row_id = v.get("rowId").and_then(Value::as_i64).unwrap_or(-1);
        Some(row)
    }
}

/// A finished tool call's fields.
pub struct ToolDone {
    pub tool_id: String,
    pub name: String,
    pub summary: String,
    pub result: String,
    pub diff: String,
    pub todos: Vec<Todo>,
    pub duration: f64,
    pub error: String,
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl AppState {
    fn push_row(&mut self, row: Row) -> usize {
        let ix = self.rows.len();
        if row.role == Role::User {
            self.last_user_index = Some(ix);
        }
        self.rows.push(row);
        self.list.splice(ix..ix, 1);
        ix
    }

    fn new_row(&mut self, role: Role) -> Row {
        self.next_row += 1;
        let mut row = Row::new(self.next_row, role);
        row.born = Some(Instant::now());
        row
    }

    /// Edits row `ix` and has the list measure it again.
    fn edit_row(&mut self, ix: usize, f: impl FnOnce(&mut Row)) {
        if let Some(row) = self.rows.get_mut(ix) {
            let was_done = row.done;
            f(row);
            if row.done && !was_done {
                row.settled_at = Some(Instant::now());
            }
            self.list.splice(ix..ix + 1, 1);
        }
    }

    pub fn toggle_reasoning(&mut self, ix: usize) {
        self.edit_row(ix, |r| {
            r.show_reasoning = !r.show_reasoning;
            r.toggled_at = Some(Instant::now());
        });
    }

    pub fn toggle_expanded(&mut self, ix: usize) {
        self.edit_row(ix, |r| {
            r.expanded = !r.expanded;
            r.toggled_at = Some(Instant::now());
        });
    }

    /// Replaces the conversation, following its end.
    pub(crate) fn load_transcript(&mut self, rows: Vec<Value>) {
        self.rows.clear();
        self.assistant_row = None;
        self.todo_row = None;
        self.last_user_index = None;
        self.thinking_since = None;
        for v in rows {
            self.next_row += 1;
            if let Some(mut row) = Row::from_value(self.next_row, &v) {
                row.done = true;
                if row.role == Role::User {
                    self.last_user_index = Some(self.rows.len());
                }
                self.rows.push(row);
            }
        }
        self.list.reset(self.rows.len());
        self.scroll_to_end();
    }

    /// Goes down to the latest message and follows it (sending, opening a chat).
    pub fn scroll_to_end(&self) {
        self.list.set_follow_mode(gpui_kit::FollowMode::Tail);
    }

    /// Drops row `from` and everything after it.
    pub(super) fn truncate_rows(&mut self, from: usize) {
        let len = self.rows.len();
        if from >= len {
            return;
        }
        self.rows.truncate(from);
        self.list.splice(from..len, 0);
        self.last_user_index = self.rows.iter().rposition(|r| r.role == Role::User);
        self.assistant_row = None;
        self.todo_row = None;
    }

    pub(super) fn on_user_message(&mut self, text: &str, files: Vec<FileRef>) {
        let mut row = self.new_row(Role::User);
        row.text = text.to_string();
        row.attachments = files;
        row.timestamp = now();
        self.push_row(row);
        self.assistant_row = None;
        self.todo_row = None;
        self.scroll_to_end();
    }

    pub(super) fn on_user_row_id(&mut self, row_id: i64) {
        if let Some(ix) = self.last_user_index {
            self.edit_row(ix, |r| r.row_id = row_id);
        }
    }

    pub(super) fn on_command_started(&mut self, command: &str) {
        let mut row = self.new_row(Role::Command);
        row.name = command.to_string();
        row.done = false;
        self.push_row(row);
        self.scroll_to_end();
    }

    /// Fills the command's row; a command that printed nothing leaves none.
    pub(super) fn on_command_output(&mut self, command: &str, text: &str) {
        let open = self.rows.iter().rposition(|r| r.role == Role::Command && !r.done && r.name == command);
        match open {
            Some(i) if !text.is_empty() => self.edit_row(i, |r| {
                r.text = text.to_string();
                r.done = true;
            }),
            Some(i) => {
                self.rows.remove(i);
                self.list.splice(i..i + 1, 0);
                let shift = |slot: &mut Option<usize>| {
                    if let Some(k) = slot.as_mut() {
                        if *k > i {
                            *k -= 1;
                        }
                    }
                };
                shift(&mut self.assistant_row);
                shift(&mut self.todo_row);
                shift(&mut self.last_user_index);
            }
            None if !text.is_empty() => {
                let mut row = self.new_row(Role::Command);
                row.name = command.to_string();
                row.text = text.to_string();
                self.push_row(row);
            }
            None => {}
        }
    }

    pub(super) fn on_steered(&mut self, text: &str, status: &str) {
        let mut row = self.new_row(Role::Info);
        row.text = if status == "rejected" { format!("Couldn't steer: {text}") } else { format!("↪ {text}") };
        self.push_row(row);
        self.scroll_to_end();
    }

    fn ensure_assistant_row(&mut self) -> usize {
        match self.assistant_row {
            Some(ix) => ix,
            None => {
                let mut row = self.new_row(Role::Assistant);
                row.done = false;
                let ix = self.push_row(row);
                self.assistant_row = Some(ix);
                ix
            }
        }
    }

    fn close_thinking(&mut self) {
        if let (Some(ix), Some(since)) = (self.assistant_row, self.thinking_since.take()) {
            let ms = since.elapsed().as_millis() as u64;
            self.edit_row(ix, |r| r.thought_ms = ms);
        }
    }

    pub(super) fn on_assistant_started(&mut self) {
        self.ensure_assistant_row();
    }

    pub(super) fn on_assistant_delta(&mut self, text: &str) {
        let ix = self.ensure_assistant_row();
        self.close_thinking();
        self.edit_row(ix, |r| r.text.push_str(text));
    }

    pub(super) fn on_reasoning_delta(&mut self, text: &str) {
        let ix = self.ensure_assistant_row();
        if self.thinking_since.is_none() && self.rows[ix].text.is_empty() {
            self.thinking_since = Some(Instant::now());
        }
        self.edit_row(ix, |r| r.reasoning.push_str(text));
    }

    pub(super) fn on_tool_started(&mut self, tool_id: String, name: String, detail: String, args: String) {
        self.close_thinking();
        if let Some(ix) = self.assistant_row {
            self.edit_row(ix, |r| r.done = true);
        }
        let mut row = self.new_row(Role::Tool);
        row.tool_id = tool_id;
        row.name = name;
        row.detail = detail;
        row.args = args;
        row.done = false;
        self.push_row(row);
        // Text after a tool call starts a new answer below it.
        self.assistant_row = None;
    }

    pub(super) fn on_tool_finished(&mut self, tool: ToolDone) {
        let found = self.rows.iter().rposition(|r| {
            r.role == Role::Tool && !r.done && (r.tool_id == tool.tool_id || (tool.tool_id.is_empty() && r.name == tool.name))
        });
        if let Some(ix) = found {
            self.edit_row(ix, |r| {
                r.result = tool.result;
                r.diff = tool.diff;
                r.todos = tool.todos;
                r.duration = tool.duration;
                r.error = tool.error;
                r.summary = tool.summary;
                r.done = true;
            });
        }
    }

    pub(super) fn on_assistant_interim(&mut self, text: &str, already_streamed: bool) {
        if !already_streamed && !text.is_empty() {
            let ix = self.ensure_assistant_row();
            self.edit_row(ix, |r| r.text.push_str(text));
        }
        // The commentary is its own section; what follows starts a new one.
        if let Some(ix) = self.assistant_row.take() {
            self.edit_row(ix, |r| r.done = true);
        }
    }

    pub(super) fn on_todos_updated(&mut self, todos: Vec<Todo>) {
        match self.todo_row.filter(|&ix| self.rows.get(ix).is_some_and(|r| r.role == Role::Todos)) {
            Some(ix) => self.edit_row(ix, |r| r.todos = todos),
            None => {
                let mut row = self.new_row(Role::Todos);
                row.todos = todos;
                self.todo_row = Some(self.push_row(row));
            }
        }
    }

    pub(super) fn on_turn_finished(&mut self, status: &str, text: &str, error: &str) {
        self.close_thinking();
        if !text.is_empty() {
            let ix = self.ensure_assistant_row();
            self.edit_row(ix, |r| r.text = text.to_string());
        }
        if let Some(ix) = self.assistant_row {
            self.edit_row(ix, |r| r.done = true);
        }
        for ix in 0..self.rows.len() {
            if self.rows[ix].role == Role::Tool && !self.rows[ix].done {
                self.edit_row(ix, |r| r.done = true);
            }
        }
        match status {
            "error" => {
                let mut row = self.new_row(Role::Error);
                row.text = if error.is_empty() { "The bot reported an error.".into() } else { error.to_string() };
                self.push_row(row);
            }
            "interrupted" => {
                let mut row = self.new_row(Role::Info);
                row.text = "Stopped".into();
                self.push_row(row);
            }
            _ => {}
        }
        self.assistant_row = None;
    }

    /// Waiting for the first word of an answer.
    pub fn waiting_for_answer(&self) -> bool {
        self.streaming && self.assistant_row.is_none() && self.rows.last().is_some_and(|r| r.role == Role::User)
    }
}
