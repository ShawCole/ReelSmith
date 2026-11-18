//! Append-only edit log.
//!
//! Events are never removed. `state_of` is a cursor: `None` means the base snapshot, and
//! `Some(id)` means that event's `after` snapshot. Undo, redo and restore append a record and
//! move the cursor. An agent edit that changes something a client last touched on the current
//! chain needs a reason, so a client trim is never replaced silently.

use crate::error::Error;
use crate::model::{Actor, EditEvent, MAX_EVENTS, MediaTick, Snapshot, is_picture_edit};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct History {
    pub base: Snapshot,
    pub events: Vec<EditEvent>,
    pub state_of: Option<u64>,
    pub redo_of: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub actor: Actor,
    pub at: String,
    pub action: String,
    pub targets: Vec<String>,
    pub summary: String,
    pub reason: Option<String>,
    pub after: Snapshot,
    pub relates_to: Option<u64>,
}

impl History {
    pub fn new(base: Snapshot) -> Self {
        Self { base, events: Vec::new(), state_of: None, redo_of: None }
    }

    pub fn current(&self) -> Result<Snapshot, Error> {
        self.snapshot_of(self.state_of)
    }

    pub fn can_undo(&self) -> bool {
        self.state_of.is_some()
    }

    pub fn can_redo(&self) -> bool {
        self.redo_of.is_some()
    }

    pub fn event(&self, id: u64) -> Result<&EditEvent, Error> {
        self.events.iter().find(|event| event.id == id).ok_or_else(|| Error::not_found(format!("no event {id}")))
    }

    pub fn snapshot_of(&self, id: Option<u64>) -> Result<Snapshot, Error> {
        match id {
            None => Ok(self.base.clone()),
            Some(id) => Ok(self.event(id)?.after.clone()),
        }
    }

    pub fn commit(&mut self, commit: Commit) -> Result<&EditEvent, Error> {
        if self.events.len() >= MAX_EVENTS {
            return Err(Error::invalid("this version's history is full"));
        }
        self.ensure_agent_reason(&commit.actor, &commit.reason, &commit.after, is_picture_edit(&commit.action))?;
        let before = self.current()?;
        let id = self.next_id();
        let event = EditEvent {
            id,
            at: commit.at,
            actor: commit.actor,
            action: commit.action,
            targets: commit.targets,
            summary: commit.summary,
            reason: nonempty(commit.reason),
            based_on: self.state_of,
            relates_to: commit.relates_to,
            before,
            after: commit.after,
        };
        self.events.push(event);
        self.state_of = Some(id);
        self.redo_of = None;
        self.events.last().ok_or_else(|| Error::invalid("history append failed"))
    }

    pub fn undo(&mut self, actor: Actor, at: String, reason: Option<String>) -> Result<&EditEvent, Error> {
        let current_id = self.state_of.ok_or_else(|| Error::invalid("nothing to undo"))?;
        let previous = self.event(current_id)?.based_on;
        let after = self.snapshot_of(previous)?;
        self.ensure_agent_reason(&actor, &reason, &after, true)?;
        if self.events.len() >= MAX_EVENTS {
            return Err(Error::invalid("this version's history is full"));
        }
        let before = self.current()?;
        let summary = format!("Undid step {current_id}");
        let id = self.next_id();
        self.events.push(EditEvent {
            id,
            at,
            actor,
            action: crate::model::action::UNDO.to_string(),
            targets: vec![current_id.to_string()],
            summary,
            reason: nonempty(reason),
            based_on: self.state_of,
            relates_to: Some(current_id),
            before,
            after,
        });
        self.redo_of = Some(current_id);
        self.state_of = previous;
        self.events.last().ok_or_else(|| Error::invalid("history append failed"))
    }

    pub fn redo(&mut self, actor: Actor, at: String) -> Result<&EditEvent, Error> {
        let target = self.redo_of.ok_or_else(|| Error::invalid("nothing to redo"))?;
        let after = self.snapshot_of(Some(target))?;
        if self.events.len() >= MAX_EVENTS {
            return Err(Error::invalid("this version's history is full"));
        }
        let before = self.current()?;
        let id = self.next_id();
        self.events.push(EditEvent {
            id,
            at,
            actor,
            action: crate::model::action::REDO.to_string(),
            targets: vec![target.to_string()],
            summary: format!("Redid step {target}"),
            reason: None,
            based_on: self.state_of,
            relates_to: Some(target),
            before,
            after,
        });
        self.state_of = Some(target);
        self.redo_of = None;
        self.events.last().ok_or_else(|| Error::invalid("history append failed"))
    }

