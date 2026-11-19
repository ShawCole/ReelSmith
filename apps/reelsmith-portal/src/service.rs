//! The portal's operations: links, boot, the action log and server replay, comments, marks,
//! uploads. HTTP (`http.rs`) is a thin wrapper. The contract is API.md beside PLAN.md.

use crate::brief::{self, BriefPlan, LengthTarget, RawFile, UploadBrief};
use crate::error::Error;
use crate::feedback::{self, FeedbackFile, VersionFeedback};
use crate::history::{ActionEvent, Stacks, effective_chain, read_log, targets_of, write_log};
use crate::ids::{self, tokens_equal};
use crate::model::{Actor, Approval, Comment, FinalType, MAX_COMMENTS, MAX_TEXT, Mark, MediaRole, MediaTick, Region, Reply, Status, clean_text};
use crate::persist::{self, BrollRecord, ClientRecord, MediaEntry, ProjectRecord, Saved, VersionFiles, VersionRecord};
use crate::r2::{self, R2Config};
use crate::replay::{self, Replayer};
use crate::storage::{self, LocalStore, UploadManifest};
use reelsmith_engine::review::{self, Role as EngineRole};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Most commands one POST may carry.
pub const MAX_BATCH: usize = 1_000;

#[derive(Clone, Debug)]
pub enum StorageMode {
    Local,
    R2(R2Config),
}

/// What a link token opens (API.md "Token kinds").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Client { client_id: String, name: String },
    Reviewer { project_id: String, client_id: String, name: String },
    Editor { project_id: String },
    Watcher { project_id: String },
}

impl Role {
    fn project_id(&self) -> Option<&str> {
        match self {
            Self::Reviewer { project_id, .. } | Self::Editor { project_id } | Self::Watcher { project_id } => Some(project_id),
            Self::Admin | Self::Client { .. } => None,
        }
    }

    fn engine_role(&self) -> Result<EngineRole, Error> {
        match self {
            Self::Editor { .. } => Ok(EngineRole::Editor),
            Self::Reviewer { .. } => Ok(EngineRole::Reviewer),
            Self::Watcher { .. } => Ok(EngineRole::Watcher),
            Self::Admin | Self::Client { .. } => Err(Error::forbidden("open an editor, review or watch link for the timeline")),
        }
    }

    fn actor(&self) -> Actor {
        match self {
            Self::Admin => Actor { kind: crate::model::ActorKind::Admin, id: "admin".to_string(), name: "Admin".to_string() },
            Self::Client { client_id, name } | Self::Reviewer { client_id, name, .. } => Actor::client(client_id, name),
            Self::Editor { .. } => Actor::agent(),
            Self::Watcher { .. } => Actor::watcher(),
        }
    }
}

fn role_name(role: EngineRole) -> &'static str {
    match role {
        EngineRole::Editor => "editor",
        EngineRole::Reviewer => "reviewer",
        EngineRole::Watcher => "watcher",
    }
}

fn parse_role(name: &str) -> EngineRole {
    match name {
        "reviewer" => EngineRole::Reviewer,
        "watcher" => EngineRole::Watcher,
        _ => EngineRole::Editor,
    }
}

/// A version in memory: its log, and (once someone edits) its server session.
struct Live {
    events: Vec<ActionEvent>,
    session: Option<Replayer>,
}

pub struct Portal {
    root: PathBuf,
    admin_token: String,
    clients: Vec<ClientRecord>,
    projects: Vec<ProjectRecord>,
    broll: Vec<BrollRecord>,
    live: HashMap<String, Live>,
    storage: LocalStore,
    mode: StorageMode,
    max_upload: u64,
}

// ----- shapes (API.md) -----

#[derive(Serialize)]
pub struct Boot {
    pub role: &'static str,
    pub actor: Actor,
    pub project: BootProject,
    pub version: BootVersion,
    pub versions: Vec<VersionLink>,
    pub seq: u64,
    pub project_url: String,
    pub media: Vec<BootMedia>,
}

#[derive(Serialize)]
pub struct BootProject {
    pub id: String,
    pub name: String,
    pub final_type: FinalType,
}

#[derive(Serialize)]
pub struct BootVersion {
    pub id: String,
    pub label: String,
    pub status: Status,
}

#[derive(Serialize)]
pub struct VersionLink {
    pub id: String,
    pub label: String,
}

#[derive(Serialize)]
pub struct BootMedia {
    pub path: String,
    pub url: String,
}

#[derive(Serialize)]
pub struct HeadInfo {
    pub seq: u64,
    pub status: Status,
}

#[derive(Serialize)]
pub struct ActionsSince {
    pub seq: u64,
    pub actions: Vec<ActionEvent>,
}

