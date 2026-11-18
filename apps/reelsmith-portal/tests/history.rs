//! The edit log is append-only. Undo, redo and restore add steps; they do not delete any.

use reelsmith_portal::edit::{self, client_actor};
use reelsmith_portal::history::{History, client_edits};
use reelsmith_portal::model::{Actor, AspectSource, Clip, ClipKind, FinalType, Gain, MediaTick, Snapshot, Timeline, action};

fn tick(frames: i64) -> MediaTick {
    MediaTick(frames * (reelsmith_time::TICKS_PER_SECOND / 30))
}

fn clip(id: &str, frames: i64) -> Clip {
    Clip {
        id: id.to_string(),
        track: "V1".to_string(),
        name: id.to_string(),
        kind: ClipKind::Video,
        start: MediaTick::ZERO,
        duration: tick(frames),
        source_in: MediaTick::ZERO,
        source_duration: tick(300),
        gain_permille: Gain::UNITY,
        muted: false,
        linked: None,
        text: String::new(),
        source_width: 1080,
        source_height: 1920,
    }
}

fn snapshot(frames: i64) -> Snapshot {
    let mut timeline = Timeline::blank(1080, 1920, AspectSource::Raw).expect("timeline");
    timeline.clips.push(clip("close", frames));
    Snapshot::new(timeline, FinalType::Reel)
}

fn commit(history: &mut History, actor: Actor, reason: Option<&str>, after: Snapshot) {
    let summary = "edit".to_string();
    history
        .commit(reelsmith_portal::history::Commit {
            actor,
            at: "2026-11-18T12:00:00Z".to_string(),
            action: action::TRIM.to_string(),
            targets: vec!["close".to_string()],
            summary,
            reason: reason.map(str::to_string),
            after,
            relates_to: None,
        })
        .expect("commit");
}

#[test]
fn undo_redo_and_restore_keep_every_step() {
    let mut history = History::new(snapshot(120));
    let mut shorter = snapshot(90);
    edit::set_length(&mut shorter, "close", 90).expect("length");
    commit(&mut history, client_actor("Harbor"), None, shorter);
    assert_eq!(history.current().expect("now").timeline.clip("close").expect("clip").duration, tick(90));
    let before_undo = history.events.len();
    history.undo(client_actor("Harbor"), "2026-11-18T12:01:00Z".to_string(), None).expect("undo");
    assert_eq!(history.current().expect("now").timeline.clip("close").expect("clip").duration, tick(120));
    assert!(history.events.len() > before_undo, "undo appends a step");
    history.redo(client_actor("Harbor"), "2026-11-18T12:02:00Z".to_string()).expect("redo");
    assert_eq!(history.current().expect("now").timeline.clip("close").expect("clip").duration, tick(90));
    history.restore(Actor::Admin, "2026-11-18T12:03:00Z".to_string(), Some(1), None).expect("restore");
    assert_eq!(history.current().expect("now").timeline.clip("close").expect("clip").duration, tick(90));
    let logged = history.events.len();
    history.undo(Actor::Admin, "2026-11-18T12:04:00Z".to_string(), None).expect("undo restore");
    assert_eq!(history.events.len(), logged + 1);
    assert!(history.events.iter().any(|event| event.action == action::UNDO));
    assert!(history.events.iter().any(|event| event.action == action::RESTORE));
}

#[test]
fn a_new_edit_after_undo_drops_redo_and_keeps_the_log() {
    let mut history = History::new(snapshot(120));
    let mut mid = snapshot(100);
    edit::set_length(&mut mid, "close", 100).expect("length");
    commit(&mut history, client_actor("Harbor"), None, mid);
    history.undo(client_actor("Harbor"), "t".to_string(), None).expect("undo");
    assert!(history.can_redo());
    let mut other = snapshot(80);
    edit::set_length(&mut other, "close", 80).expect("length");
    commit(&mut history, client_actor("Harbor"), None, other);
    assert!(!history.can_redo());
    assert!(history.events.len() >= 3);
    assert_eq!(history.current().expect("now").timeline.clip("close").expect("clip").duration, tick(80));
}

#[test]
fn agent_cannot_overwrite_a_client_edit_without_a_reason() {
    let mut history = History::new(snapshot(120));
    let mut trimmed = snapshot(90);
    edit::set_length(&mut trimmed, "close", 90).expect("length");
    commit(&mut history, client_actor("Harbor"), None, trimmed);
    let mut again = history.current().expect("now");
    edit::set_length(&mut again, "close", 100).expect("length");
    let denied = history.commit(reelsmith_portal::history::Commit {
        actor: Actor::Agent,
        at: "t".to_string(),
        action: action::TRIM.to_string(),
        targets: vec!["close".to_string()],
        summary: "silent".to_string(),
        reason: None,
        after: again.clone(),
        relates_to: None,
    });
    assert!(denied.is_err(), "a silent agent overwrite is rejected");
    assert_eq!(history.current().expect("now").timeline.clip("close").expect("clip").duration, tick(90));
    history
        .commit(reelsmith_portal::history::Commit {
            actor: Actor::Agent,
            at: "t2".to_string(),
            action: action::TRIM.to_string(),
            targets: vec!["close".to_string()],
            summary: "put the word back".to_string(),
            reason: Some("The cut was landing inside the last word.".to_string()),
            after: again,
            relates_to: None,
        })
        .expect("reason allows the edit");
    let event = history.events.last().expect("event");
    assert_eq!(event.reason.as_deref(), Some("The cut was landing inside the last word."));
    assert_eq!(client_edits(&history).len(), 1);
}

#[test]
fn one_second_is_thirty_frames_on_the_portal_time_base() {
    let timeline = Timeline::blank(1080, 1920, AspectSource::Raw).expect("timeline");
    let second = timeline.frame_to_tick(30).expect("frames");
    assert_eq!(second.to_tick(), reelsmith_time::Tick(reelsmith_time::TICKS_PER_SECOND));
    assert_eq!(timeline.tick_to_frame(second).expect("back"), 30);
}

#[test]
fn snap_pulls_a_trim_to_the_word_boundary() {
    let mut timeline = Timeline::blank(1080, 1920, AspectSource::Raw).expect("timeline");
    timeline.snap_words = true;
    timeline.clips.push(clip("talk", 40));
    timeline.words.push(reelsmith_portal::model::Word { start: tick(10), end: tick(20), text: "doors".to_string() });
    let mut snapshot = Snapshot::new(timeline, FinalType::Reel);
    edit::set_length(&mut snapshot, "talk", 11).expect("snap");
    let frames = snapshot.timeline.tick_to_frame(snapshot.timeline.clip("talk").expect("clip").duration).expect("frames");
    assert_eq!(frames, 10, "frame 11 is within two frames of the word edge at 10");
    snapshot.timeline.snap_words = false;
    edit::set_length(&mut snapshot, "talk", 11).expect("free");
    let frames = snapshot.timeline.tick_to_frame(snapshot.timeline.clip("talk").expect("clip").duration).expect("frames");
    assert_eq!(frames, 11);
}
