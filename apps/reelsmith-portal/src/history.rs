//! The action log of one version: `actions.jsonl`, one [`ActionEvent`] per line.
//!
//! Every step anyone takes on a version is an engine command `(id, params)` that the server
//! replayed on its own session, or a portal action (`portal.comment`, `portal.mark`, …) that is
//! recorded but never replayed. The file is only ever appended to: [`write_log`] refuses to
//! write fewer lines than the file holds. `seq` is the server's revision of the version.
//!
//! Undo and redo are steps too. Each carries `reverts_seq`, the project-changing step it takes
//! back or re-applies, so the history panel and the feedback export know which steps are in
//! effect ([`effective_chain`]) without replaying an undo blindly.

use crate::error::Error;
use crate::model::{Actor, MAX_ACTIONS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub const UNDO: &str = "edit.undo";
pub const REDO: &str = "edit.redo";

/// One line of `actions.jsonl` (API.md "ActionEvent").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionEvent {
    pub seq: u64,
    /// Server time, RFC 3339 UTC.
    pub at: String,
    /// From the link token, never from the page.
    pub actor: Actor,
    /// `editor`, `reviewer` or `watcher`: the role the step ran under.
    pub role: String,
    /// The engine command id, or `portal.*` for a portal action.
    pub id: String,
    pub params: Value,
    pub label: String,
    /// Whether the step changed the project (false for selection and playhead steps), as the
    /// server's own replay saw it.
    pub project_changed: bool,
    pub based_on_seq: u64,
    pub reason: Option<String>,
    pub client_ts: Option<String>,
    /// For `edit.undo` / `edit.redo`: the step it takes back or re-applies.
    pub reverts_seq: Option<u64>,
    /// Clip, track, caption and marker ids named in `params` (PLAN §3).
    #[serde(default)]
    pub targets: Vec<String>,
}

impl ActionEvent {
    /// Portal actions are logged for the history panel and never replayed on the engine.
    pub fn is_portal(&self) -> bool {
        self.id.starts_with("portal.")
    }
}

/// Read a version's log. A missing file is an empty log.
pub fn read_log(path: &Path) -> Result<Vec<ActionEvent>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|_| Error::invalid("could not read the action log"))?;
    let mut events = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|_| Error::invalid("could not read the action log"))?;
        if line.trim().is_empty() {
            continue;
        }
        let event: ActionEvent = serde_json::from_str(&line).map_err(|_| Error::invalid(format!("action log line {} is unreadable", index + 1)))?;
        events.push(event);
    }
    Ok(events)
}

/// Make the file hold `events`: append the ones it does not have yet. Refuses (409) when the
/// file already holds more steps than `events`, so the log can never be shortened, and when
/// the steps would not be numbered 1, 2, 3, ….
pub fn write_log(path: &Path, events: &[ActionEvent]) -> Result<(), Error> {
    if events.len() > MAX_ACTIONS {
        return Err(Error::invalid("this version's history is full"));
    }
    if events.iter().enumerate().any(|(i, e)| e.seq != i as u64 + 1) {
        return Err(Error::invalid("action log steps must be numbered from 1 without gaps"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = if path.exists() { count_lines(path)? } else { 0 };
    if existing > events.len() {
        return Err(Error::conflict("refusing to shorten the action log"));
    }
    if existing < events.len() {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        for event in events.iter().skip(existing) {
            let line = serde_json::to_string(event).map_err(|_| Error::invalid("could not record the step"))?;
            writeln!(file, "{line}")?;
        }
        file.sync_all()?;
    }
    Ok(())
}

fn count_lines(path: &Path) -> Result<usize, Error> {
    let file = fs::File::open(path).map_err(|_| Error::invalid("could not read the action log"))?;
    let mut count = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|_| Error::invalid("could not read the action log"))?;
        if !line.trim().is_empty() {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

/// The engine's undo stacks as seen through the log: `done` holds the project-changing steps
/// in effect (oldest first), `undone` the ones an undo took back (most recent last).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stacks {
    pub done: Vec<u64>,
    pub undone: Vec<u64>,
}

impl Stacks {
    pub fn of(events: &[ActionEvent]) -> Self {
        let mut stacks = Self::default();
        for event in events {
            stacks.push(event);
        }
        stacks
    }

    /// Account for one more step.
    pub fn push(&mut self, event: &ActionEvent) {
        if event.is_portal() || !event.project_changed {
            return;
        }
        match (event.id.as_str(), event.reverts_seq) {
            (UNDO, Some(seq)) => {
                self.done.retain(|s| *s != seq);
                self.undone.push(seq);
            }
            (REDO, Some(seq)) => {
                self.undone.retain(|s| *s != seq);
                self.done.push(seq);
            }
            (UNDO | REDO, None) => {}
            _ => {
                self.done.push(event.seq);
                self.undone.clear();
            }
        }
    }

    /// What an `edit.undo` / `edit.redo` that changed the project reverts.
    pub fn reverts(&self, id: &str) -> Option<u64> {
        match id {
            UNDO => self.done.last().copied(),
            REDO => self.undone.last().copied(),
            _ => None,
        }
    }
}

/// The project-changing steps in effect now, oldest first.
pub fn effective_chain(events: &[ActionEvent]) -> Vec<&ActionEvent> {
    let stacks = Stacks::of(events);
    stacks.done.iter().filter_map(|seq| events.iter().find(|e| e.seq == *seq)).collect()
}

/// The clients' project-changing steps in effect now: the style evidence (PLAN §7).
pub fn client_actions(events: &[ActionEvent]) -> Vec<&ActionEvent> {
    effective_chain(events).into_iter().filter(|e| e.actor.is_client()).collect()
}

/// Ids a command's parameters name (clips, tracks, captions, markers), as strings.
pub fn targets_of(params: &Value) -> Vec<String> {
    const KEYS: &[&str] = &["clip", "clips", "item", "items", "track", "tracks", "caption", "captions", "marker", "markers"];
    let mut out = Vec::new();
    let Some(map) = params.as_object() else {
        return out;
    };
    for key in KEYS {
        let Some(value) = map.get(*key) else {
            continue;
        };
        let values: Vec<&Value> = match value {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        for v in values {
            let id = match v {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => continue,
            };
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}
