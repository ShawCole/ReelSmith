//! Portal metadata (`portal.json`) and where each version's files live under the data dir:
//!
//! ```text
//! projects/{project}/versions/{version}/
//!   base.fcproj            the cut as first handed to review (written once)
//!   head.fcproj            the server's latest replayed state (atomic write)
//!   actions.jsonl          append-only (history.rs)
//!   comments.json          threads (atomic write; every change is also an action)
//!   feedback/approval.json style-pipeline Gate B mark (SPEC §5.1)
//!   feedback/feedback.json reelsmith.portal.feedback/2
//! ```

use crate::error::Error;
use crate::model::{Approval, Comment, MediaTick, Status};
use crate::storage::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
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
    /// The full editor's link (the editing agent). PR #5 called it the agent token.
    #[serde(alias = "agent_token")]
    pub editor_token: String,
    /// Watch-only link, one per project.
    #[serde(default)]
    pub watcher_token: String,
    pub plan: crate::brief::BriefPlan,
    pub versions: Vec<VersionRecord>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionRecord {
    pub id: String,
    /// What makes this version different ("Primary reel", "Short hook", …).
    #[serde(alias = "difference")]
    pub label: String,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub approval: Option<Approval>,
    /// The media the project names, in manifest order (`/api/v/{version}/media/{n}`).
    #[serde(default)]
    pub media: Vec<MediaEntry>,
}

/// One media file a version's project names.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaEntry {
    /// The path inside the project (`/files/Ocean_Sunset.mp4`).
    pub path: String,
    /// The stored object (raw upload).
    pub key: String,
    /// Its reviewer proxy, once one is made.
    #[serde(default)]
    pub proxy_key: Option<String>,
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

/// The files of one version.
#[derive(Clone, Debug)]
pub struct VersionFiles {
    pub dir: PathBuf,
}

impl VersionFiles {
    pub fn new(root: &Path, project: &str, version: &str) -> Self {
        Self { dir: root.join("projects").join(project).join("versions").join(version) }
    }

    pub fn base(&self) -> PathBuf {
        self.dir.join("base.fcproj")
    }

    pub fn head(&self) -> PathBuf {
        self.dir.join("head.fcproj")
    }

    pub fn actions(&self) -> PathBuf {
        self.dir.join("actions.jsonl")
    }

    pub fn comments(&self) -> PathBuf {
        self.dir.join("comments.json")
    }

    pub fn approval(&self) -> PathBuf {
        self.dir.join("feedback").join("approval.json")
    }

    pub fn feedback(&self) -> PathBuf {
        self.dir.join("feedback").join("feedback.json")
    }

    /// Start a version: `base.fcproj` and `head.fcproj` from `project`, an empty log.
    /// The base is never replaced once written.
    pub fn create(&self, project: &[u8]) -> Result<(), Error> {
        fs::create_dir_all(self.dir.join("feedback"))?;
        if self.base().exists() {
            return Err(Error::conflict("this version already has a base cut"));
        }
        atomic_write(&self.base(), project)?;
        atomic_write(&self.head(), project)?;
        fs::OpenOptions::new().create(true).append(true).open(self.actions())?;
        write_comments(&self.comments(), &[])
    }
}

pub fn read_comments(path: &Path) -> Result<Vec<Comment>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|_| Error::invalid("could not read comments"))?;
    #[derive(Deserialize)]
    struct File {
        comments: Vec<Comment>,
    }
    serde_json::from_str::<File>(&text).map(|f| f.comments).map_err(|_| Error::invalid("comments are unreadable"))
}

pub fn write_comments(path: &Path, comments: &[Comment]) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "comments": comments })).map_err(|_| Error::invalid("could not save comments"))?;
    atomic_write(path, &bytes)
}
