//! ReelSmith review portal.
//!
//! Clients upload raw footage and review cuts in the real editor: the portal serves the wasm
//! editor (`apps/reelsmith-web`) on editor, review and watch links and is the single writer of
//! each version's cut. Every edit is an engine command the page posts; the server checks the
//! link's role (`reelsmith_engine::review`), replays it on its own headless engine session, and
//! appends it to the version's append-only action log. See PLAN.md / API.md and
//! `docs/portal/DESIGN.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod brief;
pub mod error;
pub mod feedback;
pub mod history;
pub mod http;
pub mod ids;
pub mod model;
pub mod persist;
pub mod r2;
pub mod replay;
pub mod service;
pub mod storage;

pub use error::Error;
pub use http::{Config, router, serve};

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|_| Error::invalid("could not start the runtime"))?;
    runtime.block_on(serve(config))
}
