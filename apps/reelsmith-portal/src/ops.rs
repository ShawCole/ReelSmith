//! Turn one API edit into a new snapshot. Undo, redo and restore are handled by the history log.

use crate::edit::{self, change_format};
use crate::error::Error;
use crate::model::{FinalType, Region, Snapshot, Status};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct EditBody {
    #[serde(default)]
    pub reason: Option<String>,
    pub edit: EditRequest,
}

#[derive(Debug, Deserialize)]
pub struct RegionIn {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditRequest {
    Trim { clip_id: String, edge: String, frames: i64 },
    Length { clip_id: String, frames: i64 },
    Razor { clip_id: String, frame: i64 },
    Move { clip_id: String, frame: i64, track: Option<String> },
    Lift { clip_id: String },
    Gain { clip_id: String, permille: u16, muted: Option<bool> },
    Caption { caption_id: String, text: String },
    CaptionMove { caption_id: String, frame: i64, frames: Option<i64> },
    Snap { enabled: bool },
    ChangeFormat { width: u32, height: u32 },
    Comment { body: String, start_frame: i64, end_frame: i64, region: Option<RegionIn> },
    Resolve { comment_id: String, resolved: bool, fixed_in: Option<String> },
    Reply { comment_id: String, body: String },
    Approve { note: String },
    RequestChanges { note: String },
    Decisions { text: String },
    FinalType { final_type: FinalType },
    Status { status: Status },
    PlaceBroll { item_id: String },
    Undo,
    Redo,
    Restore { event_id: Option<u64> },
}

pub struct Applied {
    pub action: &'static str,
    pub targets: Vec<String>,
    pub summary: String,
    pub after: Snapshot,
}

pub fn apply_request(snapshot: &Snapshot, actor_name: &str, at: &str, edit: &EditRequest) -> Result<Applied, Error> {
    let mut after = snapshot.clone();
    let (action, targets, summary) = match edit {
        EditRequest::Trim { clip_id, edge, frames } => {
            let edge = parse_edge(edge)?;
            let targets = edit::trim(&mut after, clip_id, edge, *frames)?;
            let which = if matches!(edge, crate::model::Edge::In) { "in" } else { "out" };
            (crate::model::action::TRIM, targets, format!("Trimmed the {which} point of {clip_id} by {frames} frames"))
        }
        EditRequest::Length { clip_id, frames } => {
            let targets = edit::set_length(&mut after, clip_id, *frames)?;
            (crate::model::action::LENGTH, targets, format!("Set {clip_id} to {frames} frames"))
        }
        EditRequest::Razor { clip_id, frame } => {
            let targets = edit::razor(&mut after, clip_id, *frame)?;
            (crate::model::action::RAZOR, targets, format!("Split {clip_id} at frame {frame}"))
        }
        EditRequest::Move { clip_id, frame, track } => {
            let targets = edit::move_clip(&mut after, clip_id, *frame, track.as_deref())?;
            (crate::model::action::MOVE, targets, format!("Moved {clip_id} to frame {frame}"))
        }
        EditRequest::Lift { clip_id } => {
            let targets = edit::lift(&mut after, clip_id)?;
            (crate::model::action::LIFT, targets, format!("Lifted {clip_id} off the timeline"))
        }
        EditRequest::Gain { clip_id, permille, muted } => {
            let targets = edit::gain(&mut after, clip_id, *permille, *muted)?;
            (crate::model::action::GAIN, targets, format!("Set gain on {clip_id} to {permille} permille"))
        }
        EditRequest::Caption { caption_id, text } => {
            let targets = edit::caption_text(&mut after, caption_id, text)?;
            (crate::model::action::CAPTION, targets, format!("Edited caption {caption_id}"))
        }
        EditRequest::CaptionMove { caption_id, frame, frames } => {
            let targets = edit::caption_move(&mut after, caption_id, *frame, *frames)?;
            (crate::model::action::CAPTION_MOVE, targets, format!("Moved caption {caption_id}"))
        }
        EditRequest::Snap { enabled } => {
            let targets = edit::set_snap(&mut after, *enabled)?;
            let summary = if *enabled { "Snap to word boundaries is on" } else { "Snap to word boundaries is off" };
            (crate::model::action::SNAP, targets, summary.to_string())
        }
        EditRequest::ChangeFormat { width, height } => {
            let targets = change_format(&mut after, *width, *height)?;
            (crate::model::action::FORMAT, targets, format!("Changed format to {width}×{height}"))
        }
        EditRequest::Comment { body, start_frame, end_frame, region } => {
            let region = match region {
                Some(region) => Some(Region::new(region.x, region.y, region.w, region.h)?),
                None => None,
            };
            let targets = edit::add_comment(&mut after, actor_name, *start_frame, *end_frame, region, body)?;
            (crate::model::action::COMMENT, targets, "Added a comment".to_string())
        }
        EditRequest::Resolve { comment_id, resolved, fixed_in } => {
            let targets = edit::resolve_comment(&mut after, comment_id, *resolved, fixed_in.clone())?;
            let summary = if *resolved { "Resolved a comment" } else { "Reopened a comment" };
            (crate::model::action::RESOLVE, targets, summary.to_string())
        }
        EditRequest::Reply { comment_id, body } => {
            let targets = edit::reply(&mut after, comment_id, actor_name, at, body)?;
            (crate::model::action::REPLY, targets, "Replied to a comment".to_string())
        }
        EditRequest::Approve { note } => {
            let targets = edit::approve(&mut after, actor_name, at, note)?;
            (crate::model::action::APPROVE, targets, "Approved this version".to_string())
        }
        EditRequest::RequestChanges { note } => {
            let targets = edit::request_changes(&mut after, actor_name, at, note)?;
            (crate::model::action::REQUEST, targets, "Requested changes".to_string())
        }
        EditRequest::Decisions { text } => {
            let targets = edit::set_decisions(&mut after, text)?;
            (crate::model::action::DECISIONS, targets, "Updated the decisions notes".to_string())
        }
        EditRequest::FinalType { final_type } => {
            let targets = edit::set_final_type(&mut after, *final_type)?;
            (crate::model::action::FINAL_TYPE, targets, format!("Final type is now {}", final_type.label()))
        }
        EditRequest::Status { status } => {
            let targets = edit::set_status(&mut after, *status)?;
            (crate::model::action::STATUS, targets, format!("Status is now {}", status.label()))
        }
        EditRequest::PlaceBroll { item_id } => {
            let targets = edit::place_broll(&mut after, item_id)?;
            (crate::model::action::PLACE_BROLL, targets, format!("Placed B-roll {item_id}"))
        }
        EditRequest::Undo | EditRequest::Redo | EditRequest::Restore { .. } => {
            return Err(Error::invalid("undo, redo and restore are separate history steps"));
        }
    };
    Ok(Applied { action, targets, summary, after })
}

fn parse_edge(edge: &str) -> Result<crate::model::Edge, Error> {
    match edge {
        "in" => Ok(crate::model::Edge::In),
        "out" => Ok(crate::model::Edge::Out),
        _ => Err(Error::invalid("edge must be in or out")),
    }
}

pub fn reviewer_may(edit: &EditRequest) -> bool {
    matches!(
        edit,
        EditRequest::Trim { .. }
            | EditRequest::Length { .. }
            | EditRequest::Razor { .. }
            | EditRequest::Move { .. }
            | EditRequest::Lift { .. }
            | EditRequest::Gain { .. }
            | EditRequest::Caption { .. }
            | EditRequest::CaptionMove { .. }
            | EditRequest::Snap { .. }
            | EditRequest::ChangeFormat { .. }
            | EditRequest::Comment { .. }
            | EditRequest::Resolve { .. }
            | EditRequest::Reply { .. }
            | EditRequest::Approve { .. }
            | EditRequest::RequestChanges { .. }
            | EditRequest::FinalType { .. }
            | EditRequest::Undo
            | EditRequest::Redo
            | EditRequest::Restore { .. }
    )
}

pub fn is_approval(edit: &EditRequest) -> bool {
    matches!(edit, EditRequest::Approve { .. } | EditRequest::RequestChanges { .. })
}
