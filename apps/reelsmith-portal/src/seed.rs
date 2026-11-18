//! First-run demo: one client, one project, two versions, and a real edit history.
//! Tokens are generated into the data directory. They are not part of the source tree.

use crate::assemble::initial_snapshot;
use crate::brief::{self, LengthTarget, UploadBrief};
use crate::error::Error;
use crate::history::{Commit, History};
use crate::ids;
use crate::model::{Actor, ClipKind, FinalType, MediaRole, MediaTick, Region, Word, action};
use crate::persist::{self, BrollRecord, ClientRecord, ProjectRecord, Saved, VersionRecord};
use crate::{edit, model};
use reelsmith_time::TICKS_PER_SECOND;
use std::path::Path;

pub fn write(root: &Path, admin: Option<String>) -> Result<(), Error> {
    let admin_token = match admin {
        Some(token) => token,
        None => ids::token()?,
    };
    let client = ClientRecord { id: "c_harbor".to_string(), name: "Harbor & Co".to_string(), token: ids::token()?, created_at: stamp() };
    let broll = vec![
        broll(&client.id, "b_wide", "Workshop wide", 1920, 1080, seconds(4)),
        broll(&client.id, "b_hands", "Hands detail", 1920, 1080, seconds(3)),
        broll(&client.id, "b_street", "Street walk", 1080, 1920, seconds(5)),
    ];
    let brief = UploadBrief {
        project_name: "Spring workshop reel".to_string(),
        final_type: FinalType::Reel,
        target: Some(LengthTarget::Range { min: seconds(15), max: seconds(25) }),
        version_count: 2,
        differences: vec!["tighter hook".to_string(), "slower product beat".to_string()],
        use_broll: true,
        broll_library_ids: vec!["b_wide".to_string()],
        broll_uploads: Vec::new(),
        raw_files: vec![raw("Hook", 90), raw("Interview", 180), raw("Product", 150), raw("Close", 120)],
        change_format: false,
        explicit_width: None,
        explicit_height: None,
        profile_width: None,
        profile_height: None,
    };
    let known: Vec<String> = broll.iter().map(|item| item.id.clone()).collect();
    let plan =
        brief::validate_brief(&brief, &known).map_err(|errors| Error::invalid(errors.into_iter().map(|error| error.message).collect::<Vec<_>>().join("; ")))?;
    let mut assembly = initial_snapshot(&plan, &broll)?;
    paint_captions(&mut assembly)?;
    add_words(&mut assembly)?;
    let client_actor = Actor::Client { client_id: client.id.clone(), name: client.name.clone() };
    let mut primary = History::new(blank(&assembly)?);
    record(&mut primary, Actor::Agent, None, action::ASSEMBLE, "Assembled the workshop takes", assembly.clone())?;
    let close = clip_id(&primary.current()?, "Close", ClipKind::Video)?;
    record_with(&mut primary, client_actor.clone(), None, action::TRIM, "Trimmed the close", |snapshot| edit::trim(snapshot, &close, model::Edge::Out, -30))?;
    record_with(
        &mut primary,
        Actor::Agent,
        Some("Restored the last word of the CTA. The cut was landing inside the word."),
        action::TRIM,
        "Adjusted the close so the last word stays intact",
        |snapshot| edit::trim(snapshot, &close, model::Edge::Out, 12),
    )?;
    record_with(&mut primary, client_actor.clone(), None, action::COMMENT, "Noted the headline", |snapshot| {
        edit::add_comment(snapshot, "Harbor & Co", 24, 70, Some(Region::new(80, 140, 840, 180)?), "Make this headline a little smaller.")
    })?;
    record_with(&mut primary, client_actor.clone(), None, action::COMMENT, "Noted the interview line", |snapshot| {
        edit::add_comment(snapshot, "Harbor & Co", 112, 190, None, "Give this sentence a little more room.")
    })?;
    let music_note = {
        let mut id = String::new();
        record_with(&mut primary, client_actor.clone(), None, action::COMMENT, "Noted the music under the hook", |snapshot| {
            let targets = edit::add_comment(snapshot, "Harbor & Co", 12, 40, None, "The music is a little hot under the hook.")?;
            id = targets.first().cloned().unwrap_or_default();
            Ok(targets)
        })?;
        id
    };
    record_with(&mut primary, Actor::Agent, None, action::RESOLVE, "Marked the music note fixed in this cut", |snapshot| {
        edit::resolve_comment(snapshot, &music_note, true, Some("tighter hook".to_string()))
    })?;
    record_with(&mut primary, Actor::Agent, None, action::DECISIONS, "Wrote the decisions note", |snapshot| {
        edit::set_decisions(snapshot, "Kept the doors line. The close was tightened, then the last word was put back.")
    })?;
    record_with(&mut primary, Actor::Agent, None, action::STATUS, "Marked the cut ready for review", |snapshot| {
        edit::set_status(snapshot, model::Status::Ready)
    })?;

    let mut slower = History::new(blank(&assembly)?);
    record(&mut slower, Actor::Agent, None, action::ASSEMBLE, "Assembled the workshop takes", assembly.clone())?;
    let product = clip_id(&slower.current()?, "Product", ClipKind::Video)?;
    record_with(&mut slower, Actor::Agent, None, action::LENGTH, "Held the product shot longer", |snapshot| edit::set_length(snapshot, &product, 195))?;

    let versions = vec![
        VersionRecord { id: "v_hook".to_string(), difference: "tighter hook".to_string() },
        VersionRecord { id: "v_product".to_string(), difference: "slower product beat".to_string() },
    ];
    let project = ProjectRecord {
        id: "p_spring".to_string(),
        client_id: client.id.clone(),
        name: plan.project_name.clone(),
        reviewer_token: ids::token()?,
        agent_token: ids::token()?,
        plan,
        versions,
        created_at: stamp(),
    };
    persist::save_history(root, &project.id, "v_hook", &primary)?;
    persist::save_history(root, &project.id, "v_product", &slower)?;
    persist::save_saved(root, &Saved { admin_token, clients: vec![client], projects: vec![project], broll })?;
    Ok(())
}

