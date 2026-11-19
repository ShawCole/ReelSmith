//! The action log is append-only, and undo / redo are steps that name what they revert.

use reelsmith_portal::history::{ActionEvent, Stacks, client_actions, effective_chain, read_log, targets_of, write_log};
use reelsmith_portal::model::Actor;
use serde_json::json;

fn step(seq: u64, actor: Actor, id: &str, changed: bool, reverts: Option<u64>) -> ActionEvent {
    ActionEvent {
        seq,
        at: "2026-11-18T12:00:00Z".to_string(),
        actor,
        role: "reviewer".to_string(),
        id: id.to_string(),
        params: json!({"clip": 7}),
        label: id.to_string(),
        project_changed: changed,
        based_on_seq: seq - 1,
        reason: None,
        client_ts: None,
        reverts_seq: reverts,
        targets: vec!["7".to_string()],
    }
}

fn harbor() -> Actor {
    Actor::client("c_1", "Harbor & Co")
}

#[test]
fn the_store_refuses_to_shorten_the_log() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let path = dir.path().join("actions.jsonl");
    let three = vec![
        step(1, harbor(), "timeline.trim", true, None),
        step(2, harbor(), "playhead.set", false, None),
        step(3, harbor(), "timeline.razor", true, None),
    ];
    write_log(&path, &three).unwrap_or_else(|e| panic!("write: {e}"));
    let refused = write_log(&path, &three[..2]);
    assert!(refused.is_err(), "writing fewer steps than the file holds must fail");
    assert_eq!(refused.err().map(|e| e.status()), Some(409));
    assert_eq!(read_log(&path).unwrap_or_default(), three, "the file still holds every step");
    let empty = write_log(&path, &[]);
    assert!(empty.is_err(), "an empty log cannot replace a written one");
    assert_eq!(read_log(&path).unwrap_or_default().len(), 3);
}

#[test]
fn the_store_appends_only_the_new_steps_and_keeps_numbering() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let path = dir.path().join("actions.jsonl");
    let mut log = vec![step(1, harbor(), "timeline.trim", true, None)];
    write_log(&path, &log).unwrap_or_else(|e| panic!("write: {e}"));
    log.push(step(2, Actor::agent(), "clip.nest", true, None));
    write_log(&path, &log).unwrap_or_else(|e| panic!("write: {e}"));
    assert_eq!(read_log(&path).unwrap_or_default(), log);
    let gap = vec![log[0].clone(), log[1].clone(), step(9, harbor(), "timeline.trim", true, None)];
    assert!(write_log(&path, &gap).is_err(), "steps are numbered without gaps");
    assert_eq!(read_log(&path).unwrap_or_default().len(), 2);
}

#[test]
fn undo_and_redo_name_the_step_they_revert() {
    let mut events = vec![step(1, harbor(), "timeline.trim", true, None), step(2, harbor(), "playhead.set", false, None)];
    let stacks = Stacks::of(&events);
    assert_eq!(stacks.reverts("edit.undo"), Some(1), "undo skips the playhead step");
    events.push(step(3, harbor(), "edit.undo", true, Some(1)));
    assert!(client_actions(&events).is_empty(), "an undone trim is not in effect");
    let stacks = Stacks::of(&events);
    assert_eq!(stacks.reverts("edit.redo"), Some(1));
    events.push(step(4, harbor(), "edit.redo", true, Some(1)));
    assert_eq!(effective_chain(&events).iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1]);
    events.push(step(5, harbor(), "edit.undo", true, Some(1)));
    events.push(step(6, Actor::agent(), "timeline.razor", true, None));
    let stacks = Stacks::of(&events);
    assert_eq!(stacks.reverts("edit.redo"), None, "a new edit after undo drops redo");
    assert_eq!(effective_chain(&events).iter().map(|e| e.seq).collect::<Vec<_>>(), vec![6]);
    assert!(client_actions(&events).is_empty(), "the agent's razor is not client evidence");
}

#[test]
fn targets_come_from_the_command_parameters() {
    assert_eq!(targets_of(&json!({"clip": 12, "edge": "out"})), vec!["12".to_string()]);
    assert_eq!(targets_of(&json!({"clips": [3, 4], "track": "V1"})), vec!["3".to_string(), "4".to_string(), "V1".to_string()]);
    assert!(targets_of(&json!({"seconds": 1.0})).is_empty());
}
