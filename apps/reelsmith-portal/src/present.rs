//! JSON the browser draws. Reviewer payloads omit the raw library, word alignment,
//! decisions notes and QC flags.

use crate::brief::{self, BriefPlan};
use crate::error::Error;
use crate::history::History;
use crate::model::{AspectSource, Comment, EditEvent, LibraryItem, Snapshot, Status, Timeline, Word};
use serde::Serialize;

#[derive(Serialize)]
pub struct VersionPage {
    pub project_id: String,
    pub project_name: String,
    pub client_name: String,
    pub role: String,
    pub read_only: bool,
    pub read_only_reason: String,
    pub brief: BriefSummary,
    pub versions: Vec<VersionTab>,
    pub version: VersionBody,
}

#[derive(Serialize)]
pub struct BriefSummary {
    pub final_type: String,
    pub target: String,
    pub aspect_label: String,
    pub aspect_source: String,
    pub use_broll: bool,
    pub flags: Vec<crate::brief::BriefFlag>,
}

#[derive(Serialize)]
pub struct VersionTab {
    pub id: String,
    pub difference: String,
    pub status: Status,
    pub mark: Option<String>,
}

#[derive(Serialize)]
pub struct VersionBody {
    pub id: String,
    pub difference: String,
    pub status: Status,
    pub approval_note: Option<String>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub event_count: usize,
    pub state_of: Option<u64>,
    pub fps_num: u32,
    pub fps_den: u32,
    pub width: u32,
    pub height: u32,
    pub aspect_source: String,
    pub snap_words: bool,
    pub reframed: bool,
    pub duration_frames: i64,
    pub clips: Vec<ClipView>,
    pub captions: Vec<CaptionView>,
    pub comments: Vec<CommentView>,
    pub events: Vec<EventView>,
    pub words: Vec<WordView>,
    pub library: Vec<LibraryView>,
    pub decisions: String,
    pub qc: Vec<crate::brief::QcFlag>,
    pub meters_note: String,
}

#[derive(Serialize)]
pub struct ClipView {
    pub id: String,
    pub track: String,
    pub name: String,
    pub kind: String,
    pub start_frame: i64,
    pub duration_frames: i64,
    pub gain_permille: u16,
    pub muted: bool,
    pub text: String,
    pub source_width: u32,
    pub source_height: u32,
}

#[derive(Serialize)]
pub struct CaptionView {
    pub id: String,
    pub start_frame: i64,
    pub duration_frames: i64,
    pub text: String,
    pub anchor_y: u16,
}

#[derive(Serialize)]
pub struct CommentView {
    pub id: String,
    pub author: String,
    pub start_frame: i64,
    pub end_frame: i64,
    pub region: Option<RegionView>,
    pub body: String,
    pub resolved: bool,
    pub fixed_in: Option<String>,
    pub replies: Vec<ReplyView>,
}

#[derive(Serialize)]
pub struct RegionView {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Serialize)]
pub struct ReplyView {
    pub author: String,
    pub at: String,
    pub body: String,
}

#[derive(Serialize)]
pub struct EventView {
    pub id: u64,
    pub at: String,
    pub actor: String,
    pub action: String,
    pub summary: String,
    pub reason: Option<String>,
    pub targets: Vec<String>,
    pub in_effect: bool,
}

#[derive(Serialize)]
pub struct WordView {
    pub start_frame: i64,
    pub end_frame: i64,
    pub text: String,
}

#[derive(Serialize)]
pub struct LibraryView {
    pub id: String,
    pub name: String,
    pub role: String,
    pub width: u32,
    pub height: u32,
    pub duration_frames: i64,
    pub selected: bool,
}

#[derive(Serialize)]
pub struct Head {
    pub event_count: usize,
    pub state_of: Option<u64>,
    pub status: Status,
}

#[derive(Serialize)]
pub struct CompareView {
    pub left: String,
    pub right: String,
    pub only_left: Vec<String>,
    pub only_right: Vec<String>,
    pub duration_deltas: Vec<String>,
    pub caption_changes: Vec<String>,
}

