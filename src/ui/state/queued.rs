//! Messages queued to send after the bot's current answer, one queue per
//! bot. Like the official desktop, the queue lives here, not on the server:
//! entries stay editable until they go out, and each is sent with
//! `prompt.submit {queued: true}` so a turn that hasn't quite ended holds it
//! instead of taking it as a correction. A stopped, failed or lost turn
//! parks the queue until the user resumes it.

use crate::backend::attachments::Attachment;
use crate::backend::slash;

#[derive(Clone, Debug)]
pub struct QueuedMessage {
    pub id: u64,
    pub text: String,
    pub attachments: Vec<Attachment>,
}

impl QueuedMessage {
    /// One line for the queue panel.
    pub fn preview(&self) -> String {
        let text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            text
        } else if self.attachments.len() == 1 {
            self.attachments[0].name.clone()
        } else {
            format!("{} attachments", self.attachments.len())
        }
    }

    /// A steer carries text into the running turn: no attachments, and not
    /// a command (those run on their own).
    pub fn can_steer(&self) -> bool {
        self.attachments.is_empty() && !self.text.trim().is_empty() && !self.is_command()
    }

    pub fn is_command(&self) -> bool {
        slash::parse(&self.text).is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageQueue {
    pub items: Vec<QueuedMessage>,
    /// Stopped: nothing goes out until the user resumes.
    pub parked: bool,
}

impl MessageQueue {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn remove(&mut self, id: u64) -> Option<QueuedMessage> {
        let ix = self.items.iter().position(|m| m.id == id)?;
        let message = self.items.remove(ix);
        if self.items.is_empty() {
            self.parked = false;
        }
        Some(message)
    }

    /// The next message to send when a turn ends, unless the queue is parked.
    pub fn next(&mut self) -> Option<QueuedMessage> {
        if self.parked || self.items.is_empty() {
            return None;
        }
        Some(self.items.remove(0))
    }

    /// Holds the queue after a stopped or failed turn; an empty queue has
    /// nothing to hold.
    pub fn park(&mut self) {
        self.parked = !self.items.is_empty();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{MessageQueue, QueuedMessage};
    use crate::backend::attachments::{Attachment, Kind};

    fn message(id: u64, text: &str) -> QueuedMessage {
        QueuedMessage { id, text: text.into(), attachments: Vec::new() }
    }

    fn file(name: &str) -> Attachment {
        Attachment { id: 1, kind: Kind::File, name: name.into(), path: PathBuf::from(name), size: 1 }
    }

    #[test]
    fn sends_in_order_unless_parked() {
        let mut queue = MessageQueue::default();
        queue.items = vec![message(1, "first"), message(2, "second")];
        queue.park();
        assert!(queue.next().is_none(), "a parked queue holds");
        queue.parked = false;
        assert_eq!(queue.next().unwrap().text, "first");
        assert_eq!(queue.next().unwrap().text, "second");
        assert!(queue.next().is_none());
    }

    #[test]
    fn an_empty_queue_never_stays_parked() {
        let mut queue = MessageQueue::default();
        queue.park();
        assert!(!queue.parked);
        queue.items = vec![message(1, "only")];
        queue.park();
        assert!(queue.parked);
        assert_eq!(queue.remove(1).unwrap().text, "only");
        assert!(!queue.parked, "removing the last message lifts the park");
        assert!(queue.remove(1).is_none());
    }

    #[test]
    fn steers_only_plain_text() {
        assert!(message(1, "look at the logs too").can_steer());
        assert!(!message(1, "/usage").can_steer());
        assert!(!message(1, "  ").can_steer());
        let with_file = QueuedMessage { id: 1, text: "see this".into(), attachments: vec![file("a.txt")] };
        assert!(!with_file.can_steer());
    }

    #[test]
    fn previews_text_or_attachments() {
        assert_eq!(message(1, "two\n  lines").preview(), "two lines");
        let one = QueuedMessage { id: 1, text: String::new(), attachments: vec![file("notes.txt")] };
        assert_eq!(one.preview(), "notes.txt");
        let two = QueuedMessage { id: 1, text: String::new(), attachments: vec![file("a"), file("b")] };
        assert_eq!(two.preview(), "2 attachments");
    }
}
