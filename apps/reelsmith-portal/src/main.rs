//! `reelsmith-portal` serves the review portal. Configuration is environment variables only.
//! See `apps/reelsmith-portal/README.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() {
    if let Err(error) = reelsmith_portal::run() {
        eprintln!("reelsmith-portal: {error}");
        std::process::exit(1);
    }
}
