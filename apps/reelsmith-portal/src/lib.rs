//! ReelSmith review portal.
//!
//! Clients upload raw footage, review a timeline, and edit it directly. Every step is an
//! append-only event. The browser UI is a client of this crate; the edit log, the upload rules
//! and the storage adapter are the part that is tested.
//!
//! `apps/reelsmith-web` is the full egui editor compiled to WebAssembly. It is not embedded
//! here. See `docs/portal/DESIGN.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod assemble;
pub mod brief;
pub mod edit;
pub mod error;
pub mod history;
pub mod http;
pub mod ids;
pub mod model;
pub mod ops;
pub mod persist;
pub mod present;
pub mod r2;
pub mod seed;
pub mod service;
pub mod storage;

pub use error::Error;
pub use http::{Config, serve};

pub fn run() -> Result<(), Error> {
    let config = Config::from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|_| Error::invalid("could not start the runtime"))?;
    runtime.block_on(serve(config))
}
