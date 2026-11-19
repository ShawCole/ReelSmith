//! Feedback into the style pipeline (`docs/style-pipeline/SPEC.md` §5.1, PLAN §7).
//!
//! - `approval.json`: exactly SPEC §5.1, `{"version","mark":"good"|"not_good","note","by","at"}`.
//! - `feedback.json`, schema `reelsmith.portal.feedback/2`: the approval, the comments, and every
//!   CLIENT project-changing step in effect (command, params, targets, label, at), with the
//!   touched clips' durations in `base.fcproj` and `head.fcproj`.

use crate::history::{ActionEvent, client_actions};
use crate::model::{Approval, Comment, FinalType, Region};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA: &str = "reelsmith.portal.feedback/2";

/// SPEC §5.1, field for field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalJson {
    pub version: String,
    pub mark: String,
    pub note: String,
    pub by: String,
    pub at: String,
}

pub fn approval_json(version_label: &str, approval: &Approval) -> ApprovalJson {
    ApprovalJson {
        version: version_label.to_string(),
        mark: approval.mark.as_str().to_string(),
        note: approval.note.clone(),
        by: approval.by.clone(),
        at: approval.at.clone(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackFile {
    pub schema: String,
    pub client: String,
    pub project: String,
    pub final_type: FinalType,
    pub versions: Vec<VersionFeedback>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionFeedback {
    pub version_id: String,
    pub version: String,
    pub status: String,
    /// The shape `feedback.py --marks` reads (SPEC §5.1).
    pub approval: Option<ApprovalJson>,
    pub comments: Vec<CommentExport>,
    pub client_actions: Vec<ActionExport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommentExport {
    pub body: String,
    pub start_tick: String,
    pub end_tick: String,
    pub region: Option<Region>,
    pub resolved: bool,
    pub fixed_in: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionExport {
    pub seq: u64,
    pub command: String,
    pub params: Value,
    pub targets: Vec<String>,
    pub label: String,
    pub at: String,
    pub by: String,
    /// Touched clips' timeline durations in `base.fcproj`, in ticks.
    pub before: Vec<ClipLength>,
    /// The same clips in `head.fcproj` (`duration_ticks` is null for a clip no longer there).
    pub after: Vec<ClipLength>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipLength {
    pub clip: String,
    pub duration_ticks: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn version_feedback(
    version_id: &str,
    label: &str,
    status: &str,
    approval: Option<&Approval>,
    comments: &[Comment],
    events: &[ActionEvent],
    base: Option<&Value>,
    head: Option<&Value>,
) -> VersionFeedback {
    VersionFeedback {
        version_id: version_id.to_string(),
        version: label.to_string(),
        status: status.to_string(),
        approval: approval.map(|a| approval_json(label, a)),
        comments: comments
            .iter()
            .map(|c| CommentExport {
                body: c.body.clone(),
                start_tick: c.start_tick.0.to_string(),
                end_tick: c.end_tick.0.to_string(),
                region: c.region,
                resolved: c.resolved,
                fixed_in: c.fixed_in.clone(),
            })
            .collect(),
        client_actions: client_actions(events)
            .into_iter()
            .map(|e| ActionExport {
                seq: e.seq,
                command: e.id.clone(),
                params: e.params.clone(),
                targets: e.targets.clone(),
                label: e.label.clone(),
                at: e.at.clone(),
                by: e.actor.name.clone(),
                before: lengths(base, &e.targets),
                after: lengths(head, &e.targets),
            })
            .collect(),
    }
}

fn lengths(project: Option<&Value>, targets: &[String]) -> Vec<ClipLength> {
    targets.iter().map(|t| ClipLength { clip: t.clone(), duration_ticks: project.and_then(|p| clip_duration(p, t)) }).collect()
}

/// A timeline clip's duration: any object in the project with this `id` and a `start`,
/// `duration` and `source_in` (a track item).
fn clip_duration(value: &Value, id: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            let is_clip = map.contains_key("start") && map.contains_key("source_in") && map.get("id").is_some_and(|v| id_matches(v, id));
            if is_clip && let Some(d) = map.get("duration") {
                return Some(match d {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
            }
            map.values().find_map(|v| clip_duration(v, id))
        }
        Value::Array(items) => items.iter().find_map(|v| clip_duration(v, id)),
        _ => None,
    }
}

fn id_matches(value: &Value, id: &str) -> bool {
    match value {
        Value::Number(n) => n.to_string() == id,
        Value::String(s) => s == id,
        _ => false,
    }
}
