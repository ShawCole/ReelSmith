//! The portal's operations: auth, projects, edits, uploads. HTTP is a thin wrapper.

use crate::assemble::initial_snapshot;
use crate::brief::{self, BriefPlan, CommentExport, EditExport, FeedbackFile, UploadBrief, VersionFeedback, approval_json};
use crate::error::Error;
use crate::history::{Commit, History, client_edits};
use crate::ids::{self, tokens_equal};
use crate::model::{Actor, Snapshot, Status};
use crate::ops::{self, EditBody, EditRequest, apply_request};
use crate::persist::{self, BrollRecord, ClientRecord, ProjectRecord, Saved, VersionRecord};
use crate::present::{self, CompareView, Head, VersionPage};
use crate::r2::{self, R2Config};
use crate::storage::{self, LocalStore, UploadManifest};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum StorageMode {
    Local,
    R2(R2Config),
}

#[derive(Clone, Debug)]
pub enum Role {
    Admin,
    Client { client_id: String, name: String },
    Reviewer { project_id: String, client_id: String, name: String },
    Agent { project_id: String },
}

pub struct Portal {
    root: PathBuf,
    admin_token: String,
    clients: Vec<ClientRecord>,
    projects: Vec<ProjectRecord>,
    broll: Vec<BrollRecord>,
    histories: HashMap<(String, String), History>,
    storage: LocalStore,
    mode: StorageMode,
    max_upload: u64,
}

#[derive(Serialize)]
pub struct Overview {
    pub clients: Vec<ClientCard>,
    pub projects: Vec<ProjectCard>,
    pub recent: Vec<present::EventView>,
}

#[derive(Serialize)]
pub struct ClientCard {
    pub id: String,
    pub name: String,
    pub token: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ProjectCard {
    pub id: String,
    pub client_name: String,
    pub name: String,
    pub final_type: String,
    pub reviewer_token: String,
    pub agent_token: String,
    pub versions: Vec<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct Home {
    pub client_name: String,
    pub projects: Vec<HomeProject>,
    pub broll: Vec<HomeBroll>,
}

#[derive(Serialize)]
pub struct HomeProject {
    pub id: String,
    pub name: String,
    pub final_type: String,
    pub reviewer_token: String,
    pub versions: Vec<String>,
}

#[derive(Serialize)]
pub struct HomeBroll {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub duration_ticks: String,
}

#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum UploadStart {
    Local { id: String, key: String },
    R2 { deferred: bool, message: String, part_urls: Vec<String> },
}

impl Portal {
    pub fn open(root: PathBuf, admin_from_env: Option<String>, mode: StorageMode, max_upload: u64) -> Result<Self, Error> {
        fs_ensure(&root)?;
        let storage = LocalStore::open(root.join("storage"))?;
        if persist::load_saved(&root)?.is_none() {
            crate::seed::write(&root, admin_from_env.clone())?;
        }
        let saved = persist::load_saved(&root)?.ok_or_else(|| Error::invalid("portal database was not created"))?;
        let admin_token = match admin_from_env {
            Some(token) => token,
            None => saved.admin_token.clone(),
        };
        let mut portal = Self {
            root,
            admin_token,
            clients: saved.clients,
            projects: saved.projects,
            broll: saved.broll,
            histories: HashMap::new(),
            storage,
            mode,
            max_upload,
        };
        portal.load_histories()?;
        portal.save_meta()?;
        Ok(portal)
    }

    pub fn admin_token(&self) -> &str {
        &self.admin_token
    }