#[derive(Deserialize)]
pub struct ActionsIn {
    pub based_on_seq: u64,
    pub actions: Vec<ActionIn>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct ActionIn {
    pub id: String,
    #[serde(default = "empty_params")]
    pub params: Value,
    #[serde(default)]
    pub label: Option<String>,
    /// The page's view; the server records what its own replay saw instead.
    #[serde(default)]
    pub project_changed: Option<bool>,
    #[serde(default)]
    pub client_ts: Option<String>,
}

fn empty_params() -> Value {
    json!({})
}

#[derive(Serialize)]
pub struct ActionsOut {
    pub seq: u64,
    pub accepted: usize,
}

#[derive(Deserialize)]
pub struct CommentIn {
    pub start_tick: MediaTick,
    pub end_tick: MediaTick,
    #[serde(default)]
    pub region: Option<Region>,
    pub body: String,
}

#[derive(Deserialize)]
pub struct ReplyIn {
    pub body: String,
}

#[derive(Deserialize)]
pub struct ResolveIn {
    pub resolved: bool,
    #[serde(default)]
    pub fixed_in: Option<String>,
}

#[derive(Deserialize)]
pub struct MarkIn {
    pub mark: Mark,
    #[serde(default)]
    pub note: String,
}

#[derive(Deserialize)]
pub struct StatusIn {
    pub status: Status,
}

#[derive(Serialize)]
pub struct Overview {
    pub clients: Vec<ClientCard>,
    pub projects: Vec<ProjectCard>,
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
    pub editor_token: String,
    pub reviewer_token: String,
    pub watcher_token: String,
    pub versions: Vec<VersionCard>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct VersionCard {
    pub id: String,
    pub label: String,
    pub status: Status,
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
    pub versions: Vec<VersionCard>,
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

#[derive(Deserialize)]
pub struct CompleteMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub duration: MediaTick,
}

impl Portal {
    pub fn open(root: PathBuf, admin_from_env: Option<String>, mode: StorageMode, max_upload: u64) -> Result<Self, Error> {
        std::fs::create_dir_all(&root).map_err(|_| Error::invalid("could not create the data directory"))?;
        let storage = LocalStore::open(root.join("storage"))?;
        let fresh = persist::load_saved(&root)?.is_none();
        let saved = match persist::load_saved(&root)? {
            Some(saved) => saved,
            None => Saved { admin_token: ids::token()?, clients: Vec::new(), projects: Vec::new(), broll: Vec::new() },
        };
        let admin_token = admin_from_env.unwrap_or_else(|| saved.admin_token.clone());
        let mut portal = Self {
            root,
            admin_token,
            clients: saved.clients,
            projects: saved.projects,
            broll: saved.broll,
            live: HashMap::new(),
            storage,
            mode,
            max_upload,
        };
        for project in &mut portal.projects {
            if project.watcher_token.is_empty() {
                project.watcher_token = ids::token()?;
            }
        }
        if fresh {
            portal.seed()?;
        }
        portal.save_meta()?;
        Ok(portal)
    }

    /// API.md "Seed": one client, Harbor & Co, and one project whose version base is the
    /// engine's demo project.
    fn seed(&mut self) -> Result<(), Error> {
        let client = ClientRecord { id: ids::nid("c")?, name: "Harbor & Co".to_string(), token: ids::token()?, created_at: stamp() };
        let brief = UploadBrief {
            project_name: "Spring launch reel".to_string(),
            final_type: FinalType::Reel,
            target: Some(LengthTarget::Unspecified),
            end_card: None,
            version_count: 1,
            differences: vec![],
            use_broll: false,
            broll_library_ids: vec![],
            broll_uploads: vec![],
            raw_files: vec![RawFile {
                name: "Demo footage".to_string(),
                width: 1920,
                height: 1080,
                duration: MediaTick(30 * reelsmith_time::TICKS_PER_SECOND),
                role: MediaRole::Aroll,
            }],
            change_format: false,
            explicit_width: None,
            explicit_height: None,
            profile_width: None,
            profile_height: None,
        };
        let plan = brief::validate_brief(&brief, &[]).map_err(|_| Error::invalid("the seed brief is invalid"))?;
        let project_id = ids::nid("p")?;
        let version_id = ids::nid("v")?;
        let label = plan.versions.first().map(|v| v.difference.clone()).unwrap_or_else(|| "Primary reel".to_string());
        VersionFiles::new(&self.root, &project_id, &version_id).create(&replay::demo_project()?)?;
        self.projects.push(ProjectRecord {
            id: project_id,
            client_id: client.id.clone(),
            name: plan.project_name.clone(),
            reviewer_token: ids::token()?,
            editor_token: ids::token()?,
            watcher_token: ids::token()?,
            plan,
            versions: vec![VersionRecord { id: version_id, label, status: Status::Ready, approval: None, media: Vec::new() }],
            created_at: stamp(),
        });
        self.clients.push(client);
        Ok(())
    }

