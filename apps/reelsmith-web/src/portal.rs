//! Portal review links (`apps/reelsmith-portal`, docs/portal/DESIGN.md): the editor opened at
//! `/edit/#token`, `/review/#token` or `/watch/#token`.
//!
//! The portal is the single writer of the cut. This page opens the version's head project,
//! locks the session to the link's role (`reelsmith_engine::review`), and sends every top-level
//! command it runs to the portal, which replays it and logs who did it and when. When someone
//! else changes the version, the page re-opens the portal's head. The token stays in the URL
//! fragment and is only ever sent as `Authorization: Bearer`.

use std::cell::RefCell;

use reelsmith_engine::ExecutedCommand;
use reelsmith_engine::review::Role;
use reelsmith_ui_egui::review::{ActionRow, CommentRow, ReviewRequest, ReviewState};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::{WebApp, post};

/// How often the page asks the portal whether the version changed.
const POLL_MS: f64 = 1500.0;

#[derive(Clone, Debug, Deserialize)]
pub struct Actor {
    pub kind: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Named {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Media {
    pub path: String,
    pub url: String,
}

/// `GET /api/boot`.
#[derive(Clone, Debug, Deserialize)]
pub struct Boot {
    pub role: Role,
    pub actor: Actor,
    pub project: Named,
    pub version: Named,
    pub seq: u64,
    pub project_url: String,
    #[serde(default)]
    pub media: Vec<Media>,
}

/// What a link page knows between frames.
struct Link {
    token: String,
    version: String,
    /// The role the token was issued for; the page may run as a watcher while the editor works.
    issued: Role,
    project_url: String,
    seq: u64,
    pending: Vec<ExecutedCommand>,
    in_flight: bool,
    reloading: bool,
    last_poll: f64,
}

thread_local! {
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
}

/// Whether this page is a portal link that may not read or import files itself.
pub fn locked() -> bool {
    LINK.with(|l| l.borrow().is_some())
}

/// The token of a portal page URL (`/edit/`, `/review/`, `/watch/` + `#token`), if this is one.
pub fn token_from_location() -> Option<String> {
    let loc = web_sys::window()?.location();
    let path = loc.pathname().ok()?;
    if !["/edit/", "/review/", "/watch/"].iter().any(|p| path.starts_with(p)) {
        return None;
    }
    let hash = loc.hash().ok()?;
    let token = hash.trim_start_matches('#').trim_start_matches('/').to_string();
    (!token.is_empty()).then_some(token)
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

async fn request(method: &str, url: &str, token: &str, body: Option<&Value>) -> Result<(u16, web_sys::Response), JsValue> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    if let Some(b) = body {
        init.set_body(&JsValue::from_str(&b.to_string()));
    }
    let req = web_sys::Request::new_with_str_and_init(url, &init)?;
    req.headers().set("Authorization", &format!("Bearer {token}"))?;
    if body.is_some() {
        req.headers().set("Content-Type", "application/json")?;
    }
    let w = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = JsFuture::from(w.fetch_with_request(&req)).await?.dyn_into()?;
    Ok((resp.status(), resp))
}

async fn json_of(resp: &web_sys::Response) -> Value {
    match resp.text() {
        Ok(p) => JsFuture::from(p).await.ok().and_then(|t| t.as_string()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

async fn call(method: &str, url: &str, token: &str, body: Option<&Value>) -> Result<(u16, Value), JsValue> {
    let (status, resp) = request(method, url, token, body).await?;
    Ok((status, json_of(&resp).await))
}

/// The head project's bytes and the log position they are at.
async fn fetch_project(url: &str, token: &str) -> Result<(Vec<u8>, u64), JsValue> {
    let (status, resp) = request("GET", url, token, None).await?;
    if status != 200 {
        return Err(format!("project: HTTP {status}").into());
    }
    let seq = resp.headers().get("X-Portal-Seq").ok().flatten().and_then(|s| s.parse().ok()).unwrap_or(0);
    let buf = JsFuture::from(resp.array_buffer()?).await?;
    Ok((js_sys::Uint8Array::new(&buf).to_vec(), seq))
}

/// Boot a link: who this is, the head project, and its media registered where the project
/// expects them.
pub async fn boot(token: &str) -> Result<(Boot, Vec<u8>), JsValue> {
    let (status, v) = call("GET", "/api/boot", token, None).await?;
    if status != 200 {
        let msg = v["message"].as_str().unwrap_or("this link is not valid");
        return Err(format!("{msg} (HTTP {status})").into());
    }
    let mut boot: Boot = serde_json::from_value(v).map_err(|e| JsValue::from_str(&format!("boot: {e}")))?;
    for m in &boot.media {
        let (status, resp) = request("GET", &m.url, token, None).await?;
        if status != 200 {
            log::warn!("media {}: HTTP {status}", m.path);
            continue;
        }
        let blob: web_sys::Blob = JsFuture::from(resp.blob()?).await?.dyn_into()?;
        crate::fs::register_at(&m.path, blob);
    }
    let (bytes, seq) = fetch_project(&boot.project_url, token).await?;
    boot.seq = seq.max(boot.seq);
    LINK.with(|l| {
        *l.borrow_mut() = Some(Link {
            token: token.to_string(),
            version: boot.version.id.clone(),
            issued: boot.role,
            project_url: boot.project_url.clone(),
            seq: boot.seq,
            pending: Vec::new(),
            in_flight: false,
            reloading: false,
            last_poll: 0.0,
        });
    });
    Ok((boot, bytes))
}

/// The role the page runs as: a reviewer only watches while the editor is at work.
fn effective_role(issued: Role, status: &str) -> Role {
    if issued == Role::Reviewer && status == "agent_working" { Role::Watcher } else { issued }
}

/// Open the boot project and lock the app to the link. Called once the app exists.
pub fn open(app: &mut reelsmith_ui_egui::ReelsmithApp, boot: &Boot, bytes: Vec<u8>) {
    let path = format!("/portal/{}.fcproj", boot.version.id);
    crate::fs::put(&path, bytes);
    if let Err(e) = app.session.execute("file.open", json!({"path": path})) {
        log::warn!("portal project: {e}");
    }
    let mut state = ReviewState::new(effective_role(boot.role, &boot.version.status), &boot.actor.name);
    state.project_name = boot.project.name.clone();
    state.version_label = boot.version.label.clone();
    state.status = boot.version.status.clone();
    app.enter_review(state);
    // opening the project is not the reviewer's step
    let _ = app.session.take_executed();
    refresh_lists();
}

fn with_link<R>(f: impl FnOnce(&mut Link) -> R) -> Option<R> {
    LINK.with(|l| l.borrow_mut().as_mut().map(f))
}

/// Per frame: send what this page did, send its requests, and poll for other people's changes.
pub fn tick(web: &mut WebApp) {
    if !locked() {
        return;
    }
    let executed = web.app.session.take_executed();
    let requests = web.app.review.as_mut().map(|r| std::mem::take(&mut r.requests)).unwrap_or_default();
    with_link(|l| l.pending.extend(executed));
    for r in requests {
        send_request(r);
    }
    flush();
    let due = with_link(|l| {
        let due = !l.in_flight && !l.reloading && l.pending.is_empty() && now_ms() - l.last_poll >= POLL_MS;
        if due {
            l.last_poll = now_ms();
        }
        due
    })
    .unwrap_or(false);
    if due {
        poll();
    }
    // keep frames coming so the poll runs while nothing else repaints
    crate::repaint_after(POLL_MS as u64);
}

/// Send the commands this page ran, in one batch, based on the last log position it knows.
fn flush() {
    let Some((token, version, based_on, batch)) = with_link(|l| {
        if l.in_flight || l.reloading || l.pending.is_empty() {
            return None;
        }
        l.in_flight = true;
        Some((l.token.clone(), l.version.clone(), l.seq, std::mem::take(&mut l.pending)))
    })
    .flatten() else {
        return;
    };
    let now = js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default();
    let actions: Vec<Value> = batch
        .iter()
        .map(|c| json!({"id": c.id, "params": c.params, "label": c.label, "project_changed": c.project_changed, "client_ts": now}))
        .collect();
    let body = json!({"based_on_seq": based_on, "actions": actions, "reason": null});
    spawn_local(async move {
        let url = format!("/api/v/{version}/actions");
        let result = call("POST", &url, &token, Some(&body)).await;
        post(move |web, _| {
            with_link(|l| l.in_flight = false);
            match result {
                Ok((200, v)) => {
                    let seq = v["seq"].as_u64().unwrap_or(based_on);
                    with_link(|l| l.seq = l.seq.max(seq));
                    refresh_lists();
                }
                Ok((status, v)) => {
                    let message = v["message"].as_str().unwrap_or("the portal refused the change").to_string();
                    notice(web, format!("Not saved: {message} (HTTP {status}). Showing the latest version."));
                    reload(web);
                }
                Err(e) => {
                    notice(web, format!("Not saved: the portal could not be reached ({e:?})"));
                    reload(web);
                }
            }
        });
    });
}

fn notice(web: &mut WebApp, text: String) {
    if let Some(r) = web.app.review.as_mut() {
        r.notice = Some(text);
    }
}

/// Ask whether the version moved on; re-open it when someone else changed it.
fn poll() {
    let Some((token, version, seq)) = with_link(|l| (l.token.clone(), l.version.clone(), l.seq)) else { return };
    spawn_local(async move {
        let head = call("GET", &format!("/api/v/{version}/head"), &token, None).await;
        post(move |web, _| {
            let Ok((200, v)) = head else { return };
            let status = v["status"].as_str().unwrap_or_default().to_string();
            let issued = with_link(|l| l.issued).unwrap_or(Role::Watcher);
            web.app.set_review_role(effective_role(issued, &status));
            if let Some(r) = web.app.review.as_mut() {
                r.status = status;
            }
            match v["seq"].as_u64() {
                Some(head) if head != seq => moved_on(seq, head),
                _ => {}
            }
        });
    });
}

/// The log grew past `seq`: re-open the project when one of the new steps changed it (someone
/// else's edit, an undo), otherwise (comments, marks, status) only take the new position.
fn moved_on(seq: u64, head: u64) {
    let Some((token, version)) = with_link(|l| (l.token.clone(), l.version.clone())) else { return };
    spawn_local(async move {
        let since = call("GET", &format!("/api/v/{version}/actions?since={seq}"), &token, None).await;
        post(move |web, _| {
            let changed = match &since {
                Ok((200, v)) => v["actions"].as_array().is_none_or(|a| a.iter().any(|x| x["project_changed"].as_bool().unwrap_or(true))),
                _ => true,
            };
            if changed {
                reload(web);
            } else {
                with_link(|l| l.seq = l.seq.max(head));
            }
            refresh_lists();
        });
    });
}

/// Re-open the portal's head project, keeping the playhead.
fn reload(_web: &mut WebApp) {
    let Some((token, url)) = with_link(|l| {
        if l.reloading {
            return None;
        }
        l.reloading = true;
        l.pending.clear();
        Some((l.token.clone(), l.project_url.clone()))
    })
    .flatten() else {
        return;
    };
    spawn_local(async move {
        let fetched = fetch_project(&url, &token).await;
        post(move |web, _| {
            with_link(|l| l.reloading = false);
            let Ok((bytes, seq)) = fetched else {
                notice(web, "Could not load the latest version".to_string());
                return;
            };
            let version = with_link(|l| l.version.clone()).unwrap_or_default();
            let path = format!("/portal/{version}.fcproj");
            crate::fs::put(&path, bytes);
            let session = &mut web.app.session;
            let playhead = session.playhead();
            // the page opens the portal's copy; that is not a step of this link
            let policy = session.policy.take();
            let opened = session.execute("file.open", json!({"path": path}));
            let _ = session.execute("playhead.set", json!({"time": playhead.0}));
            session.policy = policy;
            let _ = session.take_executed();
            if let Err(e) = opened {
                notice(web, format!("Could not open the latest version: {e}"));
            }
            with_link(|l| l.seq = seq);
        });
    });
}

/// Refresh the Comments and Edit History panels from the portal.
fn refresh_lists() {
    let Some((token, version)) = with_link(|l| (l.token.clone(), l.version.clone())) else { return };
    spawn_local(async move {
        let actions = call("GET", &format!("/api/v/{version}/actions?since=0"), &token, None).await;
        let comments = call("GET", &format!("/api/v/{version}/comments"), &token, None).await;
        post(move |web, _| {
            let Some(r) = web.app.review.as_mut() else { return };
            if let Ok((200, v)) = actions {
                r.actions = v["actions"].as_array().map(|a| a.iter().map(action_row).collect()).unwrap_or_default();
            }
            if let Ok((200, v)) = comments {
                r.comments = v["comments"].as_array().map(|a| a.iter().map(comment_row).collect()).unwrap_or_default();
            }
        });
    });
}

fn local_time(iso: &str) -> String {
    let d = js_sys::Date::new(&JsValue::from_str(iso));
    if d.get_time().is_nan() {
        return iso.to_string();
    }
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"short".into());
    d.to_locale_string("default", &opts).as_string().unwrap_or_default()
}

fn ticks(v: &Value) -> i64 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_i64()).unwrap_or(0)
}

fn action_row(v: &Value) -> ActionRow {
    let actor = &v["actor"];
    let kind = actor["kind"].as_str().unwrap_or_default().to_string();
    let who = match kind.as_str() {
        "client" => format!("{} (client)", actor["name"].as_str().unwrap_or("Client")),
        "agent" => "Editor (agent)".to_string(),
        _ => actor["name"].as_str().unwrap_or("Someone").to_string(),
    };
    ActionRow {
        seq: v["seq"].as_u64().unwrap_or(0),
        at: local_time(v["at"].as_str().unwrap_or_default()),
        who,
        kind,
        label: v["label"].as_str().filter(|s| !s.is_empty()).or_else(|| v["id"].as_str()).unwrap_or_default().to_string(),
        command: v["id"].as_str().unwrap_or_default().to_string(),
        project_changed: v["project_changed"].as_bool().unwrap_or(false),
    }
}

fn comment_row(v: &Value) -> CommentRow {
    CommentRow {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        author: v["author"]["name"].as_str().or_else(|| v["author"].as_str()).unwrap_or("Someone").to_string(),
        start: ticks(&v["start_tick"]),
        end: ticks(&v["end_tick"]),
        body: v["body"].as_str().unwrap_or_default().to_string(),
        resolved: v["resolved"].as_bool().unwrap_or(false),
        fixed_in: v["fixed_in"].as_str().map(str::to_string),
        replies: v["replies"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| (r["author"]["name"].as_str().or_else(|| r["author"].as_str()).unwrap_or("Someone").to_string(), r["body"].as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Send one panel request (comment, reply, resolve, mark, status) and refresh the lists.
fn send_request(r: ReviewRequest) {
    let Some((token, version)) = with_link(|l| (l.token.clone(), l.version.clone())) else { return };
    let (url, body) = match r {
        ReviewRequest::Comment { start, end, body } => {
            (format!("/api/v/{version}/comments"), json!({"start_tick": start.to_string(), "end_tick": end.to_string(), "body": body}))
        }
        ReviewRequest::Reply { id, body } => (format!("/api/v/{version}/comments/{id}/replies"), json!({"body": body})),
        ReviewRequest::Resolve { id, resolved } => (format!("/api/v/{version}/comments/{id}/resolve"), json!({"resolved": resolved})),
        ReviewRequest::Mark { good, note } => (format!("/api/v/{version}/mark"), json!({"mark": if good { "good" } else { "not_good" }, "note": note})),
        ReviewRequest::Status(status) => (format!("/api/v/{version}/status"), json!({"status": status})),
    };
    spawn_local(async move {
        let result = call("POST", &url, &token, Some(&body)).await;
        post(move |web, _| {
            match result {
                Ok((200, _)) => {}
                Ok((status, v)) => notice(web, format!("{} (HTTP {status})", v["message"].as_str().unwrap_or("the portal refused that"))),
                Err(e) => notice(web, format!("The portal could not be reached ({e:?})")),
            }
            refresh_lists();
        });
    });
}
