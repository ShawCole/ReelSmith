//! The portal service end to end over HTTP (API.md): link roles, the role gate on posted
//! commands, server replay into head.fcproj, the append-only log, cross-origin isolation, and
//! the style-pipeline approval file.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use reelsmith_engine::review::Role as EngineRole;
use reelsmith_portal::brief::{RawFile, UploadBrief};
use reelsmith_portal::history::read_log;
use reelsmith_portal::model::{FinalType, MediaRole, MediaTick};
use reelsmith_portal::replay::{Replayer, project_content};
use reelsmith_portal::service::{Portal, Role, StorageMode};
use serde_json::{Value, json};
use std::path::PathBuf;
use tower::ServiceExt;

const ADMIN: &str = "0000000000000000000000000000000000000000000000000000000000000a11";

struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    app: Router,
    editor: String,
    reviewer: String,
    watcher: String,
    client: String,
    project: String,
    version: String,
    other_reviewer: String,
    other_version: String,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
    let root = dir.path().join("data");
    let dist = dir.path().join("dist");
    std::fs::create_dir_all(&dist).unwrap_or_else(|e| panic!("dist: {e}"));
    std::fs::write(dist.join("index.html"), "<!doctype html><title>editor</title>").unwrap_or_else(|e| panic!("index: {e}"));
    std::fs::write(dist.join("reelsmith_web_bg.wasm"), b"\0asm\x01\0\0\0").unwrap_or_else(|e| panic!("wasm: {e}"));
    std::fs::write(dist.join("reelsmith_web.js"), "export default 1;").unwrap_or_else(|e| panic!("js: {e}"));
    let mut portal = Portal::open(root.clone(), Some(ADMIN.to_string()), StorageMode::Local, 1 << 30).unwrap_or_else(|e| panic!("open: {e}"));
    let overview = portal.overview(&Role::Admin).unwrap_or_else(|e| panic!("overview: {e}"));
    let seeded = &overview.projects[0];
    let client = &overview.clients[0];
    let client_role = Role::Client { client_id: client.id.clone(), name: client.name.clone() };
    let brief = UploadBrief {
        project_name: "Second project".to_string(),
        final_type: FinalType::Ad,
        target: None,
        end_card: None,
        version_count: 1,
        differences: vec![],
        use_broll: false,
        broll_library_ids: vec![],
        broll_uploads: vec![],
        raw_files: vec![RawFile { name: "Take".to_string(), width: 1080, height: 1920, duration: MediaTick(10 * reelsmith_time::TICKS_PER_SECOND), role: MediaRole::Aroll }],
        change_format: false,
        explicit_width: None,
        explicit_height: None,
        profile_width: None,
        profile_height: None,
    };
    let other = portal.create_project(&client_role, brief).unwrap_or_else(|e| panic!("project B: {e}"));
    World {
        root,
        editor: seeded.editor_token.clone(),
        reviewer: seeded.reviewer_token.clone(),
        watcher: seeded.watcher_token.clone(),
        client: client.token.clone(),
        project: seeded.id.clone(),
        version: seeded.versions[0].id.clone(),
        other_reviewer: other.reviewer_token.clone(),
        other_version: other.versions[0].id.clone(),
        app: reelsmith_portal::router(portal, dist),
        _dir: dir,
    }
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).unwrap_or(Value::Null)
    }
}