    pub fn links(&self) -> Vec<(String, String)> {
        let mut rows = vec![("Admin".to_string(), format!("#/admin/{}", self.admin_token))];
        for client in &self.clients {
            rows.push((format!("Client {}", client.name), format!("#/c/{}", client.token)));
        }
        for project in &self.projects {
            rows.push((format!("Reviewer {}", project.name), format!("#/r/{}", project.reviewer_token)));
            rows.push((format!("Editor {}", project.name), format!("#/a/{}", project.agent_token)));
        }
        rows
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn authorize(&self, token: &str) -> Result<Role, Error> {
        if tokens_equal(token, &self.admin_token) {
            return Ok(Role::Admin);
        }
        if let Some(client) = self.clients.iter().find(|client| tokens_equal(token, &client.token)) {
            return Ok(Role::Client { client_id: client.id.clone(), name: client.name.clone() });
        }
        if let Some(project) = self.projects.iter().find(|project| tokens_equal(token, &project.reviewer_token)) {
            let name = self.client_name(&project.client_id).unwrap_or_else(|| "Client".to_string());
            return Ok(Role::Reviewer { project_id: project.id.clone(), client_id: project.client_id.clone(), name });
        }
        if let Some(project) = self.projects.iter().find(|project| tokens_equal(token, &project.agent_token)) {
            return Ok(Role::Agent { project_id: project.id.clone() });
        }
        Err(Error::forbidden("that link does not open a project"))
    }

    pub fn overview(&self, role: &Role) -> Result<Overview, Error> {
        self.require_admin(role)?;
        let mut recent = Vec::new();
        for project in &self.projects {
            for version in &project.versions {
                if let Some(history) = self.histories.get(&(project.id.clone(), version.id.clone())) {
                    let chain: Vec<u64> = history.chain().iter().map(|event| event.id).collect();
                    for event in history.events.iter().rev().take(8) {
                        recent.push(event_view(event, &chain));
                    }
                }
            }
        }
        recent.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
        recent.truncate(12);
        Ok(Overview {
            clients: self
                .clients
                .iter()
                .map(|client| ClientCard {
                    id: client.id.clone(),
                    name: client.name.clone(),
                    token: client.token.clone(),
                    created_at: client.created_at.clone(),
                })
                .collect(),
            projects: self
                .projects
                .iter()
                .map(|project| ProjectCard {
                    id: project.id.clone(),
                    client_name: self.client_name(&project.client_id).unwrap_or_else(|| "Client".to_string()),
                    name: project.name.clone(),
                    final_type: project.plan.final_type.as_str().to_string(),
                    reviewer_token: project.reviewer_token.clone(),
                    agent_token: project.agent_token.clone(),
                    versions: project.versions.iter().map(|version| version.difference.clone()).collect(),
                    created_at: project.created_at.clone(),
                })
                .collect(),
            recent,
        })
    }

    pub fn create_client(&mut self, role: &Role, name: &str) -> Result<ClientCard, Error> {
        self.require_admin(role)?;
        let name = crate::model::clean_name(name, "client name")?;
        let client = ClientRecord { id: ids::nid("c")?, name: name.clone(), token: ids::token()?, created_at: stamp() };
        let card = ClientCard { id: client.id.clone(), name, token: client.token.clone(), created_at: client.created_at.clone() };
        self.clients.push(client);
        self.save_meta()?;
        Ok(card)
    }

    pub fn home(&self, role: &Role) -> Result<Home, Error> {
        let client_id = self.client_scope(role)?;
        let client_name = self.client_name(&client_id).unwrap_or_else(|| "Client".to_string());
        let projects = self
            .projects
            .iter()
            .filter(|project| project.client_id == client_id)
            .map(|project| HomeProject {
                id: project.id.clone(),
                name: project.name.clone(),
                final_type: project.plan.final_type.as_str().to_string(),
                reviewer_token: project.reviewer_token.clone(),
                versions: project.versions.iter().map(|version| version.difference.clone()).collect(),
            })
            .collect();
        let broll = self
            .broll
            .iter()
            .filter(|item| item.client_id == client_id)
            .map(|item| HomeBroll {
                id: item.id.clone(),
                name: item.name.clone(),
                width: item.width,
                height: item.height,
                duration_ticks: item.duration.0.to_string(),
            })
            .collect();
        Ok(Home { client_name, projects, broll })
    }

    pub fn check_brief(&self, role: &Role, brief: &UploadBrief) -> Result<BriefPlan, Vec<crate::brief::FieldError>> {
        let client_id = match self.client_scope(role) {
            Ok(id) => id,
            Err(error) => return Err(vec![crate::brief::FieldError { field: "auth".to_string(), message: error.message }]),
        };
        let known = self.known_broll(&client_id);
        brief::validate_brief(brief, &known)
    }

    pub fn create_project(&mut self, role: &Role, brief: UploadBrief) -> Result<HomeProject, Error> {
        let client_id = self.client_scope(role)?;
        let known = self.known_broll(&client_id);
        let plan = brief::validate_brief(&brief, &known)
            .map_err(|errors| Error::invalid(errors.into_iter().map(|error| error.message).collect::<Vec<_>>().join("; ")))?;
        let library: Vec<BrollRecord> = self.broll.iter().filter(|item| item.client_id == client_id).cloned().collect();
        let snapshot = initial_snapshot(&plan, &library)?;
        let project_id = ids::nid("p")?;
        let mut versions = Vec::new();
        let mut histories = Vec::new();
        for planned in &plan.versions {
            let version_id = ids::nid("v")?;
            let mut history = History::new(blank_like(&snapshot)?);
            history.commit(Commit {
                actor: Actor::Agent,
                at: stamp(),
                action: crate::model::action::ASSEMBLE.to_string(),
                targets: snapshot.timeline.clips.iter().map(|clip| clip.id.clone()).collect(),
                summary: format!("Assembled {}", planned.difference),
                reason: None,
                after: snapshot.clone(),
                relates_to: None,
            })?;
            versions.push(VersionRecord { id: version_id.clone(), difference: planned.difference.clone() });
            histories.push((version_id, history));
        }
        let project = ProjectRecord {
            id: project_id.clone(),
            client_id,
            name: plan.project_name.clone(),
            reviewer_token: ids::token()?,
            agent_token: ids::token()?,
            plan,
            versions,
            created_at: stamp(),
        };
        let home = HomeProject {
            id: project.id.clone(),
            name: project.name.clone(),
            final_type: project.plan.final_type.as_str().to_string(),
            reviewer_token: project.reviewer_token.clone(),
            versions: project.versions.iter().map(|version| version.difference.clone()).collect(),
        };
        for (version_id, history) in &histories {
            persist::save_history(&self.root, &project.id, version_id, history)?;
            self.histories.insert((project.id.clone(), version_id.clone()), history.clone());
        }
        self.projects.push(project);
        self.save_meta()?;
        Ok(home)
    }

    pub fn workspace(&self, role: &Role, version_id: Option<&str>) -> Result<VersionPage, Error> {
        let project_id = self.project_scope(role)?.to_string();
        let version_id = self.resolve_version(&project_id, version_id)?;
        self.page(role, &project_id, &version_id)
    }

    pub fn head(&self, role: &Role, version_id: &str) -> Result<Head, Error> {
        let project_id = self.project_scope(role)?.to_string();
        let history = self.history(&project_id, version_id)?;
        present::head(history)
    }

    pub fn edit(&mut self, role: &Role, version_id: &str, body: EditBody) -> Result<VersionPage, Error> {
        let project_id = self.project_scope(role)?.to_string();
        let actor = self.actor(role, &project_id)?;
        let actor_name = actor.label();
        let at = stamp();
        let root = self.root.clone();
        {
            let history = self.histories.get_mut(&(project_id.clone(), version_id.to_string())).ok_or_else(|| Error::not_found("no version"))?;
            let status = history.current()?.status;
            guard(role, status, &body.edit)?;
            match &body.edit {
                EditRequest::Undo => {
                    history.undo(actor.clone(), at.clone(), body.reason.clone())?;
                }
                EditRequest::Redo => {
                    history.redo(actor.clone(), at.clone())?;
                }
                EditRequest::Restore { event_id } => {
                    history.restore(actor.clone(), at.clone(), *event_id, body.reason.clone())?;
                }
                other => {
                    let snapshot = history.current()?;
                    let applied = apply_request(&snapshot, &actor_name, &at, other)?;
                    history.commit(Commit {
                        actor,
                        at,
                        action: applied.action.to_string(),
                        targets: applied.targets,
                        summary: applied.summary,
                        reason: body.reason,
                        after: applied.after,
                        relates_to: None,
                    })?;
                }
            }
            persist::save_history(&root, &project_id, version_id, history)?;
        }
        self.write_feedback(&project_id, version_id)?;
        self.page(role, &project_id, version_id)
    }

    pub fn compare(&self, role: &Role, left: &str, right: &str) -> Result<CompareView, Error> {
        let project_id = self.project_scope(role)?.to_string();
        let left_diff = self.version_difference(&project_id, left)?;
        let right_diff = self.version_difference(&project_id, right)?;
        let left_snap = self.history(&project_id, left)?.current()?;
        let right_snap = self.history(&project_id, right)?.current()?;
        present::compare(&left_diff, &right_diff, &left_snap, &right_snap)
    }

    pub fn feedback(&self, role: &Role) -> Result<FeedbackFile, Error> {
        let project_id = self.project_scope(role)?.to_string();
        self.feedback_file(&project_id)
    }

    pub fn begin_upload(&self, role: &Role, filename: &str, media_role: &str, total_size: u64) -> Result<UploadStart, Error> {
        let client_id = self.client_scope(role)?;
        if media_role != "aroll" && media_role != "broll" {
            return Err(Error::invalid("role must be aroll or broll"));
        }
        let safe = storage::sanitize_filename(filename);
        match &self.mode {
            StorageMode::Local => {
                let id = ids::nid("up")?;
                let key = format!("c/{client_id}/{media_role}/{id}/{safe}");
                let manifest = self.storage.create_upload(&key, total_size, media_role, self.max_upload)?;
                Ok(UploadStart::Local { id: manifest.id, key: manifest.key })
            }
            StorageMode::R2(config) => {
                let part_size = 5 * 1024 * 1024;
                let count = r2::part_count(total_size.max(part_size), part_size)?;
                let key = format!("c/{client_id}/{media_role}/{safe}");
                let when = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
                let mut part_urls = Vec::new();
                let parts = count.min(8);
                for _ in 0..parts {
                    part_urls.push(r2::presign_put(config, &key, &when, 3600)?);
                }
                Ok(UploadStart::R2 {
                    deferred: true,
                    message: "R2 URLs are signed, and finishing a multipart upload against the bucket is not wired in this build. Run with PORTAL_STORAGE=local to store files on disk.".to_string(),
                    part_urls,
                })
            }
        }
    }

    pub fn write_part(&self, role: &Role, upload_id: &str, part: u32, bytes: &[u8]) -> Result<UploadManifest, Error> {
        let client_id = self.client_scope(role)?;
        let manifest = self.storage.status(upload_id)?;
        self.owns_key(&client_id, &manifest.key)?;
        self.storage.write_part(upload_id, part, bytes)
    }

    pub fn upload_status(&self, role: &Role, upload_id: &str) -> Result<UploadManifest, Error> {
        let client_id = self.client_scope(role)?;
        let manifest = self.storage.status(upload_id)?;
        self.owns_key(&client_id, &manifest.key)?;
        Ok(manifest)
    }

    pub fn complete_upload(&mut self, role: &Role, upload_id: &str, meta: CompleteMeta) -> Result<UploadManifest, Error> {
        let client_id = self.client_scope(role)?;
        let current = self.storage.status(upload_id)?;
        self.owns_key(&client_id, &current.key)?;
        let already = current.complete;
        let manifest = self.storage.complete(upload_id)?;
        if manifest.role == "broll" && !already {
            let name = crate::model::clean_name(&meta.name, "B-roll name")?;
            if meta.width == 0 || meta.height == 0 || meta.duration.0 <= 0 {
                return Err(Error::invalid("B-roll needs a picture size and a duration"));
            }
            self.broll.push(BrollRecord { id: ids::nid("b")?, client_id, name, width: meta.width, height: meta.height, duration: meta.duration });
            self.save_meta()?;
        }
        Ok(manifest)
    }

    pub fn read_object(&self, role: &Role, key: &str) -> Result<Vec<u8>, Error> {
        match role {
            Role::Reviewer { .. } | Role::Client { .. } => Err(Error::forbidden("reviewers watch the proxy timeline; raw uploads stay in storage")),
            Role::Agent { project_id } => {
                let project = self.project(project_id)?;
                self.owns_key(&project.client_id, key)?;
                self.storage.read(key)
            }
            Role::Admin => self.storage.read(key),
        }
    }

    fn page(&self, role: &Role, project_id: &str, version_id: &str) -> Result<VersionPage, Error> {
        let project_name = self.project(project_id)?.name.clone();
        let client_id = self.project(project_id)?.client_id.clone();
        let client_name = self.client_name(&client_id).unwrap_or_else(|| "Client".to_string());
        let plan = self.project(project_id)?.plan.clone();
        let version_rows: Vec<(String, String)> =
            self.project(project_id)?.versions.iter().map(|version| (version.id.clone(), version.difference.clone())).collect();
        let difference = version_rows.iter().find(|(id, _)| id == version_id).map(|(_, name)| name.clone()).unwrap_or_default();
        let mut tabs = Vec::new();
        for (id, name) in &version_rows {
            let current = self.histories.get(&(project_id.to_string(), id.clone())).and_then(|history| history.current().ok());
            let status = current.as_ref().map(|snap| snap.status).unwrap_or(Status::Draft);
            let mark = current.as_ref().and_then(|snap| snap.approval.as_ref().map(|approval| approval.mark.as_str().to_string()));
            tabs.push(present::VersionTab { id: id.clone(), difference: name.clone(), status, mark });
        }
        let history = self.history(project_id, version_id)?;
        let snapshot = history.current()?;
        let agent = matches!(role, Role::Agent { .. } | Role::Admin);
        let read_only = matches!(role, Role::Reviewer { .. } | Role::Client { .. }) && snapshot.status == Status::AgentWorking;
        Ok(VersionPage {
            project_id: project_id.to_string(),
            project_name,
            client_name,
            role: role_name(role).to_string(),
            read_only,
            read_only_reason: if read_only { "Your editor is working. This cut is read-only until it's marked ready.".to_string() } else { String::new() },
            brief: present::BriefSummary {
                final_type: plan.final_type.as_str().to_string(),
                target: plan.target.label(),
                aspect_label: plan.aspect.ratio.label(),
                aspect_source: aspect_source_name(plan.aspect.source).to_string(),
                use_broll: !matches!(plan.broll, brief::BrollChoice::None),
                flags: plan.flags.clone(),
            },
            versions: tabs,
            version: present::version_body(version_id, &difference, history, &plan, agent)?,
        })
    }

    fn feedback_file(&self, project_id: &str) -> Result<FeedbackFile, Error> {
        let project = self.project(project_id)?;
        let mut versions = Vec::new();
        for version in &project.versions {
            let history = self.history(project_id, &version.id)?;
            let snapshot = history.current()?;
            let comments = snapshot
                .comments
                .iter()
                .map(|comment| CommentExport {
                    body: comment.body.clone(),
                    start_ticks: comment.start.0.to_string(),
                    end_ticks: comment.end.0.to_string(),
                    region: comment.region,
                    resolved: comment.resolved,
                    fixed_in: comment.fixed_in.clone(),
                })
                .collect();
            let client_edits = client_edits(history)
                .iter()
                .map(|event| EditExport {
                    action: event.action.clone(),
                    targets: event.targets.clone(),
                    summary: event.summary.clone(),
                    at: event.at.clone(),
                    before_frames: frame_notes(&event.before, &event.targets),
                    after_frames: frame_notes(&event.after, &event.targets),
                })
                .collect();
            versions.push(VersionFeedback {
                approval: approval_json(&version.difference, &snapshot),
                difference: version.difference.clone(),
                comments,
                client_edits,
            });
        }
        Ok(FeedbackFile {
            schema: "reelsmith.portal.feedback/1".to_string(),
            client: self.client_name(&project.client_id).unwrap_or_else(|| "Client".to_string()),
            project: project.name.clone(),
            final_type: project.plan.final_type,
            versions,
        })
    }

    fn write_feedback(&self, project_id: &str, version_id: &str) -> Result<(), Error> {
        let file = self.feedback_file(project_id)?;
        let project = self.project(project_id)?;
        let difference = self.version_difference(project_id, version_id)?;
        let dir = self.root.join("feedback").join(project_id).join(version_id);
        std::fs::create_dir_all(&dir)?;
        let bytes = serde_json::to_vec_pretty(&file).map_err(|_| Error::invalid("could not write feedback"))?;
        storage::atomic_write(&dir.join("feedback.json"), &bytes)?;
        if let Some(version) = file.versions.iter().find(|version| version.difference == difference)
            && let Some(approval) = &version.approval
        {
            let bytes = serde_json::to_vec_pretty(approval).map_err(|_| Error::invalid("could not write approval"))?;
            storage::atomic_write(&dir.join("approval.json"), &bytes)?;
        }
        let _ = project;
        Ok(())
    }

    fn load_histories(&mut self) -> Result<(), Error> {
        let pairs: Vec<(String, String)> =
            self.projects.iter().flat_map(|project| project.versions.iter().map(|version| (project.id.clone(), version.id.clone()))).collect();
        for (project_id, version_id) in pairs {
            let fallback =
                blank_like(&Snapshot::new(crate::model::Timeline::blank(1920, 1080, crate::model::AspectSource::Raw)?, crate::model::FinalType::Reel))?;
            let history = persist::load_history(&self.root, &fallback, &project_id, &version_id)?;
            self.histories.insert((project_id, version_id), history);
        }
        Ok(())
    }

    fn save_meta(&self) -> Result<(), Error> {
        persist::save_saved(
            &self.root,
            &Saved { admin_token: self.admin_token.clone(), clients: self.clients.clone(), projects: self.projects.clone(), broll: self.broll.clone() },
        )
    }

    fn require_admin(&self, role: &Role) -> Result<(), Error> {
        if matches!(role, Role::Admin) { Ok(()) } else { Err(Error::forbidden("the dashboard is for the admin link")) }
    }

    fn client_scope(&self, role: &Role) -> Result<String, Error> {
        match role {
            Role::Admin => Err(Error::invalid("choose a client link for uploads and new projects")),
            Role::Client { client_id, .. } => Ok(client_id.clone()),
            Role::Reviewer { client_id, .. } => Ok(client_id.clone()),
            Role::Agent { project_id } => Ok(self.project(project_id)?.client_id.clone()),
        }
    }

    fn project_scope<'a>(&'a self, role: &'a Role) -> Result<&'a str, Error> {
        match role {
            Role::Reviewer { project_id, .. } | Role::Agent { project_id } => Ok(project_id),
            Role::Client { .. } | Role::Admin => Err(Error::invalid("open a reviewer or editor link for the timeline")),
        }
    }

    fn actor(&self, role: &Role, project_id: &str) -> Result<Actor, Error> {
        let project = self.project(project_id)?;
        match role {
            Role::Admin => Ok(Actor::Admin),
            Role::Agent { .. } => Ok(Actor::Agent),
            Role::Client { client_id, name } | Role::Reviewer { client_id, name, .. } => {
                if &project.client_id != client_id && !matches!(role, Role::Admin) {
                    return Err(Error::forbidden("that project belongs to another client"));
                }
                Ok(Actor::Client { client_id: client_id.clone(), name: name.clone() })
            }
        }
    }

    fn project(&self, id: &str) -> Result<&ProjectRecord, Error> {
        self.projects.iter().find(|project| project.id == id).ok_or_else(|| Error::not_found("no project"))
    }

    fn history(&self, project: &str, version: &str) -> Result<&History, Error> {
        self.histories.get(&(project.to_string(), version.to_string())).ok_or_else(|| Error::not_found("no version"))
    }

    fn resolve_version(&self, project_id: &str, version_id: Option<&str>) -> Result<String, Error> {
        let project = self.project(project_id)?;
        if let Some(id) = version_id {
            if project.versions.iter().any(|version| version.id == id) {
                return Ok(id.to_string());
            }
            return Err(Error::not_found("no version"));
        }
        project.versions.first().map(|version| version.id.clone()).ok_or_else(|| Error::not_found("this project has no versions"))
    }

    fn version_difference(&self, project_id: &str, version_id: &str) -> Result<String, Error> {
        self.project(project_id)?
            .versions
            .iter()
            .find(|version| version.id == version_id)
            .map(|version| version.difference.clone())
            .ok_or_else(|| Error::not_found("no version"))
    }

    fn client_name(&self, client_id: &str) -> Option<String> {
        self.clients.iter().find(|client| client.id == client_id).map(|client| client.name.clone())
    }

    fn known_broll(&self, client_id: &str) -> Vec<String> {
        self.broll.iter().filter(|item| item.client_id == client_id).map(|item| item.id.clone()).collect()
    }

    fn owns_key(&self, client_id: &str, key: &str) -> Result<(), Error> {
        let prefix = format!("c/{client_id}/");
        if key.starts_with(&prefix) { Ok(()) } else { Err(Error::forbidden("that file belongs to another client")) }
    }
}

#[derive(serde::Deserialize)]
pub struct CompleteMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub duration: crate::model::MediaTick,
}

fn guard(role: &Role, status: Status, edit: &EditRequest) -> Result<(), Error> {
    let reviewer = matches!(role, Role::Reviewer { .. } | Role::Client { .. });
    if reviewer && status == Status::AgentWorking {
        return Err(Error::locked("your editor is working — the timeline is read-only until it's marked ready"));
    }
    if reviewer && !ops::reviewer_may(edit) {
        return Err(Error::forbidden("that control is not on the review timeline"));
    }
    if matches!(role, Role::Agent { .. }) && ops::is_approval(edit) {
        return Err(Error::forbidden("approval is the client's mark"));
    }
    Ok(())
}

fn blank_like(snapshot: &Snapshot) -> Result<Snapshot, Error> {
    let mut blank =
        Snapshot::new(crate::model::Timeline::blank(snapshot.timeline.width, snapshot.timeline.height, snapshot.timeline.aspect_source)?, snapshot.final_type);
    blank.timeline.fps_num = snapshot.timeline.fps_num;
    blank.timeline.fps_den = snapshot.timeline.fps_den;
    Ok(blank)
}

fn stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Client { .. } => "client",
        Role::Reviewer { .. } => "reviewer",
        Role::Agent { .. } => "agent",
    }
}

