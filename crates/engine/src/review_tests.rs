//! Review links: the role gate runs only on top-level commands, the outbox records each
//! top-level command once, and a recorded log replays to the same project.

use serde_json::{Value, json};

use crate::Session;
use crate::commands::command_specs;
use crate::review::{Role, allows, not_in_portal, replay_safe};

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn first_video_clip(s: &Session) -> u64 {
    let seq = s.active_sequence().unwrap_or_else(|| panic!("the demo project has no sequence"));
    let clip = seq.video_tracks.first().and_then(|t| t.items.first()).unwrap_or_else(|| panic!("no clip on V1"));
    serde_json::to_value(clip.id).ok().and_then(|v| v.as_u64()).unwrap_or_else(|| panic!("clip ids are numbers"))
}

fn project_json(s: &Session) -> Value {
    serde_json::to_value(&*s.project).unwrap_or(Value::Null)
}

#[test]
fn every_journaled_command_is_classified() {
    let unclassified: Vec<&str> = command_specs().iter().filter(|c| c.journal && !not_in_portal(c.id) && !replay_safe(c.id)).map(|c| c.id).collect();
    assert!(unclassified.is_empty(), "add these to REPLAY_SAFE_NAMESPACES or the refused lists in review.rs: {unclassified:?}");
}

#[test]
fn reviewer_edits_are_real_journaled_commands() {
    for id in crate::review::REVIEWER_EDITS {
        let spec = crate::commands::find(id).unwrap_or_else(|| panic!("REVIEWER_EDITS names {id}, which is not a command"));
        assert!(spec.journal, "{id} is not journaled, so it would never reach the log");
        assert!(replay_safe(id), "{id} is a reviewer edit but not replay-safe");
    }
}

#[test]
fn roles_allow_what_the_design_says() {
    assert!(allows(Role::Reviewer, "timeline.trim").is_ok());
    assert!(allows(Role::Reviewer, "timeline.razor").is_ok());
    assert!(allows(Role::Reviewer, "playhead.set").is_ok());
    assert!(allows(Role::Reviewer, "file.importDemoFootage").is_err());
    assert!(allows(Role::Reviewer, "file.open").is_err());
    assert!(allows(Role::Reviewer, "clip.nest").is_err(), "nesting is the editor's");
    assert!(allows(Role::Watcher, "timeline.trim").is_err());
    assert!(allows(Role::Watcher, "playhead.set").is_ok());
    assert!(allows(Role::Editor, "clip.nest").is_ok());
    assert!(allows(Role::Editor, "file.open").is_err(), "a portal session never opens files itself");
}

#[test]
fn the_gate_refuses_a_top_level_command_and_leaves_the_project_alone() {
    // `clip.enable` toggles a clip; a watcher may not run it. Without the gate it succeeds, so
    // only the gate can make it fail here.
    let clip = first_video_clip(&demo());
    let mut open = demo();
    assert!(open.execute("clip.enable", json!({"clips": [clip]})).is_ok(), "without a policy the command runs");
    let mut s = demo();
    s.enter_review(Role::Watcher);
    let before = project_json(&s);
    let r = s.execute("clip.enable", json!({"clips": [clip]}));
    assert!(r.is_err(), "a watcher may not edit");
    assert_eq!(project_json(&s), before);
    assert!(s.take_executed().is_empty(), "a refused command is not logged");
}

#[test]
fn the_gate_does_not_refuse_commands_an_allowed_command_runs() {
    // timeline.delete runs edit.clear / edit.rippleDelete inside itself (commands.rs); and a
    // watcher-only policy would refuse those if the gate looked at nested calls.
    let mut s = demo();
    let clip = first_video_clip(&s);
    s.enter_review(Role::Reviewer);
    s.execute("timeline.select", json!({"clips": [clip]})).unwrap();
    s.execute("timeline.delete", json!({})).unwrap();
    let ids: Vec<String> = s.take_executed().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, vec!["timeline.select".to_string(), "timeline.delete".to_string()], "nested commands are logged once, as their outer command");
}

#[test]
fn a_recorded_log_replays_to_the_same_project() {
    let mut page = demo();
    let base = page.project.clone();
    let clip = first_video_clip(&page);
    page.enter_review(Role::Reviewer);
    page.execute("timeline.trim", json!({"clip": clip, "edge": "out", "mode": "regular", "deltaFrames": -12})).unwrap();
    page.execute("timeline.select", json!({"clips": [clip]})).unwrap();
    page.execute("clip.audioGain", json!({"db": -3.0})).unwrap();
    page.execute("playhead.set", json!({"seconds": 1.0})).unwrap();
    page.execute("timeline.razor", json!({"time": reelsmith_time::TICKS_PER_SECOND * 2, "track": "V1"})).unwrap();
    page.execute("edit.undo", json!({})).unwrap();
    let log = page.take_executed();
    assert!(log.iter().any(|c| c.id == "timeline.trim" && c.project_changed));
    assert!(log.iter().any(|c| c.id == "playhead.set" && !c.project_changed), "a playhead move does not change the project");

    let mut server = Session::default();
    server.execute("file.openDemoProject", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&*server.project).ok(), serde_json::to_value(&*base).ok(), "the server starts from the same base");
    server.enter_review(Role::Reviewer);
    for c in &log {
        server.execute(&c.id, c.params.clone()).unwrap_or_else(|e| panic!("replaying {} failed: {e}", c.id));
    }
    assert_eq!(project_json(&server), project_json(&page), "replaying the log gives the page's project");
}