    pub fn admin_token(&self) -> &str {
        &self.admin_token
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The links to print at start-up: (label, path with the token in the fragment).
    pub fn links(&self) -> Vec<(String, String)> {
        let mut rows = vec![("Admin".to_string(), format!("/#/admin/{}", self.admin_token))];
        for client in &self.clients {
            rows.push((format!("Client {}", client.name), format!("/#/c/{}", client.token)));
        }
        for project in &self.projects {
            rows.push((format!("Editor {}", project.name), format!("/edit/#{}", project.editor_token)));
            rows.push((format!("Review {}", project.name), format!("/review/#{}", project.reviewer_token)));
            rows.push((format!("Watch {}", project.name), format!("/watch/#{}", project.watcher_token)));
        }
        rows
    }

    pub fn authorize(&self, token: &str) -> Result<Role, Error> {
        if tokens_equal(token, &self.admin_token) {
            return Ok(Role::Admin);
        }
        if let Some(client) = self.clients.iter().find(|client| tokens_equal(token, &client.token)) {
            return Ok(Role::Client { client_id: client.id.clone(), name: client.name.clone() });
        }
        for project in &self.projects {
            if tokens_equal(token, &project.reviewer_token) {
                let name = self.client_name(&project.client_id).unwrap_or_else(|| "Client".to_string());
                return Ok(Role::Reviewer { project_id: project.id.clone(), client_id: project.client_id.clone(), name });
            }
            if tokens_equal(token, &project.editor_token) {
                return Ok(Role::Editor { project_id: project.id.clone() });
            }
            if tokens_equal(token, &project.watcher_token) {
                return Ok(Role::Watcher { project_id: project.id.clone() });
            }
        }
        Err(Error::forbidden("that link does not open a project"))
    }

    // ----- boot and version reads -----

    pub fn boot(&self, role: &Role, version: Option<&str>) -> Result<Boot, Error> {
        let engine_role = role.engine_role()?;
        let project_id = role.project_id().ok_or_else(|| Error::forbidden("open an editor, review or watch link"))?;
        let project = self.project(project_id)?;
        let version = match version {
            Some(id) => project.versions.iter().find(|v| v.id == id).ok_or_else(|| Error::forbidden("that version is not part of this link's project"))?,
            None => project.versions.first().ok_or_else(|| Error::not_found("this project has no versions"))?,
        };
        let shown = if engine_role == EngineRole::Reviewer && version.status == Status::AgentWorking { EngineRole::Watcher } else { engine_role };
        Ok(Boot {
            role: role_name(shown),
            actor: role.actor(),
            project: BootProject { id: project.id.clone(), name: project.name.clone(), final_type: project.plan.final_type },
            version: BootVersion { id: version.id.clone(), label: version.label.clone(), status: version.status },
            versions: project.versions.iter().map(|v| VersionLink { id: v.id.clone(), label: v.label.clone() }).collect(),
            seq: self.seq_of(&project.id, &version.id)?,
            project_url: format!("/api/v/{}/project", version.id),
            media: version.media.iter().enumerate().map(|(n, m)| BootMedia { path: m.path.clone(), url: format!("/api/v/{}/media/{n}", version.id) }).collect(),
        })
    }

    /// The head `.fcproj` bytes and the seq they are at.
    pub fn project_file(&self, role: &Role, version_id: &str) -> Result<(Vec<u8>, u64), Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let (project_id, version_id) = self.ids(pi, vi);
        let bytes = std::fs::read(self.files(&project_id, &version_id).head()).map_err(|_| Error::not_found("this version has no cut yet"))?;
        Ok((bytes, self.seq_of(&project_id, &version_id)?))
    }

    pub fn head(&self, role: &Role, version_id: &str) -> Result<HeadInfo, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let (project_id, version_id) = self.ids(pi, vi);
        Ok(HeadInfo { seq: self.seq_of(&project_id, &version_id)?, status: self.projects[pi].versions[vi].status })
    }

    pub fn actions_since(&mut self, role: &Role, version_id: &str, since: u64) -> Result<ActionsSince, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let live = self.live(pi, vi)?;
        Ok(ActionsSince { seq: live.events.len() as u64, actions: live.events.iter().filter(|e| e.seq > since).cloned().collect() })
    }

