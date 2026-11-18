//! Golden-image tests for ReelSmith's renderer (test-only crate; the library is empty).
//!
//! `tests/golden.rs` builds small procedural projects (no media files), renders one frame of each
//! through the CPU reference compositor (`reelsmith-render`) and compares it with a committed PNG
//! in `goldens/` using `reelsmith_testkit::golden`. The same scenes are composited on the GPU
//! (`reelsmith-gpu`) when an adapter is available and compared with the CPU result.
//!
//! Regenerate references with `REELSMITH_BLESS=1 cargo test -p reelsmith-golden`.
