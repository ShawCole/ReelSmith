//! Portal metadata and the append-only event files. The event file is only appended to.
//! A cursor file records which step is current. Neither file is truncated.

use crate::error::Error;
use crate::history::History;
use crate::model::{EditEvent, MediaTick};
use crate::storage::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub admin_token: String,
    pub clients: Vec<ClientRecord>,
    pub projects: Vec<ProjectRecord>,
    pub broll: Vec<BrollRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientRecord {
    pub id: String,
    pub name: String,
    pub token: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub id: String,
    pub client_id: String,
    pub name: String,
    pub reviewer_token: String,
    pub agent_token: String,
    pub plan: crate::brief::BriefPlan,
    pub versions: Vec<VersionRecord>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionRecord {
    pub id: String,
    pub difference: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrollRecord {
    pub id: String,
    pub client_id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub duration: MediaTick,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CursorFile {
    state_of: Option<u64>,
    redo_of: Option<u64>,
}

pub fn load_saved(root: &Path) -> Result<Option<Saved>, Error> {
    let path = root.join("portal.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|_| Error::invalid("could not read the portal database"))?;
    serde_json::from_str(&text).map(Some).map_err(|_| Error::invalid("portal database is unreadable"))
}

pub fn save_saved(root: &Path, saved: &Saved) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(saved).map_err(|_| Error::invalid("could not save the portal database"))?;
    atomic_write(&root.join("portal.json"), &bytes)
}

pub fn load_history(root: &Path, fallback: &crate::model::Snapshot, project: &str, version: &str) -> Result<History, Error> {
    let base = read_base(root, project, version)?.unwrap_or_else(|| fallback.clone());
    let path = history_path(root, project, version);
    let mut events = Vec::new();
    if path.exists() {
        let file = fs::File::open(&path).map_err(|_| Error::invalid("could not read history"))?;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|_| Error::invalid("could not read history"))?;
            if line.trim().is_empty() {
                continue;
            }
            let event: EditEvent = serde_json::from_str(&line).map_err(|_| Error::invalid(format!("history line {} is unreadable", index + 1)))?;
            events.push(event);
        }
    }
    let mut history = History::new(base.clone());
    history.events = events;
    if let Some(cursor) = read_cursor(root, project, version)? {
        let known = |id: u64| history.events.iter().any(|event| event.id == id);
        history.state_of = cursor.state_of.filter(|id| known(*id));
        history.redo_of = cursor.redo_of.filter(|id| known(*id));
    } else if let Some(last) = history.events.last() {
        history.state_of = Some(last.id);
    }
    Ok(history)
}

pub fn save_history(root: &Path, project: &str, version: &str, history: &History) -> Result<(), Error> {
    let base_bytes = serde_json::to_vec(&history.base).map_err(|_| Error::invalid("could not record the base cut"))?;
    atomic_write(&base_path(root, project, version), &base_bytes)?;
    let path = history_path(root, project, version);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = if path.exists() { count_lines(&path)? } else { 0 };
    if existing > history.events.len() {
        return Err(Error::conflict("refusing to shorten the edit history"));
    }
    if existing < history.events.len() {
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        for event in history.events.iter().skip(existing) {
            let line = serde_json::to_string(event).map_err(|_| Error::invalid("could not record the edit"))?;
            writeln!(file, "{line}")?;
        }
        file.sync_all()?;
    }
    let cursor = CursorFile { state_of: history.state_of, redo_of: history.redo_of };
    let bytes = serde_json::to_vec(&cursor).map_err(|_| Error::invalid("could not record the history cursor"))?;
    atomic_write(&cursor_path(root, project, version), &bytes)
}

fn count_lines(path: &Path) -> Result<usize, Error> {
    let file = fs::File::open(path).map_err(|_| Error::invalid("could not read history"))?;
    let mut count = 0usize;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|_| Error::invalid("could not read history"))?;
        if !line.trim().is_empty() {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

fn read_cursor(root: &Path, project: &str, version: &str) -> Result<Option<CursorFile>, Error> {
    let path = cursor_path(root, project, version);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|_| Error::invalid("could not read the history cursor"))?;
    serde_json::from_str(&text).map(Some).map_err(|_| Error::invalid("history cursor is unreadable"))
}

fn read_base(root: &Path, project: &str, version: &str) -> Result<Option<crate::model::Snapshot>, Error> {
    let path = base_path(root, project, version);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|_| Error::invalid("could not read the base cut"))?;
    serde_json::from_str(&text).map(Some).map_err(|_| Error::invalid("base cut is unreadable"))
}

fn base_path(root: &Path, project: &str, version: &str) -> PathBuf {
    root.join("history").join(project).join(format!("{version}.base.json"))
}

fn history_path(root: &Path, project: &str, version: &str) -> PathBuf {
    root.join("history").join(project).join(format!("{version}.jsonl"))
}

fn cursor_path(root: &Path, project: &str, version: &str) -> PathBuf {
    root.join("history").join(project).join(format!("{version}.cursor.json"))
}