    /// Restore the snapshot as of `event_id`. `None` restores the base. The restored steps stay
    /// in the log, and this restore is itself a step that can be undone.
    pub fn restore(&mut self, actor: Actor, at: String, event_id: Option<u64>, reason: Option<String>) -> Result<&EditEvent, Error> {
        if let Some(id) = event_id {
            let _ = self.event(id)?;
        }
        let after = self.snapshot_of(event_id)?;
        self.ensure_agent_reason(&actor, &reason, &after, true)?;
        if self.events.len() >= MAX_EVENTS {
            return Err(Error::invalid("this version's history is full"));
        }
        let before = self.current()?;
        let label = event_id.map(|id| id.to_string()).unwrap_or_else(|| "the original assembly".to_string());
        let id = self.next_id();
        self.events.push(EditEvent {
            id,
            at,
            actor,
            action: crate::model::action::RESTORE.to_string(),
            targets: vec![label.clone()],
            summary: format!("Restored {label}"),
            reason: nonempty(reason),
            based_on: self.state_of,
            relates_to: event_id,
            before,
            after,
        });
        self.state_of = Some(id);
        self.redo_of = None;
        self.events.last().ok_or_else(|| Error::invalid("history append failed"))
    }

    /// Events in effect, oldest first. A restore jumps to the step it restored, so an undone
    /// branch stays in `events` without counting as the current chain. Cycles are cut off.
    pub fn chain(&self) -> Vec<&EditEvent> {
        let mut cursor = self.state_of;
        let mut out = Vec::new();
        let mut seen = Vec::new();
        while let Some(id) = cursor {
            if seen.contains(&id) || seen.len() >= MAX_EVENTS {
                break;
            }
            seen.push(id);
            let Some(event) = self.events.iter().find(|event| event.id == id) else { break };
            let next = if event.action == crate::model::action::RESTORE { event.relates_to } else { event.based_on };
            out.push(event);
            cursor = next;
        }
        out.reverse();
        out
    }

    fn next_id(&self) -> u64 {
        self.events.iter().map(|event| event.id).max().unwrap_or(0).saturating_add(1)
    }

    fn ensure_agent_reason(&self, actor: &Actor, reason: &Option<String>, after: &Snapshot, check: bool) -> Result<(), Error> {
        if !check || !actor.is_agent() {
            return Ok(());
        }
        let before = self.current()?;
        let changed = changed_targets(&before, after);
        let needs_reason = changed.iter().any(|target| self.last_touch(target).is_some_and(|event| event.actor.is_client()));
        if needs_reason && nonempty(reason.clone()).is_none() {
            return Err(Error::invalid("the agent needs a reason before it changes a client's edit — nothing was overwritten"));
        }
        Ok(())
    }

    fn last_touch(&self, target: &str) -> Option<&EditEvent> {
        self.chain().into_iter().rev().find(|event| is_picture_edit(&event.action) && event.targets.iter().any(|item| item == target))
    }
}

fn nonempty(reason: Option<String>) -> Option<String> {
    reason.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    })
}

pub fn changed_targets(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    let mut ids = Vec::new();
    for clip in &before.timeline.clips {
        match after.timeline.clips.iter().find(|other| other.id == clip.id) {
            Some(other) if other != clip => ids.push(clip.id.clone()),
            None => ids.push(clip.id.clone()),
            Some(_) => {}
        }
    }
    for clip in &after.timeline.clips {
        if before.timeline.clips.iter().all(|other| other.id != clip.id) {
            ids.push(clip.id.clone());
        }
    }
    for caption in &before.timeline.captions {
        match after.timeline.captions.iter().find(|other| other.id == caption.id) {
            Some(other) if other != caption => ids.push(caption.id.clone()),
            None => ids.push(caption.id.clone()),
            Some(_) => {}
        }
    }
    for caption in &after.timeline.captions {
        if before.timeline.captions.iter().all(|other| other.id != caption.id) {
            ids.push(caption.id.clone());
        }
    }
    if before.timeline.width != after.timeline.width
        || before.timeline.height != after.timeline.height
        || before.timeline.aspect_source != after.timeline.aspect_source
    {
        ids.push("sequence".to_string());
    }
    ids.sort();
    ids.dedup();
    ids
}

pub fn client_edits(history: &History) -> Vec<&EditEvent> {
    history.chain().into_iter().filter(|event| event.actor.is_client() && is_picture_edit(&event.action)).collect()
}

/// Duration of the current cut, used by length checks. Not a calendar conversion.
pub fn current_duration(history: &History) -> Result<MediaTick, Error> {
    Ok(history.current()?.timeline.duration())
}
