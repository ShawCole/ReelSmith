//! The server's own copy of each version's cut: a headless engine [`Session`] (engine +
//! format, no UI) that replays the action log. The server is the single writer of the cut
//! (PLAN §3): pages never upload a project, they post engine commands and this replays them.
//!
//! The session reads files only through [`PortalFs`], a path map from the virtual paths a
//! project names (`/files/<name>`, the page's paths) to stored objects, and writes nothing.
//! Auto-transcribe and other background analysis are off.

use crate::error::Error;
use reelsmith_engine::review::Role;
use reelsmith_engine::{FsServices, Services, Session};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The virtual path the server session opens its project from.
pub const PROJECT_PATH: &str = "/portal/project.fcproj";

/// File access for a server session: only mapped paths can be read, nothing can be written.
pub struct PortalFs {
    map: HashMap<String, PathBuf>,
}

impl PortalFs {
    pub fn new(map: HashMap<String, PathBuf>) -> Self {
        Self { map }
    }

    fn resolve(&self, path: &str) -> std::io::Result<String> {
        self.map
            .get(path)
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not a file of this version"))
    }
}

impl Services for PortalFs {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        FsServices.read_file(&self.resolve(path)?)
    }

    fn read_file_bounded(&self, path: &str, max: u64) -> std::io::Result<Vec<u8>> {
        FsServices.read_file_bounded(&self.resolve(path)?, max)
    }

    fn write_file(&self, _path: &str, _data: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "a portal session does not write files"))
    }

    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        FsServices.file_size(&self.resolve(path)?)
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        FsServices.read_range(&self.resolve(path)?, offset, len)
    }
}

/// What one replayed command did.
#[derive(Clone, Debug)]
pub struct Ran {
    pub label: String,
    pub project_changed: bool,
}

/// A live server session for one version.
pub struct Replayer {
    session: Session,
}

impl Replayer {
    /// Open `project` (a `.fcproj` on disk) with `media` mapping project paths to stored files.
    pub fn open(project: &Path, media: &[(String, PathBuf)]) -> Result<Self, Error> {
        let mut map: HashMap<String, PathBuf> = media.iter().cloned().collect();
        map.insert(PROJECT_PATH.to_string(), project.to_path_buf());
        let mut session = Session::new(Arc::new(PortalFs::new(map)));
        session.prefs.media_analysis.auto_transcribe = false;
        session.execute("file.open", json!({ "path": PROJECT_PATH })).map_err(|_| Error::invalid("the version's project file could not be opened"))?;
        session.enter_review(Role::Editor);
        session.take_executed();
        Ok(Self { session })
    }

    /// Run one top-level command under `role`'s gate. `Err` carries the engine's reason.
    pub fn run(&mut self, role: Role, id: &str, params: Value) -> Result<Ran, String> {
        self.session.enter_review(role);
        self.session.take_executed();
        let label = reelsmith_engine::commands::find(id).map(|spec| spec.label.to_string()).unwrap_or_else(|| id.to_string());
        let before = self.session.project.clone();
        self.session.execute(id, params).map_err(|e| e.to_string())?;
        let executed = self.session.take_executed();
        let project_changed = match executed.last() {
            Some(cmd) => cmd.project_changed,
            None => !Arc::ptr_eq(&before, &self.session.project),
        };
        Ok(Ran { label: executed.last().map(|c| c.label.clone()).unwrap_or(label), project_changed })
    }

    /// The current project as `.fcproj` bytes.
    pub fn project_bytes(&self) -> Vec<u8> {
        reelsmith_format::encode(&self.session.project, true)
    }

    /// The current project as JSON, for content comparisons.
    pub fn project_json(&self) -> Value {
        serde_json::to_value(&*self.session.project).unwrap_or(Value::Null)
    }
}

/// The engine's demo project (generator media, so no files are needed), as `.fcproj` bytes.
pub fn demo_project() -> Result<Vec<u8>, Error> {
    let mut session = Session::default();
    session.execute("file.openDemoProject", json!({})).map_err(|_| Error::invalid("could not make the demo project"))?;
    Ok(reelsmith_format::encode(&session.project, true))
}

/// An empty project named `name`, as `.fcproj` bytes: the base of a version nobody has cut yet.
pub fn empty_project(name: &str) -> Vec<u8> {
    let session = Session::default();
    let mut project = (*session.project).clone();
    project.name = name.to_string();
    reelsmith_format::encode(&project, true)
}

/// The project inside `.fcproj` bytes as JSON (`None` when they cannot be read).
pub fn project_content(bytes: &[u8]) -> Option<Value> {
    let loaded = reelsmith_format::decode(bytes).ok()?;
    serde_json::to_value(&loaded.project).ok()
}
