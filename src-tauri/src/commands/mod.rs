//! Tauri command handlers, one module per group (DESIGN §12.1). Every
//! command returns `Result<T, CommandError>` and does its real work in
//! `repo-radar-core`; this layer only adapts types and owns state.

pub mod repos;
pub mod settings;
pub mod system;