async fn call(w: &World, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> Answer {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => request.header("content-type", "application/json").body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap_or_else(|e| panic!("request: {e}"));
    let response = w.app.clone().oneshot(request).await.unwrap_or_else(|e| panic!("call: {e}"));
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
    Answer { status, headers, bytes }
}

async fn head_content(w: &World) -> Value {
    let answer = call(w, "GET", &format!("/api/v/{}/project", w.version), Some(&w.reviewer), None).await;
    assert_eq!(answer.status, StatusCode::OK);
    project_content(&answer.bytes).unwrap_or_else(|| panic!("head is not a project"))
}

async fn seq(w: &World) -> u64 {
    call(w, "GET", &format!("/api/v/{}/head", w.version), Some(&w.reviewer), None).await.json()["seq"].as_u64().unwrap_or(u64::MAX)
}

/// The first clip on the first video track of the seeded (demo) project.
fn first_video_clip(project: &Value) -> u64 {
    fn find(v: &Value) -> Option<u64> {
        match v {
            Value::Object(map) => {
                if let Some(Value::Array(tracks)) = map.get("video_tracks")
                    && let Some(id) = tracks.iter().find_map(|t| t["items"].get(0).and_then(|i| i["id"].as_u64()))
                {
                    return Some(id);
                }
                map.values().find_map(find)
            }
            Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    find(project).unwrap_or_else(|| panic!("the demo project has no video clip"))
}

fn trim(clip: u64, frames: i64) -> Value {
    json!({"id": "timeline.trim", "params": {"clip": clip, "edge": "out", "mode": "regular", "deltaFrames": frames}, "label": "Trim Edit", "project_changed": true, "client_ts": "2026-11-18T23:59:00Z"})
}

fn actions_path(w: &World) -> PathBuf {
    w.root.join("projects").join(&w.project).join("versions").join(&w.version).join("actions.jsonl")
}

#[tokio::test]
async fn boot_gives_each_link_its_role_and_refuses_other_projects() {
    let w = world();
    let editor = call(&w, "GET", "/api/boot", Some(&w.editor), None).await;
    assert_eq!(editor.status, StatusCode::OK);
    let editor = editor.json();
    assert_eq!(editor["role"], "editor");
    assert_eq!(editor["actor"]["kind"], "agent");
    assert_eq!(editor["project"]["name"], "Spring launch reel");
    assert_eq!(editor["version"]["id"], w.version.as_str());
    assert_eq!(editor["project_url"], format!("/api/v/{}/project", w.version));
    assert_eq!(editor["seq"], 0);
    assert!(editor["media"].as_array().is_some_and(Vec::is_empty), "the demo project uses generator media");

    let reviewer = call(&w, "GET", "/api/boot", Some(&w.reviewer), None).await.json();
    assert_eq!(reviewer["role"], "reviewer");
    assert_eq!(reviewer["actor"]["kind"], "client");
    assert_eq!(reviewer["actor"]["name"], "Harbor & Co");
    assert_eq!(call(&w, "GET", "/api/boot", Some(&w.watcher), None).await.json()["role"], "watcher");

    for token in [ADMIN, w.client.as_str(), "not-a-token"] {
        assert_eq!(call(&w, "GET", "/api/boot", Some(token), None).await.status, StatusCode::FORBIDDEN, "admin, client and unknown tokens do not boot the editor");
    }
    assert_eq!(call(&w, "GET", "/api/boot", None, None).await.status, StatusCode::FORBIDDEN);

    // A reviewer link for project A cannot reach project B's version, by boot or by route.
    let b = &w.other_version;
    assert_eq!(call(&w, "GET", &format!("/api/boot?version={b}"), Some(&w.reviewer), None).await.status, StatusCode::FORBIDDEN);
    for path in [format!("/api/v/{b}/head"), format!("/api/v/{b}/project"), format!("/api/v/{b}/actions?since=0"), format!("/api/v/{b}/comments")] {
        assert_eq!(call(&w, "GET", &path, Some(&w.reviewer), None).await.status, StatusCode::FORBIDDEN, "{path}");
    }
    let post = call(&w, "POST", &format!("/api/v/{b}/actions"), Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(1, -1)]}))).await;
    assert_eq!(post.status, StatusCode::FORBIDDEN);
    // …while project B's own reviewer can.
    assert_eq!(call(&w, "GET", &format!("/api/v/{b}/head"), Some(&w.other_reviewer), None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_reviewer_trim_is_replayed_logged_with_the_token_actor_and_changes_head() {
    let w = world();
    let before = head_content(&w).await;
    let clip = first_video_clip(&before);
    let answer = call(&w, "POST", &format!("/api/v/{}/actions", w.version), Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -12)], "reason": null}))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.bytes));
    assert_eq!(answer.json(), json!({"seq": 1, "accepted": 1}));

    let after = head_content(&w).await;
    assert_ne!(after, before, "head.fcproj changed");
    let project = call(&w, "GET", &format!("/api/v/{}/project", w.version), Some(&w.watcher), None).await;
    assert_eq!(project.headers.get("x-portal-seq").and_then(|v| v.to_str().ok()), Some("1"));

    let log = call(&w, "GET", &format!("/api/v/{}/actions?since=0", w.version), Some(&w.editor), None).await.json();
    assert_eq!(log["seq"], 1);
    let step = &log["actions"][0];
    assert_eq!(step["seq"], 1);
    assert_eq!(step["id"], "timeline.trim");
    assert_eq!(step["role"], "reviewer");
    assert_eq!(step["actor"]["kind"], "client");
    assert_eq!(step["actor"]["name"], "Harbor & Co", "the actor comes from the token");
    assert_eq!(step["project_changed"], true);
    assert_eq!(step["based_on_seq"], 0);
    assert_eq!(step["client_ts"], "2026-11-18T23:59:00Z");
    assert!(step["at"].as_str().is_some_and(|at| at.ends_with('Z')), "server time in UTC");
    assert_eq!(read_log(&actions_path(&w)).unwrap_or_default().len(), 1, "written to actions.jsonl");
    assert_eq!(call(&w, "GET", &format!("/api/v/{}/actions?since=1", w.version), Some(&w.editor), None).await.json()["actions"], json!([]));
}

