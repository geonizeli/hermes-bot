//! Editing a message you sent: what to ask the server for, and how to read
//! back the ids of the messages it keeps.
//!
//! An edit is a `prompt.submit` that cuts the history before the message
//! (`truncate_before_row_id` with `confirm_truncate`), like the official
//! desktop. The server asks for extra consent when the cut drops more than
//! that one turn (`confirm_deep_truncate`) or leaves nothing at all
//! (`confirm_empty_truncate`). It rewrites the rows it keeps, so it reports
//! their new ids: a map for the ids we name in `rebind_survivor_row_ids`, or,
//! from older servers, the kept user messages' ids in order.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::chat::{Role, Row};

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The edited message's row; it and everything after go.
    pub cut: usize,
    pub row_id: i64,
    /// Later messages of yours go too.
    pub deep: bool,
    /// Nothing of the conversation stays.
    pub empty: bool,
    /// Stored ids of the rows that stay, for the server to map.
    pub keep_ids: Vec<i64>,
    /// The rows of your kept messages, in order (for older servers' lists).
    pub user_ordinals: Vec<usize>,
}

pub fn plan(rows: &[Row], cut: usize) -> Plan {
    let (before, after) = rows.split_at(cut.min(rows.len()));
    Plan {
        cut,
        row_id: rows.get(cut).map_or(-1, |r| r.row_id),
        deep: after.iter().skip(1).any(|r| r.role == Role::User),
        empty: !before.iter().any(|r| matches!(r.role, Role::User | Role::Assistant | Role::Tool)),
        keep_ids: before.iter().map(|r| r.row_id).filter(|&id| id >= 0).collect(),
        user_ordinals: before.iter().enumerate().filter(|(_, r)| r.role == Role::User).map(|(i, _)| i).collect(),
    }
}

impl Plan {
    pub fn add_params(&self, params: &mut Value) {
        params["truncate_before_row_id"] = json!(self.row_id);
        params["confirm_truncate"] = json!(true);
        if self.deep {
            params["confirm_deep_truncate"] = json!(true);
        }
        if self.empty {
            params["confirm_empty_truncate"] = json!(true);
        }
        if !self.keep_ids.is_empty() {
            params["rebind_survivor_row_ids"] = json!(self.keep_ids);
        }
    }
}

/// The kept rows' new ids, from a truncating submit's reply.
#[derive(Debug, PartialEq)]
pub enum Rebind {
    /// Old id to new id; `None` when that row no longer has one.
    Map(HashMap<i64, Option<i64>>),
    /// Your kept messages' ids, in order.
    Users(Vec<Option<i64>>),
}

impl Rebind {
    pub fn from_reply(reply: &Value) -> Option<Rebind> {
        if let Some(map) = reply.get("survivor_row_id_map").and_then(Value::as_object) {
            return Some(Rebind::Map(
                map.iter().filter_map(|(old, new)| Some((old.parse().ok()?, new.as_i64()))).collect(),
            ));
        }
        let users = reply.get("survivor_user_row_ids")?.as_array()?;
        Some(Rebind::Users(users.iter().map(Value::as_i64).collect()))
    }

    /// The new id for a row that had `old`; `user_ordinal` is its place among
    /// your kept messages, if it's one. `Some(-1)` drops a stale id.
    pub fn new_id(&self, old: i64, user_ordinal: Option<usize>) -> Option<i64> {
        match self {
            Rebind::Map(map) => map.get(&old).map(|new| new.unwrap_or(-1)),
            Rebind::Users(ids) => user_ordinal.map(|i| ids.get(i).copied().flatten().unwrap_or(-1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Rebind, plan};
    use crate::ui::state::chat::{Role, Row};

    fn row(id: u64, role: Role, row_id: i64) -> Row {
        let mut r = Row::new(id, role);
        r.row_id = row_id;
        r
    }

    fn chat() -> Vec<Row> {
        vec![
            row(1, Role::User, 10),
            row(2, Role::Assistant, 11),
            row(3, Role::User, 12),
            row(4, Role::Tool, -1),
            row(5, Role::Assistant, 13),
        ]
    }

    #[test]
    fn editing_the_last_message_drops_one_turn() {
        let p = plan(&chat(), 2);
        assert_eq!(p.row_id, 12);
        assert!(!p.deep && !p.empty);
        assert_eq!(p.keep_ids, [10, 11]);
        assert_eq!(p.user_ordinals, [0]);
        let mut params = json!({});
        p.add_params(&mut params);
        assert_eq!(
            params,
            json!({ "truncate_before_row_id": 12, "confirm_truncate": true, "rebind_survivor_row_ids": [10, 11] })
        );
    }

    #[test]
    fn editing_the_first_message_cuts_everything() {
        let p = plan(&chat(), 0);
        assert!(p.deep && p.empty);
        let mut params = json!({});
        p.add_params(&mut params);
        assert_eq!(params["confirm_deep_truncate"], true);
        assert_eq!(params["confirm_empty_truncate"], true);
        assert!(params.get("rebind_survivor_row_ids").is_none());
    }

    #[test]
    fn reads_new_ids_as_a_map_or_a_list() {
        let map = Rebind::from_reply(&json!({ "survivor_row_id_map": { "10": 20, "11": null } })).unwrap();
        assert_eq!(map.new_id(10, Some(0)), Some(20));
        assert_eq!(map.new_id(11, None), Some(-1));
        assert_eq!(map.new_id(99, None), None, "rows the server didn't touch keep their id");

        let list = Rebind::from_reply(&json!({ "survivor_user_row_ids": [30, null] })).unwrap();
        assert_eq!(list.new_id(10, Some(0)), Some(30));
        assert_eq!(list.new_id(12, Some(1)), Some(-1));
        assert_eq!(list.new_id(11, None), None, "the list only covers your messages");

        assert!(Rebind::from_reply(&json!({ "status": "streaming" })).is_none());
    }
}
