//! Editing a message you sent, against a fake gateway.

use gpui_kit::TestAppContext;
use serde_json::{Value, json};

use super::queue_tests::{fake_gateway_with, state_with, wait_for};
use super::Role;

/// Answers a truncating submit with the kept rows' new ids, the way the
/// server reports them after rewriting the history.
fn reply(request: &Value) -> Value {
    if request["method"] == "prompt.submit" {
        json!({ "status": "streaming", "user_row_id": 40, "survivor_row_id_map": { "10": 30, "11": 31 } })
    } else {
        json!({ "status": "ok" })
    }
}

#[gpui_kit::test]
fn editing_an_older_message_cuts_the_history_there(cx: &mut TestAppContext) {
    let (state, requests, _dir) = state_with(cx, fake_gateway_with(reply));
    state.update(cx, |s, _| {
        s.load_transcript(vec![
            json!({ "role": "user", "text": "hello", "rowId": 10 }),
            json!({ "role": "assistant", "text": "Hi!", "rowId": 11 }),
            json!({ "role": "user", "text": "look at these", "rowId": 12,
                    "attachments": [{ "kind": "image", "name": "a.png", "path": "/srv/a.png" },
                                    { "kind": "file", "name": "notes.txt", "path": "/srv/notes.txt" }] }),
            json!({ "role": "assistant", "text": "Nice.", "rowId": 13 }),
            json!({ "role": "user", "text": "thanks", "rowId": 14 }),
            json!({ "role": "assistant", "text": "Anytime.", "rowId": 15 }),
        ]);
        s.start_edit(2);
    });
    assert_eq!(state.read_with(cx, |s, _| s.editing_index()), Some(2));

    state.update(cx, |s, cx| s.resend_from(2, "look at these again", cx));
    state.read_with(cx, |s, _| {
        assert_eq!(s.editing, None);
        assert!(s.streaming);
        let texts: Vec<&str> = s.rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["hello", "Hi!", "look at these again"], "the later turns are gone");
        assert_eq!(s.rows[2].attachments.len(), 2, "the edit keeps its attachments");
    });

    let attached = wait_for(&requests, "image.attach", 1);
    assert_eq!(attached[0]["path"], "/srv/a.png");
    let submit = &wait_for(&requests, "prompt.submit", 1)[0];
    assert_eq!(submit["text"], "@file:/srv/notes.txt\n\nlook at these again");
    assert_eq!(submit["truncate_before_row_id"], 12);
    assert_eq!(submit["confirm_truncate"], true);
    assert_eq!(submit["confirm_deep_truncate"], true, "a later message of yours goes too");
    assert!(submit.get("confirm_empty_truncate").is_none());
    assert_eq!(submit["rebind_survivor_row_ids"], json!([10, 11]));

    // The kept rows take the ids the server gave them.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.read_with(cx, |s, _| s.rows[2].row_id) != 40 && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    state.read_with(cx, |s, _| {
        let ids: Vec<i64> = s.rows.iter().map(|r| r.row_id).collect();
        assert_eq!(ids, [30, 31, 40]);
        assert_eq!(s.rows[2].role, Role::User);
    });
}