#[tokio::test]
async fn a_reviewer_may_not_nest_and_nothing_changes() {
    let w = world();
    let before = head_content(&w).await;
    let clip = first_video_clip(&before);
    let nest = json!({"id": "clip.nest", "params": {"clips": [clip]}});
    let answer = call(&w, "POST", &format!("/api/v/{}/actions", w.version), Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -6), nest]}))).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN, "{}", String::from_utf8_lossy(&answer.bytes));
    assert_eq!(seq(&w).await, 0);
    assert_eq!(head_content(&w).await, before, "the allowed trim before it did not run either");
    assert!(read_log(&actions_path(&w)).unwrap_or_default().is_empty());
    // A watcher may not trim at all.
    let watcher = call(&w, "POST", &format!("/api/v/{}/actions", w.version), Some(&w.watcher), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -6)]}))).await;
    assert_eq!(watcher.status, StatusCode::FORBIDDEN);
    assert_eq!(head_content(&w).await, before);
}

#[tokio::test]
async fn a_stale_batch_gets_409_and_nothing_changes() {
    let w = world();
    let clip = first_video_clip(&head_content(&w).await);
    let path = format!("/api/v/{}/actions", w.version);
    assert_eq!(call(&w, "POST", &path, Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -6)]}))).await.status, StatusCode::OK);
    let at_one = head_content(&w).await;
    let stale = call(&w, "POST", &path, Some(&w.editor), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -6)]}))).await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.json()["seq"], 1, "the 409 says where the cut is now");
    assert!(stale.json()["message"].is_string());
    assert_eq!(seq(&w).await, 1);
    assert_eq!(head_content(&w).await, at_one);
}

#[tokio::test]
async fn a_command_failing_mid_batch_rolls_the_whole_batch_back() {
    let w = world();
    let before = head_content(&w).await;
    let clip = first_video_clip(&before);
    let path = format!("/api/v/{}/actions", w.version);
    let broken = json!({"id": "timeline.trim", "params": {"clip": 987_654_321u64, "edge": "out", "mode": "regular", "deltaFrames": -3}});
    let answer = call(&w, "POST", &path, Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -12), broken]}))).await;
    assert_eq!(answer.status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&answer.bytes));
    assert_eq!(answer.json()["seq"], 0);
    assert_eq!(seq(&w).await, 0);
    assert_eq!(head_content(&w).await, before, "the first trim was rolled back with the batch");
    assert!(read_log(&actions_path(&w)).unwrap_or_default().is_empty());

    // The rebuilt session is the last good state: the same trim alone now gives one step.
    let ok = call(&w, "POST", &path, Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -12)]}))).await;
    assert_eq!(ok.status, StatusCode::OK);
    let mut once = Replayer::open(&w.root.join("projects").join(&w.project).join("versions").join(&w.version).join("base.fcproj"), &[]).unwrap_or_else(|e| panic!("open: {e}"));
    once.run(EngineRole::Reviewer, "timeline.trim", trim(clip, -12)["params"].clone()).unwrap_or_else(|e| panic!("trim: {e}"));
    assert_eq!(head_content(&w).await, once.project_json(), "the failed batch left nothing behind in the session");
}