fn record(history: &mut History, actor: Actor, reason: Option<&str>, action_name: &str, summary: &str, after: model::Snapshot) -> Result<(), Error> {
    let targets = after.timeline.clips.iter().map(|clip| clip.id.clone()).collect();
    history
        .commit(Commit {
            actor,
            at: stamp(),
            action: action_name.to_string(),
            targets,
            summary: summary.to_string(),
            reason: reason.map(str::to_string),
            after,
            relates_to: None,
        })
        .map(|_| ())
}

fn record_with(
    history: &mut History,
    actor: Actor,
    reason: Option<&str>,
    action_name: &str,
    summary: &str,
    edit_fn: impl FnOnce(&mut model::Snapshot) -> Result<Vec<String>, Error>,
) -> Result<(), Error> {
    let mut after = history.current()?;
    let targets = edit_fn(&mut after)?;
    history
        .commit(Commit {
            actor,
            at: stamp(),
            action: action_name.to_string(),
            targets,
            summary: summary.to_string(),
            reason: reason.map(str::to_string),
            after,
            relates_to: None,
        })
        .map(|_| ())
}

fn paint_captions(snapshot: &mut model::Snapshot) -> Result<(), Error> {
    let lines = ["Spring workshop", "We opened the doors", "Four sessions. One city.", "Come with us"];
    for (caption, line) in snapshot.timeline.captions.iter_mut().zip(lines) {
        caption.text = (*line).to_string();
    }
    Ok(())
}

fn add_words(snapshot: &mut model::Snapshot) -> Result<(), Error> {
    let words = [("we", 96, 110), ("opened", 112, 140), ("the", 142, 155), ("doors", 158, 190)];
    for (text, start, end) in words {
        snapshot.timeline.words.push(Word {
            start: snapshot.timeline.frame_to_tick(start)?,
            end: snapshot.timeline.frame_to_tick(end)?,
            text: text.to_string(),
        });
    }
    Ok(())
}

fn clip_id(snapshot: &model::Snapshot, name: &str, kind: ClipKind) -> Result<String, Error> {
    snapshot
        .timeline
        .clips
        .iter()
        .find(|clip| clip.name == name && clip.kind == kind)
        .map(|clip| clip.id.clone())
        .ok_or_else(|| Error::invalid(format!("demo clip {name} is missing")))
}

fn blank(snapshot: &model::Snapshot) -> Result<model::Snapshot, Error> {
    Ok(model::Snapshot::new(model::Timeline::blank(snapshot.timeline.width, snapshot.timeline.height, snapshot.timeline.aspect_source)?, snapshot.final_type))
}

fn raw(name: &str, frames: i64) -> brief::RawFile {
    brief::RawFile { name: name.to_string(), width: 1080, height: 1920, duration: frames_to_ticks(frames), role: MediaRole::Aroll }
}

fn broll(client_id: &str, id: &str, name: &str, width: u32, height: u32, duration: MediaTick) -> BrollRecord {
    BrollRecord { id: id.to_string(), client_id: client_id.to_string(), name: name.to_string(), width, height, duration }
}

fn frames_to_ticks(frames: i64) -> MediaTick {
    MediaTick(frames.saturating_mul(TICKS_PER_SECOND / 30))
}

fn seconds(seconds: i64) -> MediaTick {
    MediaTick(seconds.saturating_mul(TICKS_PER_SECOND))
}

fn stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
