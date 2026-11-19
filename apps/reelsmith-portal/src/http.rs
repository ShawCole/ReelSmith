//! HTTP shell (API.md). Tokens arrive on `Authorization: Bearer`, never in a path or query, and
//! are not logged. Every response is cross-origin isolated (COOP/COEP/CORP), so the wasm editor
//! served under `/edit/`, `/review/` and `/watch/` runs like `cargo xtask web --serve`.

use crate::brief::UploadBrief;
use crate::error::Error;
use crate::service::{ActionsIn, CommentIn, CompleteMeta, MarkIn, Portal, ReplyIn, ResolveIn, Role, StatusIn, StorageMode};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

/// Largest request body: one 8 MiB upload part plus headroom.
pub const BODY_LIMIT: usize = 9 * 1024 * 1024;

pub struct Config {
    pub root: PathBuf,
    pub bind: String,
    pub admin: Option<String>,
    pub mode: StorageMode,
    pub max_upload: u64,
    pub quiet: bool,
    /// The wasm editor's `dist/` (`cargo xtask web`).
    pub editor_dist: PathBuf,
}

/// `<repo>/target/web/dist`, where `cargo xtask web` writes the editor.
pub fn default_editor_dist() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/web/dist"))
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        let root = std::env::var("PORTAL_DATA_DIR").unwrap_or_else(|_| ".portal-data".to_string());
        let bind = std::env::var("PORTAL_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
        let admin = std::env::var("PORTAL_ADMIN_TOKEN").ok().filter(|value| !value.trim().is_empty());
        let max_upload = match std::env::var("PORTAL_MAX_UPLOAD_BYTES") {
            Ok(value) => value.parse::<u64>().map_err(|_| Error::invalid("PORTAL_MAX_UPLOAD_BYTES is not a number"))?,
            Err(_) => 8 * 1024 * 1024 * 1024,
        };
        let mode = match std::env::var("PORTAL_STORAGE").unwrap_or_else(|_| "local".to_string()).to_ascii_lowercase().as_str() {
            "local" | "" => StorageMode::Local,
            "r2" => StorageMode::R2(crate::r2::R2Config::from_env()?),
            other => return Err(Error::invalid(format!("PORTAL_STORAGE must be local or r2, got {other}"))),
        };
        let quiet = std::env::var("PORTAL_QUIET").ok().is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
        let editor_dist = std::env::var("PORTAL_EDITOR_DIST").ok().filter(|v| !v.trim().is_empty()).map(PathBuf::from).unwrap_or_else(default_editor_dist);
        Ok(Self { root: PathBuf::from(root), bind, admin, mode, max_upload, quiet, editor_dist })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub portal: Arc<Mutex<Portal>>,
    pub editor_dist: Arc<PathBuf>,
}

pub async fn serve(config: Config) -> Result<(), Error> {
    let quiet = config.quiet;
    let bind = config.bind.clone();
    let portal = Portal::open(config.root, config.admin, config.mode, config.max_upload)?;
    if !quiet {
        print_links(&bind, &portal, &config.editor_dist);
    }
    let app = router(portal, config.editor_dist);
    let address: SocketAddr = bind.parse().map_err(|_| Error::invalid("PORTAL_BIND is not host:port"))?;
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|_| Error::invalid("could not bind the portal port"))?;
    axum::serve(listener, app).await.map_err(|_| Error::invalid("the portal server stopped"))?;
    Ok(())
}