    pub fn media(&self, role: &Role, version_id: &str, n: usize) -> Result<Vec<u8>, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let entry = self.projects[pi].versions[vi].media.get(n).ok_or_else(|| Error::not_found("no such media file"))?;
        match role {
            Role::Editor { .. } => self.storage.read(&entry.key),
            _ => match &entry.proxy_key {
                Some(key) => self.storage.read(key),
                None => Err(Error::forbidden("review links get proxies, and this file has none yet")),
            },
        }
    }

    // ----- the action log (server replay) -----

    /// API.md `POST /api/v/{version}/actions`: gate every command, then replay the batch in
    /// order on the version's session. Any failure leaves nothing behind: the session is
    /// rebuilt from the last good state and the answer is 409.
    pub fn post_actions(&mut self, role: &Role, version_id: &str, body: ActionsIn) -> Result<ActionsOut, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let engine_role = role.engine_role()?;
        if engine_role == EngineRole::Reviewer && self.projects[pi].versions[vi].status == Status::AgentWorking {
            return Err(Error::locked("your editor is working; the cut is read-only until it is marked ready"));
        }
        if body.actions.is_empty() {
            return Err(Error::invalid("the batch has no commands"));
        }
        if body.actions.len() > MAX_BATCH {
            return Err(Error::invalid(format!("a batch holds at most {MAX_BATCH} commands")));
        }
        let seq = self.live(pi, vi)?.events.len() as u64;
        if body.based_on_seq != seq {
            return Err(Error::conflict(format!("this cut was updated (step {seq}); re-open it and redo your change")).with_seq(seq));
        }
        for action in &body.actions {
            if action.id.starts_with("portal.") {
                return Err(Error::forbidden(format!("{}: portal actions have their own routes", action.id)));
            }
            review::allows(engine_role, &action.id).map_err(|why| Error::forbidden(format!("{}: {why}", action.id)))?;
        }
        let actor = role.actor();
        let reason = body.reason.as_ref().map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
        if let Some(reason) = &reason
            && reason.chars().count() > MAX_TEXT
        {
            return Err(Error::invalid("the reason is too long"));
        }
        self.ensure_session(pi, vi)?;
        let (project_id, vid) = self.ids(pi, vi);
        let files = self.files(&project_id, &vid);
        let at = stamp();
        let outcome = {
            let live = self.live.get_mut(&vid).ok_or_else(|| Error::not_found("no version"))?;
            let session = live.session.as_mut().ok_or_else(|| Error::invalid("the version's session is not open"))?;
            let mut stacks = Stacks::of(&live.events);
            let mut added: Vec<ActionEvent> = Vec::new();
            let mut failure = None;
            for action in body.actions {
                let targets = targets_of(&action.params);
                match session.run(engine_role, &action.id, action.params.clone()) {
                    Ok(ran) => {
                        if engine_role == EngineRole::Editor
                            && ran.project_changed
                            && reason.is_none()
                            && let Some(target) = overwrites_client(&live.events, &targets)
                        {
                            failure = Some(Error::conflict(format!("{} changes {target}, which a client edited last; send a reason", action.id)));
                            break;
                        }
                        let reverts_seq = if ran.project_changed { stacks.reverts(&action.id) } else { None };
                        let event = ActionEvent {
                            seq: seq + added.len() as u64 + 1,
                            at: at.clone(),
                            actor: actor.clone(),
                            role: role_name(engine_role).to_string(),
                            id: action.id,
                            params: action.params,
                            label: if ran.label.is_empty() { action.label.unwrap_or_default() } else { ran.label },
                            project_changed: ran.project_changed,
                            based_on_seq: seq,
                            reason: reason.clone(),
                            client_ts: action.client_ts,
                            reverts_seq,
                            targets,
                        };
                        stacks.push(&event);
                        added.push(event);
                    }
                    Err(why) => {
                        failure = Some(Error::conflict(format!("{} could not be applied ({why}); nothing in this batch was kept", action.id)));
                        break;
                    }
                }
            }
            match failure {
                Some(error) => Err(error),
                None => {
                    let changed = added.iter().any(|e| e.project_changed);
                    let mut all = live.events.clone();
                    all.extend(added);
                    let written = write_log(&files.actions(), &all).and_then(|()| if changed { storage::atomic_write(&files.head(), &session.project_bytes()) } else { Ok(()) });
                    written.map(|()| all)
                }
            }
        };
        match outcome {
            Ok(all) => {
                let accepted = all.len() - seq as usize;
                let new_seq = all.len() as u64;
                if let Some(live) = self.live.get_mut(&vid) {
                    live.events = all;
                }
                self.write_feedback(pi, vi)?;
                Ok(ActionsOut { seq: new_seq, accepted })
            }
            Err(error) => {
                // Rebuild from the last good state on next use (base + the log as written).
                self.live.remove(&vid);
                Err(Error { seq: Some(seq), ..error })
            }
        }
    }

    // ----- comments, marks, status (portal actions) -----

    pub fn comments(&self, role: &Role, version_id: &str) -> Result<Vec<Comment>, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let (project_id, version_id) = self.ids(pi, vi);
        persist::read_comments(&self.files(&project_id, &version_id).comments())
    }

    pub fn add_comment(&mut self, role: &Role, version_id: &str, body: CommentIn) -> Result<Comment, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        no_watchers(role)?;
        let text = clean_text(&body.body, MAX_TEXT, "comment")?;
        if body.start_tick.0 < 0 || body.end_tick.0 < body.start_tick.0 {
            return Err(Error::invalid("a comment's end is before its start"));
        }
        let region = body.region.map(Region::checked).transpose()?;
        let (project_id, vid) = self.ids(pi, vi);
        let path = self.files(&project_id, &vid).comments();
        let mut comments = persist::read_comments(&path)?;
        if comments.len() >= MAX_COMMENTS {
            return Err(Error::invalid("this version has too many comments"));
        }
        let comment = Comment {
            id: ids::nid("cm")?,
            author: role.actor(),
            at: stamp(),
            start_tick: body.start_tick,
            end_tick: body.end_tick,
            region,
            body: text,
            resolved: false,
            fixed_in: None,
            replies: Vec::new(),
        };
        comments.push(comment.clone());
        persist::write_comments(&path, &comments)?;
        let params = json!({"comment": comment.id, "body": comment.body, "start_tick": comment.start_tick, "end_tick": comment.end_tick, "region": comment.region});
        self.log_portal(pi, vi, role, "portal.comment", params, "Comment")?;
        Ok(comment)
    }

    pub fn reply(&mut self, role: &Role, version_id: &str, comment_id: &str, body: ReplyIn) -> Result<Comment, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        no_watchers(role)?;
        let text = clean_text(&body.body, MAX_TEXT, "reply")?;
        let author = role.actor();
        let comment = self.change_comment(pi, vi, comment_id, |c| {
            c.replies.push(Reply { author, at: stamp(), body: text.clone() });
            Ok(())
        })?;
        self.log_portal(pi, vi, role, "portal.reply", json!({"comment": comment_id, "body": text}), "Reply")?;
        Ok(comment)
    }

    pub fn resolve(&mut self, role: &Role, version_id: &str, comment_id: &str, body: ResolveIn) -> Result<Comment, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        no_watchers(role)?;
        let fixed_in = body.fixed_in.as_deref().map(|f| clean_text(f, MAX_TEXT, "fixed in")).transpose()?;
        let comment = self.change_comment(pi, vi, comment_id, |c| {
            c.resolved = body.resolved;
            c.fixed_in = fixed_in.clone();
            Ok(())
        })?;
        let label = if body.resolved { "Resolve comment" } else { "Reopen comment" };
        self.log_portal(pi, vi, role, "portal.resolve", json!({"comment": comment_id, "resolved": body.resolved, "fixed_in": fixed_in}), label)?;
        Ok(comment)
    }

    /// Reviewer only: the client's Approve / Request changes. Writes approval.json and
    /// feedback.json.
    pub fn mark(&mut self, role: &Role, version_id: &str, body: MarkIn) -> Result<Value, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        let Role::Reviewer { name, .. } = role else {
            return Err(Error::forbidden("approval is the client's mark"));
        };
        let note = body.note.trim().to_string();
        if note.chars().count() > MAX_TEXT {
            return Err(Error::invalid("the note is too long"));
        }
        let approval = Approval { mark: body.mark, note: note.clone(), by: name.clone(), at: stamp() };
        {
            let version = &mut self.projects[pi].versions[vi];
            version.approval = Some(approval.clone());
            version.status = match body.mark {
                Mark::Good => Status::Approved,
                Mark::NotGood => Status::ChangesRequested,
            };
        }
        self.save_meta()?;
        let label = match body.mark {
            Mark::Good => "Approve",
            Mark::NotGood => "Request changes",
        };
        self.log_portal(pi, vi, role, "portal.mark", json!({"mark": body.mark, "note": note}), label)?;
        Ok(serde_json::to_value(feedback::approval_json(&self.projects[pi].versions[vi].label, &approval)).unwrap_or(Value::Null))
    }

    /// Editor only: `agent_working` locks review links, `ready` hands the cut back.
    pub fn set_status(&mut self, role: &Role, version_id: &str, body: StatusIn) -> Result<HeadInfo, Error> {
        let (pi, vi) = self.version_index(role, version_id)?;
        if !matches!(role, Role::Editor { .. }) {
            return Err(Error::forbidden("only the editor link sets the status"));
        }
        if !matches!(body.status, Status::AgentWorking | Status::Ready) {
            return Err(Error::invalid("status must be agent_working or ready"));
        }
        self.projects[pi].versions[vi].status = body.status;
        self.save_meta()?;
        let label = if body.status == Status::AgentWorking { "Editor working" } else { "Ready for review" };
        self.log_portal(pi, vi, role, "portal.status", json!({"status": body.status}), label)?;
        let (project_id, vid) = self.ids(pi, vi);
        Ok(HeadInfo { seq: self.seq_of(&project_id, &vid)?, status: body.status })
    }

    pub fn feedback(&self, role: &Role) -> Result<FeedbackFile, Error> {
        let project_id = role.project_id().ok_or_else(|| Error::forbidden("open an editor, review or watch link"))?;
        let pi = self.projects.iter().position(|p| p.id == project_id).ok_or_else(|| Error::not_found("no project"))?;
        let versions = (0..self.projects[pi].versions.len()).map(|vi| self.version_feedback(pi, vi)).collect::<Result<Vec<_>, _>>()?;
        Ok(self.feedback_file(pi, versions))
    }

    // ----- admin, client home, projects -----

    pub fn overview(&self, role: &Role) -> Result<Overview, Error> {
        if !matches!(role, Role::Admin) {
            return Err(Error::forbidden("the dashboard is for the admin link"));
        }
        Ok(Overview {
            clients: self
                .clients
                .iter()
                .map(|c| ClientCard { id: c.id.clone(), name: c.name.clone(), token: c.token.clone(), created_at: c.created_at.clone() })
                .collect(),
            projects: self
                .projects
                .iter()
                .map(|p| ProjectCard {
                    id: p.id.clone(),
                    client_name: self.client_name(&p.client_id).unwrap_or_else(|| "Client".to_string()),
                    name: p.name.clone(),
                    final_type: p.plan.final_type.as_str().to_string(),
                    editor_token: p.editor_token.clone(),
                    reviewer_token: p.reviewer_token.clone(),
                    watcher_token: p.watcher_token.clone(),
                    versions: version_cards(p),
                    created_at: p.created_at.clone(),
                })
                .collect(),
        })
    }

    pub fn create_client(&mut self, role: &Role, name: &str) -> Result<ClientCard, Error> {
        if !matches!(role, Role::Admin) {
            return Err(Error::forbidden("only the admin link adds clients"));
        }
        let name = crate::model::clean_name(name, "client name")?;
        let client = ClientRecord { id: ids::nid("c")?, name: name.clone(), token: ids::token()?, created_at: stamp() };
        let card = ClientCard { id: client.id.clone(), name, token: client.token.clone(), created_at: client.created_at.clone() };
        self.clients.push(client);
        self.save_meta()?;
        Ok(card)
    }

    pub fn home(&self, role: &Role) -> Result<Home, Error> {
        let Role::Client { client_id, name } = role else {
            return Err(Error::forbidden("open a client link"));
        };
        let projects = self
            .projects
            .iter()
            .filter(|p| &p.client_id == client_id)
            .map(|p| HomeProject {
                id: p.id.clone(),
                name: p.name.clone(),
                final_type: p.plan.final_type.as_str().to_string(),
                reviewer_token: p.reviewer_token.clone(),
                versions: version_cards(p),
            })
            .collect();
        let broll = self
            .broll
            .iter()
            .filter(|b| &b.client_id == client_id)
            .map(|b| HomeBroll { id: b.id.clone(), name: b.name.clone(), width: b.width, height: b.height, duration_ticks: b.duration.0.to_string() })
            .collect();
        Ok(Home { client_name: name.clone(), projects, broll })
    }

    pub fn check_brief(&self, role: &Role, brief: &UploadBrief) -> Result<BriefPlan, Vec<brief::FieldError>> {
        let client_id = match self.client_scope(role) {
            Ok(id) => id,
            Err(error) => return Err(vec![brief::FieldError { field: "auth".to_string(), message: error.message }]),
        };
        brief::validate_brief(brief, &self.known_broll(&client_id))
    }

    /// A client's upload form: a project with one version per planned version. Each version's
    /// base is an empty project until the editor assembles the first cut.
    pub fn create_project(&mut self, role: &Role, brief: UploadBrief) -> Result<HomeProject, Error> {
        let Role::Client { client_id, .. } = role else {
            return Err(Error::forbidden("projects are created from a client link"));
        };
        let plan = brief::validate_brief(&brief, &self.known_broll(client_id))
            .map_err(|errors| Error::invalid(errors.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ")))?;
        let project_id = ids::nid("p")?;
        let mut versions = Vec::new();
        for planned in &plan.versions {
            let id = ids::nid("v")?;
            VersionFiles::new(&self.root, &project_id, &id).create(&replay::empty_project(&format!("{} - {}", plan.project_name, planned.difference)))?;
            versions.push(VersionRecord { id, label: planned.difference.clone(), status: Status::Draft, approval: None, media: Vec::new() });
        }
        let project = ProjectRecord {
            id: project_id,
            client_id: client_id.clone(),
            name: plan.project_name.clone(),
            reviewer_token: ids::token()?,
            editor_token: ids::token()?,
            watcher_token: ids::token()?,
            plan,
            versions,
            created_at: stamp(),
        };
        let home = HomeProject {
            id: project.id.clone(),
            name: project.name.clone(),
            final_type: project.plan.final_type.as_str().to_string(),
            reviewer_token: project.reviewer_token.clone(),
            versions: version_cards(&project),
        };
        self.projects.push(project);
        self.save_meta()?;
        Ok(home)
    }

    // ----- uploads (PR #5, unchanged rules) -----

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
                for _ in 0..count.min(8) {
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
        owns_key(&client_id, &manifest.key)?;
        self.storage.write_part(upload_id, part, bytes)
    }

    pub fn upload_status(&self, role: &Role, upload_id: &str) -> Result<UploadManifest, Error> {
        let client_id = self.client_scope(role)?;
        let manifest = self.storage.status(upload_id)?;
        owns_key(&client_id, &manifest.key)?;
        Ok(manifest)
    }

    pub fn complete_upload(&mut self, role: &Role, upload_id: &str, meta: CompleteMeta) -> Result<UploadManifest, Error> {
        let client_id = self.client_scope(role)?;
        let current = self.storage.status(upload_id)?;
        owns_key(&client_id, &current.key)?;
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

    // ----- internals -----

    /// The project and version indexes of `version_id`, refusing (403) a token for another project.
    fn version_index(&self, role: &Role, version_id: &str) -> Result<(usize, usize), Error> {
        let token_project = role.project_id().ok_or_else(|| Error::forbidden("open an editor, review or watch link"))?;
        for (pi, project) in self.projects.iter().enumerate() {
            if let Some(vi) = project.versions.iter().position(|v| v.id == version_id) {
                if project.id != token_project {
                    return Err(Error::forbidden("that version belongs to another project"));
                }
                return Ok((pi, vi));
            }
        }
        Err(Error::not_found("no such version"))
    }

    fn ids(&self, pi: usize, vi: usize) -> (String, String) {
        (self.projects[pi].id.clone(), self.projects[pi].versions[vi].id.clone())
    }

    fn files(&self, project_id: &str, version_id: &str) -> VersionFiles {
        VersionFiles::new(&self.root, project_id, version_id)
    }

    fn seq_of(&self, project_id: &str, version_id: &str) -> Result<u64, Error> {
        match self.live.get(version_id) {
            Some(live) => Ok(live.events.len() as u64),
            None => Ok(read_log(&self.files(project_id, version_id).actions())?.len() as u64),
        }
    }

    fn live(&mut self, pi: usize, vi: usize) -> Result<&mut Live, Error> {
        let (project_id, version_id) = self.ids(pi, vi);
        if !self.live.contains_key(&version_id) {
            let events = read_log(&self.files(&project_id, &version_id).actions())?;
            self.live.insert(version_id.clone(), Live { events, session: None });
        }
        self.live.get_mut(&version_id).ok_or_else(|| Error::not_found("no version"))
    }

    /// Open the version's server session: base + replay of the log. If the log no longer
    /// replays (an engine change), open the saved head instead.
    fn ensure_session(&mut self, pi: usize, vi: usize) -> Result<(), Error> {
        let (project_id, version_id) = self.ids(pi, vi);
        let files = self.files(&project_id, &version_id);
        let media = self.media_map(pi, vi)?;
        let live = self.live(pi, vi)?;
        if live.session.is_some() {
            return Ok(());
        }
        let mut session = Replayer::open(&files.base(), &media)?;
        for event in live.events.iter().filter(|e| !e.is_portal()) {
            if session.run(parse_role(&event.role), &event.id, event.params.clone()).is_err() {
                session = Replayer::open(&files.head(), &media)?;
                break;
            }
        }
        live.session = Some(session);
        Ok(())
    }

    fn media_map(&self, pi: usize, vi: usize) -> Result<Vec<(String, PathBuf)>, Error> {
        self.projects[pi].versions[vi].media.iter().map(|m: &MediaEntry| Ok((m.path.clone(), self.storage.path_of(&m.key)?))).collect()
    }

    fn change_comment(&self, pi: usize, vi: usize, comment_id: &str, change: impl FnOnce(&mut Comment) -> Result<(), Error>) -> Result<Comment, Error> {
        let (project_id, version_id) = self.ids(pi, vi);
        let path = self.files(&project_id, &version_id).comments();
        let mut comments = persist::read_comments(&path)?;
        let comment = comments.iter_mut().find(|c| c.id == comment_id).ok_or_else(|| Error::not_found("no such comment"))?;
        if comment.replies.len() >= MAX_COMMENTS {
            return Err(Error::invalid("this thread is full"));
        }
        change(comment)?;
        let out = comment.clone();
        persist::write_comments(&path, &comments)?;
        Ok(out)
    }

    /// Append a portal action (not replayed) and refresh feedback.json.
    fn log_portal(&mut self, pi: usize, vi: usize, role: &Role, id: &str, params: Value, label: &str) -> Result<u64, Error> {
        let engine_role = role.engine_role()?;
        let (project_id, version_id) = self.ids(pi, vi);
        let path = self.files(&project_id, &version_id).actions();
        let actor = role.actor();
        let live = self.live(pi, vi)?;
        let seq = live.events.len() as u64;
        let event = ActionEvent {
            seq: seq + 1,
            at: stamp(),
            actor,
            role: role_name(engine_role).to_string(),
            id: id.to_string(),
            params,
            label: label.to_string(),
            project_changed: false,
            based_on_seq: seq,
            reason: None,
            client_ts: None,
            reverts_seq: None,
            targets: Vec::new(),
        };
        let mut all = live.events.clone();
        all.push(event);
        write_log(&path, &all)?;
        live.events = all;
        self.write_feedback(pi, vi)?;
        Ok(seq + 1)
    }

    fn version_feedback(&self, pi: usize, vi: usize) -> Result<VersionFeedback, Error> {
        let (project_id, version_id) = self.ids(pi, vi);
        let files = self.files(&project_id, &version_id);
        let version = &self.projects[pi].versions[vi];
        let events = match self.live.get(&version_id) {
            Some(live) => live.events.clone(),
            None => read_log(&files.actions())?,
        };
        let comments = persist::read_comments(&files.comments())?;
        let base = std::fs::read(files.base()).ok().and_then(|b| replay::project_content(&b));
        let head = std::fs::read(files.head()).ok().and_then(|b| replay::project_content(&b));
        Ok(feedback::version_feedback(
            &version.id,
            &version.label,
            version.status.as_str(),
            version.approval.as_ref(),
            &comments,
            &events,
            base.as_ref(),
            head.as_ref(),
        ))
    }

    fn feedback_file(&self, pi: usize, versions: Vec<VersionFeedback>) -> FeedbackFile {
        let project = &self.projects[pi];
        FeedbackFile {
            schema: feedback::SCHEMA.to_string(),
            client: self.client_name(&project.client_id).unwrap_or_else(|| "Client".to_string()),
            project: project.name.clone(),
            final_type: project.plan.final_type,
            versions,
        }
    }

    /// Rewrite the version's feedback/feedback.json and, once there is a mark, approval.json.
    fn write_feedback(&self, pi: usize, vi: usize) -> Result<(), Error> {
        let (project_id, version_id) = self.ids(pi, vi);
        let files = self.files(&project_id, &version_id);
        let file = self.feedback_file(pi, vec![self.version_feedback(pi, vi)?]);
        let bytes = serde_json::to_vec_pretty(&file).map_err(|_| Error::invalid("could not write feedback"))?;
        storage::atomic_write(&files.feedback(), &bytes)?;
        let version = &self.projects[pi].versions[vi];
        if let Some(approval) = &version.approval {
            let bytes = serde_json::to_vec_pretty(&feedback::approval_json(&version.label, approval)).map_err(|_| Error::invalid("could not write approval"))?;
            storage::atomic_write(&files.approval(), &bytes)?;
        }
        Ok(())
    }

    fn save_meta(&self) -> Result<(), Error> {
        persist::save_saved(
            &self.root,
            &Saved { admin_token: self.admin_token.clone(), clients: self.clients.clone(), projects: self.projects.clone(), broll: self.broll.clone() },
        )
    }

    fn client_scope(&self, role: &Role) -> Result<String, Error> {
        match role {
            Role::Admin => Err(Error::invalid("choose a client link for uploads and new projects")),
            Role::Client { client_id, .. } | Role::Reviewer { client_id, .. } => Ok(client_id.clone()),
            Role::Editor { project_id } | Role::Watcher { project_id } => Ok(self.project(project_id)?.client_id.clone()),
        }
    }

    fn project(&self, id: &str) -> Result<&ProjectRecord, Error> {
        self.projects.iter().find(|p| p.id == id).ok_or_else(|| Error::not_found("no project"))
    }

    fn client_name(&self, client_id: &str) -> Option<String> {
        self.clients.iter().find(|c| c.id == client_id).map(|c| c.name.clone())
    }

    fn known_broll(&self, client_id: &str) -> Vec<String> {
        self.broll.iter().filter(|b| b.client_id == client_id).map(|b| b.id.clone()).collect()
    }
}

/// The client whose step on the chain last touched one of `targets`, when that step is a
/// client's: an editor command changing it needs a reason (PR #5's overwrite rule).
fn overwrites_client(events: &[ActionEvent], targets: &[String]) -> Option<String> {
    let chain = effective_chain(events);
    targets.iter().find(|t| chain.iter().rev().find(|e| e.targets.contains(t)).is_some_and(|e| e.actor.is_client())).cloned()
}

fn no_watchers(role: &Role) -> Result<(), Error> {
    if matches!(role, Role::Watcher { .. }) { Err(Error::forbidden("this link can watch but not comment")) } else { Ok(()) }
}

fn version_cards(project: &ProjectRecord) -> Vec<VersionCard> {
    project.versions.iter().map(|v| VersionCard { id: v.id.clone(), label: v.label.clone(), status: v.status }).collect()
}

fn owns_key(client_id: &str, key: &str) -> Result<(), Error> {
    if key.starts_with(&format!("c/{client_id}/")) { Ok(()) } else { Err(Error::forbidden("that file belongs to another client")) }
}

fn stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