pub fn head(history: &History) -> Result<Head, Error> {
    let status = history.current()?.status;
    Ok(Head { event_count: history.events.len(), state_of: history.state_of, status })
}

pub fn version_body(id: &str, difference: &str, history: &History, plan: &BriefPlan, agent: bool) -> Result<VersionBody, Error> {
    let snapshot = history.current()?;
    let timeline = &snapshot.timeline;
    let chain: Vec<u64> = history.chain().iter().map(|event| event.id).collect();
    Ok(VersionBody {
        id: id.to_string(),
        difference: difference.to_string(),
        status: snapshot.status,
        approval_note: snapshot.approval.as_ref().map(|approval| format!("{} — {}", approval.mark.as_str(), approval.note)),
        can_undo: history.can_undo(),
        can_redo: history.can_redo(),
        event_count: history.events.len(),
        state_of: history.state_of,
        fps_num: timeline.fps_num,
        fps_den: timeline.fps_den,
        width: timeline.width,
        height: timeline.height,
        aspect_source: aspect_name(timeline.aspect_source).to_string(),
        snap_words: timeline.snap_words,
        reframed: timeline.reframed,
        duration_frames: timeline.tick_to_frame(timeline.duration())?,
        clips: clips(timeline)?,
        captions: captions(timeline)?,
        comments: comments(&snapshot)?,
        events: events(&history.events, &chain),
        words: if agent { words(timeline)? } else { Vec::new() },
        library: if agent { library(timeline)? } else { Vec::new() },
        decisions: if agent { snapshot.decisions.clone() } else { String::new() },
        qc: if agent { brief::qc_flags(plan, &snapshot) } else { Vec::new() },
        meters_note: if agent {
            "Loudness is not measured in this build. Flags come from the timeline and the upload brief.".to_string()
        } else {
            String::new()
        },
    })
}

pub fn compare(left_name: &str, right_name: &str, left: &Snapshot, right: &Snapshot) -> Result<CompareView, Error> {
    let mut only_left = Vec::new();
    let mut only_right = Vec::new();
    let mut duration_deltas = Vec::new();
    for clip in &left.timeline.clips {
        if right.timeline.clips.iter().all(|other| other.id != clip.id) {
            only_left.push(format!("{} ({})", clip.name, clip.id));
        }
    }
    for clip in &right.timeline.clips {
        match left.timeline.clips.iter().find(|other| other.id == clip.id) {
            None => only_right.push(format!("{} ({})", clip.name, clip.id)),
            Some(other) if other.duration != clip.duration => {
                let left_frames = left.timeline.tick_to_frame(other.duration)?;
                let right_frames = right.timeline.tick_to_frame(clip.duration)?;
                duration_deltas.push(format!("{} is {left_frames} frames on the left and {right_frames} on the right", clip.name));
            }
            Some(_) => {}
        }
    }
    let mut caption_changes = Vec::new();
    for caption in &left.timeline.captions {
        match right.timeline.captions.iter().find(|other| other.id == caption.id) {
            Some(other) if other.text != caption.text => caption_changes.push(format!("{} → {}", caption.text, other.text)),
            None => caption_changes.push(format!("only on the left: {}", caption.text)),
            Some(_) => {}
        }
    }
    Ok(CompareView { left: left_name.to_string(), right: right_name.to_string(), only_left, only_right, duration_deltas, caption_changes })
}

fn clips(timeline: &Timeline) -> Result<Vec<ClipView>, Error> {
    let mut out = Vec::new();
    for clip in &timeline.clips {
        out.push(ClipView {
            id: clip.id.clone(),
            track: clip.track.clone(),
            name: clip.name.clone(),
            kind: clip_kind_name(clip.kind).to_string(),
            start_frame: timeline.tick_to_frame(clip.start)?,
            duration_frames: timeline.tick_to_frame(clip.duration)?,
            gain_permille: clip.gain_permille.0,
            muted: clip.muted,
            text: clip.text.clone(),
            source_width: clip.source_width,
            source_height: clip.source_height,
        });
    }
    Ok(out)
}