pub fn router(portal: Portal, editor_dist: PathBuf) -> Router {
    let state = AppState { portal: Arc::new(Mutex::new(portal)), editor_dist: Arc::new(editor_dist) };
    Router::new()
        .route("/", get(home_page))
        .route("/assets/home.js", get(home_js))
        .route("/assets/home.css", get(home_css))
        .route("/edit", get(|| async { Redirect::permanent("/edit/") }))
        .route("/review", get(|| async { Redirect::permanent("/review/") }))
        .route("/watch", get(|| async { Redirect::permanent("/watch/") }))
        .route("/edit/", get(editor_index))
        .route("/review/", get(editor_index))
        .route("/watch/", get(editor_index))
        .route("/edit/{*file}", get(editor_file))
        .route("/review/{*file}", get(editor_file))
        .route("/watch/{*file}", get(editor_file))
        .route("/api/health", get(health))
        .route("/api/boot", get(boot))
        .route("/api/v/{version}/project", get(project_file))
        .route("/api/v/{version}/head", get(head))
        .route("/api/v/{version}/actions", get(actions_since).post(post_actions))
        .route("/api/v/{version}/media/{n}", get(media))
        .route("/api/v/{version}/comments", get(comments).post(add_comment))
        .route("/api/v/{version}/comments/{id}/replies", post(reply))
        .route("/api/v/{version}/comments/{id}/resolve", post(resolve))
        .route("/api/v/{version}/mark", post(mark))
        .route("/api/v/{version}/status", post(set_status))
        .route("/api/feedback", get(feedback))
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/clients", post(create_client))
        .route("/api/home", get(home))
        .route("/api/briefs/check", post(check_brief))
        .route("/api/projects", post(create_project))
        .route("/api/uploads", post(begin_upload))
        .route("/api/uploads/{id}", get(upload_status))
        .route("/api/uploads/{id}/complete", post(complete_upload))
        .route("/api/uploads/{id}/{part}", put(write_part))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(axum::middleware::map_response(isolate))
        .with_state(state)
}

/// COOP + COEP + CORP on every response (API.md).
async fn isolate(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    headers.insert("cross-origin-embedder-policy", HeaderValue::from_static("require-corp"));
    headers.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    response
}

fn print_links(bind: &str, portal: &Portal, dist: &std::path::Path) {
    let host = bind.replace("0.0.0.0", "127.0.0.1");
    eprintln!("ReelSmith Review Portal");
    eprintln!("  data: {}", portal.root().display());
    eprintln!("  editor: {}{}", dist.display(), if dist.join("index.html").exists() { "" } else { " (missing: run `cargo xtask web`)" });
    for (label, href) in portal.links() {
        eprintln!("  {label}: http://{host}{href}");
    }
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"message": "not found"}))).into_response()
}

async fn home_page() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], include_str!("../static/home.html")).into_response()
}

async fn home_js() -> Response {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../static/home.js")).into_response()
}

async fn home_css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../static/home.css")).into_response()
}

async fn editor_index(State(state): State<AppState>) -> Response {
    serve_dist(&state, "index.html").await
}

async fn editor_file(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    if file.is_empty() {
        return serve_dist(&state, "index.html").await;
    }
    serve_dist(&state, &file).await
}

/// A file of the editor's `dist/`. Only plain relative paths inside it.
async fn serve_dist(state: &AppState, file: &str) -> Response {
    let safe = !file.is_empty()
        && file.len() <= 256
        && file.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.starts_with('.'))
        && file.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'));
    if !safe {
        return not_found().await;
    }
    let path = state.editor_dist.join(file);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let cache = if file == "index.html" { "no-cache" } else { "no-cache, must-revalidate" };
            ([(header::CONTENT_TYPE, content_type(file)), (header::CACHE_CONTROL, cache)], bytes).into_response()
        }
        Err(_) if file == "index.html" => {
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"message": "the editor is not built; run `cargo xtask web`"}))).into_response()
        }
        Err(_) => not_found().await,
    }
}

pub fn content_type(file: &str) -> &'static str {
    let ext = file.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "wasm" => "application/wasm",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"ok": true, "service": "reelsmith-portal"}))
}

fn json_of<T: Serialize>(value: T) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::to_value(value).map_err(|_| Error::invalid("could not encode the answer"))?))
}

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<String>,
}

#[derive(Deserialize)]
struct SinceQuery {
    #[serde(default)]
    since: u64,
}

async fn boot(State(state): State<AppState>, headers: HeaderMap, Query(query): Query<VersionQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.boot(&role, query.version.as_deref())?)
}

