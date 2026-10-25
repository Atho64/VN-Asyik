//! Database layer: connection pool, migrations and typed queries.
//!
//! All `SQL` lives in Rust, behind Tauri commands. The frontend never talks to
//! `SQLite` directly (AGENTS.md §3), so tracking and sync rules cannot be bypassed.

pub mod models;
pub mod pool;
pub mod queries;

pub use pool::{Db, DbError};
