// `lib.rs` is the crate root for shared code.
// Think of it like the top-level header/interface for this package: each
// `pub mod` exposes a submodule so other Rust code can refer to
// `triginta::app`, `triginta::storage`, and so on.
pub mod app;
pub mod config;
pub mod domain;
pub mod integrations;
pub mod storage;
pub mod task_nlp;
pub mod theme;
pub mod ui;