fn aspect_source_name(source: crate::model::AspectSource) -> &'static str {
    match source {
        crate::model::AspectSource::Raw => "raw",
        crate::model::AspectSource::Profile => "profile",
        crate::model::AspectSource::ChangeFormat => "change_format",
    }
}

fn event_view(event: &crate::model::EditEvent, chain: &[u64]) -> present::EventView {
    present::EventView {
        id: event.id,
        at: event.at.clone(),
        actor: event.actor.label(),
        action: event.action.clone(),
        summary: event.summary.clone(),
        reason: event.reason.clone(),
        targets: event.targets.clone(),
        in_effect: chain.contains(&event.id),
    }
}

fn frame_notes(snapshot: &Snapshot, targets: &[String]) -> Vec<String> {
    let mut notes = Vec::new();
    for target in targets {
        if let Some(clip) = snapshot.timeline.clips.iter().find(|clip| &clip.id == target) {
            if let Ok(frames) = snapshot.timeline.tick_to_frame(clip.duration) {
                notes.push(format!("{} {frames}f", clip.name));
            }
        } else if let Some(caption) = snapshot.timeline.captions.iter().find(|caption| &caption.id == target) {
            notes.push(caption.text.clone());
        }
    }
    notes
}

fn fs_ensure(root: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(root).map_err(|_| Error::invalid("could not create the data directory"))
}