#[tokio::test]
async fn replaying_the_log_on_base_gives_head() {
    let w = world();
    let clip = first_video_clip(&head_content(&w).await);
    let path = format!("/api/v/{}/actions", w.version);
    let batches = [
        (w.reviewer.as_str(), json!([trim(clip, -12), {"id": "timeline.select", "params": {"clips": [clip]}}, {"id": "clip.audioGain", "params": {"db": -3.0}}])),
        (w.reviewer.as_str(), json!([{"id": "playhead.set", "params": {"seconds": 1.0}}, {"id": "timeline.razor", "params": {"time": reelsmith_time::TICKS_PER_SECOND * 2, "track": "V1"}}])),
        (w.reviewer.as_str(), json!([{"id": "edit.undo", "params": {}}])),
        (w.editor.as_str(), json!([{"id": "timeline.select", "params": {"clips": [clip]}}, {"id": "clip.nest", "params": {"clips": [clip]}}])),
    ];
    for (token, actions) in batches {
        let at = seq(&w).await;
        let mut body = json!({"based_on_seq": at, "actions": actions});
        if token == w.editor {
            body["reason"] = json!("tightening the open");
        }
        let answer = call(&w, "POST", &path, Some(token), Some(body)).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.bytes));
    }
    // Comments are portal actions in the same log; replay skips them.
    let comment = call(&w, "POST", &format!("/api/v/{}/comments", w.version), Some(&w.reviewer), Some(json!({"start_tick": "0", "end_tick": "254016000000", "body": "Tighter here"}))).await;
    assert_eq!(comment.status, StatusCode::OK);

    let log = read_log(&actions_path(&w)).unwrap_or_else(|e| panic!("log: {e}"));
    let undo = log.iter().find(|e| e.id == "edit.undo").unwrap_or_else(|| panic!("no undo logged"));
    let razor = log.iter().find(|e| e.id == "timeline.razor").unwrap_or_else(|| panic!("no razor logged"));
    assert_eq!(undo.reverts_seq, Some(razor.seq), "undo names the razor it took back, skipping the playhead step");
    assert!(log.iter().any(|e| e.id == "portal.comment" && !e.project_changed));

    let dir = w.root.join("projects").join(&w.project).join("versions").join(&w.version);
    let mut replay = Replayer::open(&dir.join("base.fcproj"), &[]).unwrap_or_else(|e| panic!("open base: {e}"));
    for event in log.iter().filter(|e| !e.is_portal()) {
        let role = match event.role.as_str() {
            "reviewer" => EngineRole::Reviewer,
            "watcher" => EngineRole::Watcher,
            _ => EngineRole::Editor,
        };
        replay.run(role, &event.id, event.params.clone()).unwrap_or_else(|e| panic!("replaying {} failed: {e}", event.id));
    }
    let head = std::fs::read(dir.join("head.fcproj")).unwrap_or_else(|e| panic!("head: {e}"));
    assert_eq!(replay.project_json(), project_content(&head).unwrap_or(Value::Null), "base + actions.jsonl = head.fcproj (project content)");
}

#[tokio::test]
async fn every_response_is_cross_origin_isolated_and_wasm_is_typed() {
    let w = world();
    let paths: Vec<(String, Option<&str>)> = vec![
        ("/".to_string(), None),
        ("/assets/home.js".to_string(), None),
        ("/edit/".to_string(), None),
        ("/review/".to_string(), None),
        ("/watch/".to_string(), None),
        ("/review/reelsmith_web_bg.wasm".to_string(), None),
        ("/edit/reelsmith_web.js".to_string(), None),
        ("/review/../portal.json".to_string(), None),
        ("/api/boot".to_string(), None),
        ("/api/boot".to_string(), Some(w.reviewer.as_str())),
        (format!("/api/v/{}/project", w.version), Some(w.reviewer.as_str())),
        ("/nowhere".to_string(), None),
    ];
    for (path, token) in paths {
        let answer = call(&w, "GET", &path, token, None).await;
        let header = |name: &str| answer.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
        assert_eq!(header("cross-origin-opener-policy").as_deref(), Some("same-origin"), "COOP on {path} ({})", answer.status);
        assert_eq!(header("cross-origin-embedder-policy").as_deref(), Some("require-corp"), "COEP on {path}");
        assert_eq!(header("cross-origin-resource-policy").as_deref(), Some("same-origin"), "CORP on {path}");
    }
    let wasm = call(&w, "GET", "/review/reelsmith_web_bg.wasm", None, None).await;
    assert_eq!(wasm.status, StatusCode::OK);
    assert_eq!(wasm.headers.get("content-type").and_then(|v| v.to_str().ok()), Some("application/wasm"));
    assert_eq!(wasm.bytes, b"\0asm\x01\0\0\0");
    let index = call(&w, "GET", "/watch/", None, None).await;
    assert!(String::from_utf8_lossy(&index.bytes).contains("<title>editor</title>"), "the editor's index.html");
    assert_eq!(call(&w, "GET", "/review/../portal.json", None, None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn approval_json_has_exactly_the_spec_shape() {
    let w = world();
    let clip = first_video_clip(&head_content(&w).await);
    let path = format!("/api/v/{}/actions", w.version);
    assert_eq!(call(&w, "POST", &path, Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -12)]}))).await.status, StatusCode::OK);
    let mark_path = format!("/api/v/{}/mark", w.version);
    assert_eq!(call(&w, "POST", &mark_path, Some(&w.editor), Some(json!({"mark": "good", "note": ""}))).await.status, StatusCode::FORBIDDEN, "the editor does not approve");
    let mark = call(&w, "POST", &mark_path, Some(&w.reviewer), Some(json!({"mark": "good", "note": "Love the open"}))).await;
    assert_eq!(mark.status, StatusCode::OK);

    let dir = w.root.join("projects").join(&w.project).join("versions").join(&w.version).join("feedback");
    let approval: Value = serde_json::from_slice(&std::fs::read(dir.join("approval.json")).unwrap_or_default()).unwrap_or(Value::Null);
    let keys: Vec<&str> = approval.as_object().map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec!["at", "by", "mark", "note", "version"], "SPEC §5.1: version, mark, note, by, at and nothing else");
    assert_eq!(approval["mark"], "good");
    assert_eq!(approval["note"], "Love the open");
    assert_eq!(approval["by"], "Harbor & Co");
    assert!(approval["version"].is_string() && approval["at"].is_string());
    assert!(approval.as_object().is_some_and(|m| m.values().all(Value::is_string)));

    let feedback: Value = serde_json::from_slice(&std::fs::read(dir.join("feedback.json")).unwrap_or_default()).unwrap_or(Value::Null);
    assert_eq!(feedback["schema"], "reelsmith.portal.feedback/2");
    let version = &feedback["versions"][0];
    assert_eq!(version["approval"], approval);
    assert_eq!(version["status"], "approved");
    let edit = &version["client_actions"][0];
    assert_eq!(edit["command"], "timeline.trim");
    assert_eq!(edit["by"], "Harbor & Co");
    let (before, after) = (edit["before"][0]["duration_ticks"].as_str(), edit["after"][0]["duration_ticks"].as_str());
    assert!(before.is_some() && after.is_some() && before != after, "touched clip durations from base and head: {edit}");
    assert_eq!(call(&w, "GET", "/api/feedback", Some(&w.reviewer), None).await.json()["schema"], "reelsmith.portal.feedback/2");
}