fn captions(timeline: &Timeline) -> Result<Vec<CaptionView>, Error> {
    let mut out = Vec::new();
    for caption in &timeline.captions {
        out.push(CaptionView {
            id: caption.id.clone(),
            start_frame: timeline.tick_to_frame(caption.start)?,
            duration_frames: timeline.tick_to_frame(caption.duration)?,
            text: caption.text.clone(),
            anchor_y: caption.anchor_y.0,
        });
    }
    Ok(out)
}

fn comments(snapshot: &Snapshot) -> Result<Vec<CommentView>, Error> {
    let mut out = Vec::new();
    for comment in &snapshot.comments {
        out.push(comment_view(&snapshot.timeline, comment)?);
    }
    Ok(out)
}

fn comment_view(timeline: &Timeline, comment: &Comment) -> Result<CommentView, Error> {
    Ok(CommentView {
        id: comment.id.clone(),
        author: comment.author.clone(),
        start_frame: timeline.tick_to_frame(comment.start)?,
        end_frame: timeline.tick_to_frame(comment.end)?,
        region: comment.region.map(|region| RegionView { x: region.x.0, y: region.y.0, w: region.w.0, h: region.h.0 }),
        body: comment.body.clone(),
        resolved: comment.resolved,
        fixed_in: comment.fixed_in.clone(),
        replies: comment.replies.iter().map(|reply| ReplyView { author: reply.author.clone(), at: reply.at.clone(), body: reply.body.clone() }).collect(),
    })
}

fn events(events: &[EditEvent], chain: &[u64]) -> Vec<EventView> {
    events
        .iter()
        .map(|event| EventView {
            id: event.id,
            at: event.at.clone(),
            actor: event.actor.label(),
            action: event.action.clone(),
            summary: event.summary.clone(),
            reason: event.reason.clone(),
            targets: event.targets.clone(),
            in_effect: chain.contains(&event.id),
        })
        .collect()
}

fn words(timeline: &Timeline) -> Result<Vec<WordView>, Error> {
    let mut out = Vec::new();
    for word in &timeline.words {
        out.push(word_view(timeline, word)?);
    }
    Ok(out)
}

fn word_view(timeline: &Timeline, word: &Word) -> Result<WordView, Error> {
    Ok(WordView { start_frame: timeline.tick_to_frame(word.start)?, end_frame: timeline.tick_to_frame(word.end)?, text: word.text.clone() })
}

fn library(timeline: &Timeline) -> Result<Vec<LibraryView>, Error> {
    let mut out = Vec::new();
    for item in &timeline.library {
        out.push(library_view(timeline, item)?);
    }
    Ok(out)
}

fn aspect_name(source: AspectSource) -> &'static str {
    match source {
        AspectSource::Raw => "raw",
        AspectSource::Profile => "profile",
        AspectSource::ChangeFormat => "change_format",
    }
}

fn clip_kind_name(kind: crate::model::ClipKind) -> &'static str {
    match kind {
        crate::model::ClipKind::Video => "video",
        crate::model::ClipKind::Audio => "audio",
        crate::model::ClipKind::Music => "music",
        crate::model::ClipKind::Broll => "broll",
        crate::model::ClipKind::Title => "title",
        crate::model::ClipKind::EndCard => "end_card",
    }
}

fn library_view(timeline: &Timeline, item: &LibraryItem) -> Result<LibraryView, Error> {
    Ok(LibraryView {
        id: item.id.clone(),
        name: item.name.clone(),
        role: match item.role {
            crate::model::MediaRole::Aroll => "aroll",
            crate::model::MediaRole::Broll => "broll",
        }
        .to_string(),
        width: item.width,
        height: item.height,
        duration_frames: timeline.tick_to_frame(item.duration)?,
        selected: item.selected,
    })
}
