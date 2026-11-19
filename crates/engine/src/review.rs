//! Review links (`apps/reelsmith-portal`, docs/portal/DESIGN.md): which top-level commands each
//! link role may run, and which journaled commands a server can replay from a log.
//!
//! The portal records every top-level command a page runs ([`crate::ExecutedCommand`]) and
//! replays it on its own [`Session`] to keep the one copy of the cut. A command is only accepted
//! into that log when replaying it gives the same project: it reads its targets from its
//! parameters or from selection/playhead steps that are in the log too. Commands that read the
//! disk, analyse media or start background jobs are never run inside a portal session.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::find;
use crate::{CommandPolicy, Session};

/// What a link lets its holder do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The full editor (the editing agent, or a person at the editor's desk).
    Editor,
    /// A client's "ready for review" link: timeline edits, comments, no library or tools.
    Reviewer,
    /// Watching someone else edit: playback and navigation only.
    Watcher,
}

/// Edits a reviewer may make on the timeline. Everything else that changes the project is
/// the editor's.
pub const REVIEWER_EDITS: &[&str] = &[
    "timeline.select",
    "timeline.selectClipAtPlayhead",
    "timeline.selectNextClip",
    "timeline.selectPrevClip",
    "timeline.selectGap",
    "timeline.trim",
    "timeline.razor",
    "timeline.move",
    "timeline.delete",
    "timeline.nudgeLeft",
    "timeline.nudgeRight",
    "timeline.toggleMuteTargetedAudio",
    "timeline.toggleSoloTargetedAudio",
    "sequence.addEdit",
    "sequence.addEditAllTracks",
    "sequence.lift",
    "sequence.extract",
    "sequence.snap",
    "sequence.closeGap",
    "edit.undo",
    "edit.redo",
    "edit.clear",
    "edit.rippleDelete",
    "edit.selectAll",
    "edit.deselectAll",
    "clip.audioGain",
    "clip.volumeUp",
    "clip.volumeDown",
    "clip.enable",
    "captions.select",
    "captions.setText",
    "captions.move",
    "captions.trim",
    "captions.split",
    "captions.merge",
    "markers.add",
    "markers.addRange",
    "markers.edit",
    "markers.delete",
    "markers.markIn",
    "markers.markOut",
    "markers.clearIn",
    "markers.clearOut",
    "markers.clearInOut",
];

/// Prefixes of commands that are never run inside a portal session: they read or write the
/// disk, import, export or render, analyse media, or start background jobs. The portal
/// server runs the few it needs itself (import of an upload, proxies) and logs the result.
const NOT_IN_PORTAL_PREFIXES: &[&str] = &[
    "file.",
    "proxies.",
    "jobs.",
    "prefs.",
    "settings.",
    "speech.",
    "transcript.",
    "tts.",
    "voiceover.",
    "mediaBrowser.",
    "projectManager.",
    "export.",
    "media.",
    "mediaCache.",
    "shortcuts.",
    "presets.",
    "events.",
];

/// Words that mark a disk, render or analysis command in any namespace.
const NOT_IN_PORTAL_WORDS: &[&str] = &["export", "Export", "render", "Render", "transcribe", "Transcribe", "SceneEdit", "import", "Import"];

/// Namespaces whose journaled commands replay to the same project from the log. A new
/// namespace must be added here or to the refused lists (`review_tests` fails until it is).
const REPLAY_SAFE_NAMESPACES: &[&str] = &[
    "timeline", "sequence", "clip", "edit", "trim", "playhead", "markers", "captions", "effects", "colors", "graphics", "project",
    "history", "source", "multicam", "masks", "mixer", "keyframes", "transitions", "text", "textPanel", "tool", "panel", "audio",
    "titles", "essentialSound", "remix", "color", "lumetri", "metadata", "labels", "bins", "sync", "speed", "view", "clipMixer", "lut",
];

/// Whether `id` is refused in every portal session.
pub fn not_in_portal(id: &str) -> bool {
    NOT_IN_PORTAL_PREFIXES.iter().any(|p| id.starts_with(p)) || NOT_IN_PORTAL_WORDS.iter().any(|w| id.contains(w))
}

/// Whether a journaled `id` can be accepted into a portal log and replayed.
pub fn replay_safe(id: &str) -> bool {
    let namespace = id.split('.').next().unwrap_or(id);
    !not_in_portal(id) && REPLAY_SAFE_NAMESPACES.contains(&namespace)
}

/// Whether `role` may run `id` as a top-level command. `Err` says why not, in words a client
/// can read.
pub fn allows(role: Role, id: &str) -> Result<(), String> {
    let Some(spec) = find(id) else {
        return Err("unknown command".to_string());
    };
    if not_in_portal(id) {
        return Err("not available on a review link".to_string());
    }
    let navigation = id.starts_with("playhead.") || id.starts_with("playback.");
    let allowed = match role {
        Role::Editor => !spec.journal || replay_safe(id),
        Role::Reviewer => !spec.journal || navigation || id.starts_with("trim.") || REVIEWER_EDITS.contains(&id),
        Role::Watcher => !spec.journal || navigation,
    };
    if allowed {
        Ok(())
    } else if role == Role::Watcher {
        Err("this link can watch but not edit".to_string())
    } else {
        Err("not available on a review link".to_string())
    }
}

/// The policy a review page installs on its session ([`Session::policy`]).
pub fn policy(role: Role) -> CommandPolicy {
    Arc::new(move |id: &str, _params: &Value| allows(role, id))
}

impl Session {
    /// Lock this session to a review link's role and start collecting executed commands.
    /// Call after the project is open (opening a file is not a portal command).
    pub fn enter_review(&mut self, role: Role) {
        self.policy = Some(policy(role));
        self.outbox.get_or_insert_with(Vec::new);
    }

    /// The commands executed since the last call, oldest first.
    pub fn take_executed(&mut self) -> Vec<crate::ExecutedCommand> {
        self.outbox.as_mut().map(std::mem::take).unwrap_or_default()
    }
}