async fn project_file(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>) -> Result<Response, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    let (bytes, seq) = portal.project_file(&role, &version)?;
    let seq = HeaderValue::from_str(&seq.to_string()).map_err(|_| Error::invalid("bad seq"))?;
    Ok(([(header::CONTENT_TYPE, HeaderValue::from_static("application/json")), (header::HeaderName::from_static("x-portal-seq"), seq)], bytes).into_response())
}

async fn head(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.head(&role, &version)?)
}

async fn actions_since(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version): Path<String>,
    Query(query): Query<SinceQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.actions_since(&role, &version, query.since)?)
}

async fn post_actions(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>, Json(body): Json<ActionsIn>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.post_actions(&role, &version, body)?)
}

async fn media(State(state): State<AppState>, headers: HeaderMap, Path((version, n)): Path<(String, usize)>) -> Result<Response, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    let bytes = portal.media(&role, &version, n)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

async fn comments(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(json!({"comments": portal.comments(&role, &version)?})))
}

async fn add_comment(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>, Json(body): Json<CommentIn>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.add_comment(&role, &version, body)?)
}

async fn reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((version, id)): Path<(String, String)>,
    Json(body): Json<ReplyIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.reply(&role, &version, &id, body)?)
}

async fn resolve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((version, id)): Path<(String, String)>,
    Json(body): Json<ResolveIn>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.resolve(&role, &version, &id, body)?)
}

async fn mark(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>, Json(body): Json<MarkIn>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.mark(&role, &version, body)?)
}

async fn set_status(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>, Json(body): Json<StatusIn>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.set_status(&role, &version, body)?)
}

async fn feedback(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.feedback(&role)?)
}

async fn overview(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.overview(&role)?)
}

async fn create_client(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<NameBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.create_client(&role, &body.name)?)
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.home(&role)?)
}

async fn check_brief(State(state): State<AppState>, headers: HeaderMap, Json(brief): Json<UploadBrief>) -> Result<Response, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    match portal.check_brief(&role, &brief) {
        Ok(plan) => Ok(Json(json!({"ok": true, "plan": plan})).into_response()),
        Err(errors) => Ok((StatusCode::BAD_REQUEST, Json(json!({"ok": false, "errors": errors}))).into_response()),
    }
}

async fn create_project(State(state): State<AppState>, headers: HeaderMap, Json(brief): Json<UploadBrief>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.create_project(&role, brief)?)
}

async fn begin_upload(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<UploadIn>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.begin_upload(&role, &body.filename, &body.role, body.size)?)
}

async fn upload_status(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.upload_status(&role, &id)?)
}

async fn complete_upload(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, Json(meta): Json<CompleteMeta>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.complete_upload(&role, &id, meta)?)
}

async fn write_part(State(state): State<AppState>, headers: HeaderMap, Path((id, part)): Path<(String, u32)>, body: Bytes) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    json_of(portal.write_part(&role, &id, part, &body)?)
}

fn auth(portal: &Portal, headers: &HeaderMap) -> Result<Role, Error> {
    let header = headers.get(header::AUTHORIZATION).ok_or_else(|| Error::forbidden("sign in with a project link"))?;
    let value = header.to_str().map_err(|_| Error::forbidden("the authorization header is not text"))?;
    let token = value.strip_prefix("Bearer ").ok_or_else(|| Error::forbidden("authorization must be a bearer token"))?;
    portal.authorize(token.trim())
}

fn lock(state: &AppState) -> MutexGuard<'_, Portal> {
    state.portal.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status()).unwrap_or(StatusCode::BAD_REQUEST);
        let body = match self.0.seq {
            Some(seq) => json!({"message": self.0.message, "seq": seq}),
            None => json!({"message": self.0.message}),
        };
        (status, Json(body)).into_response()
    }
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

#[derive(Deserialize)]
struct UploadIn {
    filename: String,
    role: String,
    size: u64,
}
