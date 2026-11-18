//! HTTP shell. Tokens arrive on `Authorization: Bearer`, never in the query string, and are not logged.

use crate::brief::UploadBrief;
use crate::error::Error;
use crate::ops::EditBody;
use crate::service::{CompleteMeta, Portal, Role, StorageMode};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

pub struct Config {
    pub root: PathBuf,
    pub bind: String,
    pub admin: Option<String>,
    pub mode: StorageMode,
    pub max_upload: u64,
    pub quiet: bool,
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
        Ok(Self { root: PathBuf::from(root), bind, admin, mode, max_upload, quiet })
    }
}

pub type AppState = Arc<Mutex<Portal>>;

pub async fn serve(config: Config) -> Result<(), Error> {
    let quiet = config.quiet;
    let bind = config.bind.clone();
    let portal = Portal::open(config.root, config.admin, config.mode, config.max_upload)?;
    if !quiet {
        print_links(&bind, &portal);
    }
    let state = Arc::new(Mutex::new(portal));
    let app = router(state);
    let address: SocketAddr = bind.parse().map_err(|_| Error::invalid("PORTAL_BIND is not host:port"))?;
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|_| Error::invalid("could not bind the portal port"))?;
    axum::serve(listener, app).await.map_err(|_| Error::invalid("the portal server stopped"))?;
    Ok(())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/portal.css", get(css))
        .route("/assets/portal.js", get(js))
        .route("/api/health", get(health))
        .route("/api/admin/overview", get(overview))
        .route("/api/admin/clients", post(create_client))
        .route("/api/home", get(home))
        .route("/api/briefs/check", post(check_brief))
        .route("/api/projects", post(create_project))
        .route("/api/workspace", get(workspace))
        .route("/api/workspace/{version}/head", get(head))
        .route("/api/workspace/{version}/edits", post(edits))
        .route("/api/workspace/compare/{left}/{right}", get(compare))
        .route("/api/feedback", get(feedback))
        .route("/api/uploads", post(begin_upload))
        .route("/api/uploads/{id}", get(upload_status))
        .route("/api/uploads/{id}/complete", post(complete_upload))
        .route("/api/uploads/{id}/{part}", put(write_part))
        .route("/api/objects", get(read_object))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .with_state(state)
}

fn print_links(bind: &str, portal: &Portal) {
    let host = bind.replace("0.0.0.0", "127.0.0.1");
    eprintln!("ReelSmith Review Portal");
    eprintln!("  data: {}", portal.root().display());
    eprintln!("  open: http://{host}/");
    for (label, href) in portal.links() {
        eprintln!("  {label}: http://{host}/{href}");
    }
}

async fn index() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], include_str!("../static/portal.html")).into_response()
}

async fn css() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../static/portal.css")).into_response()
}

async fn js() -> Response {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../static/portal.js")).into_response()
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"ok": true, "service": "reelsmith-portal"}))
}

async fn overview(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.overview(&role)?).map_err(|_| Error::invalid("could not encode the overview"))?))
}

async fn create_client(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<NameBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.create_client(&role, &body.name)?).map_err(|_| Error::invalid("could not encode the client"))?))
}

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.home(&role)?).map_err(|_| Error::invalid("could not encode the home page"))?))
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
    Ok(Json(serde_json::to_value(portal.create_project(&role, brief)?).map_err(|_| Error::invalid("could not encode the project"))?))
}

async fn workspace(State(state): State<AppState>, headers: HeaderMap, Query(query): Query<VersionQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    let page = portal.workspace(&role, query.version.as_deref())?;
    Ok(Json(serde_json::to_value(page).map_err(|_| Error::invalid("could not encode the workspace"))?))
}

async fn head(State(state): State<AppState>, headers: HeaderMap, Path(version): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.head(&role, &version)?).map_err(|_| Error::invalid("could not encode the head"))?))
}

async fn edits(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(version): Path<String>,
    Json(body): Json<EditBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    let page = portal.edit(&role, &version, body)?;
    Ok(Json(serde_json::to_value(page).map_err(|_| Error::invalid("could not encode the workspace"))?))
}

async fn compare(State(state): State<AppState>, headers: HeaderMap, Path((left, right)): Path<(String, String)>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.compare(&role, &left, &right)?).map_err(|_| Error::invalid("could not encode the comparison"))?))
}

async fn feedback(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.feedback(&role)?).map_err(|_| Error::invalid("could not encode the feedback"))?))
}

async fn begin_upload(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<UploadIn>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(
        serde_json::to_value(portal.begin_upload(&role, &body.filename, &body.role, body.size)?).map_err(|_| Error::invalid("could not encode the upload"))?,
    ))
}

async fn upload_status(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.upload_status(&role, &id)?).map_err(|_| Error::invalid("could not encode the upload"))?))
}

async fn complete_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(meta): Json<CompleteMeta>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut portal = lock(&state);
    let role = auth(&portal, &headers)?;
    Ok(Json(serde_json::to_value(portal.complete_upload(&role, &id, meta)?).map_err(|_| Error::invalid("could not encode the upload"))?))
}

async fn write_part(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, part)): Path<(String, u32)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    let manifest = portal.write_part(&role, &id, part, &body)?;
    Ok(Json(serde_json::to_value(manifest).map_err(|_| Error::invalid("could not encode the upload"))?))
}

async fn read_object(State(state): State<AppState>, headers: HeaderMap, Query(query): Query<ObjectQuery>) -> Result<Response, ApiError> {
    let portal = lock(&state);
    let role = auth(&portal, &headers)?;
    let bytes = portal.read_object(&role, &query.key)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

fn auth(portal: &Portal, headers: &HeaderMap) -> Result<Role, Error> {
    let header = headers.get(header::AUTHORIZATION).ok_or_else(|| Error::forbidden("sign in with a project link"))?;
    let value = header.to_str().map_err(|_| Error::forbidden("the authorization header is not text"))?;
    let token = value.strip_prefix("Bearer ").ok_or_else(|| Error::forbidden("authorization must be a bearer token"))?;
    portal.authorize(token.trim())
}

fn lock(state: &AppState) -> MutexGuard<'_, Portal> {
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
        (status, Json(json!({"message": self.0.message}))).into_response()
    }
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<String>,
}

#[derive(Deserialize)]
struct UploadIn {
    filename: String,
    role: String,
    size: u64,
}

#[derive(Deserialize)]
struct ObjectQuery {
    key: String,
}