#[tokio::test]
async fn while_the_editor_works_reviewers_watch() {
    let w = world();
    let clip = first_video_clip(&head_content(&w).await);
    let status = format!("/api/v/{}/status", w.version);
    assert_eq!(call(&w, "POST", &status, Some(&w.reviewer), Some(json!({"status": "agent_working"}))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(call(&w, "POST", &status, Some(&w.editor), Some(json!({"status": "agent_working"}))).await.status, StatusCode::OK);
    assert_eq!(call(&w, "GET", "/api/boot", Some(&w.reviewer), None).await.json()["role"], "watcher");
    let at = seq(&w).await;
    let locked = call(&w, "POST", &format!("/api/v/{}/actions", w.version), Some(&w.reviewer), Some(json!({"based_on_seq": at, "actions": [trim(clip, -6)]}))).await;
    assert_eq!(locked.status, StatusCode::LOCKED);
    assert_eq!(seq(&w).await, at);
    assert_eq!(call(&w, "POST", &status, Some(&w.editor), Some(json!({"status": "ready"}))).await.status, StatusCode::OK);
    assert_eq!(call(&w, "GET", "/api/boot", Some(&w.reviewer), None).await.json()["role"], "reviewer");
}

#[tokio::test]
async fn an_editor_changing_a_clients_edit_needs_a_reason() {
    let w = world();
    let clip = first_video_clip(&head_content(&w).await);
    let path = format!("/api/v/{}/actions", w.version);
    assert_eq!(call(&w, "POST", &path, Some(&w.reviewer), Some(json!({"based_on_seq": 0, "actions": [trim(clip, -12)]}))).await.status, StatusCode::OK);
    let at_one = head_content(&w).await;
    let silent = call(&w, "POST", &path, Some(&w.editor), Some(json!({"based_on_seq": 1, "actions": [trim(clip, 6)]}))).await;
    assert_eq!(silent.status, StatusCode::CONFLICT);
    assert_eq!(head_content(&w).await, at_one);
    let with_reason = call(&w, "POST", &path, Some(&w.editor), Some(json!({"based_on_seq": 1, "actions": [trim(clip, 6)], "reason": "the line needs its last word"}))).await;
    assert_eq!(with_reason.status, StatusCode::OK);
    let log = read_log(&actions_path(&w)).unwrap_or_default();
    assert_eq!(log[1].reason.as_deref(), Some("the line needs its last word"));
}
