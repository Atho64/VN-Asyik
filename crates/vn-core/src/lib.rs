//! Pure domain logic for VN Asyik.
//!
//! Nothing in this crate may depend on Tauri, the OS, the filesystem, or the
//! network. That keeps the rules that matter most (identity, time accounting,
//! merge order) testable at full speed and reusable outside the app.

pub mod adapter;
pub mod afk;
pub mod chapter;
pub mod chat;
pub mod clock;
pub mod companion;
pub mod emulator;
pub mod ids;
pub mod journal;
pub mod library;
pub mod locale_emulator;
pub mod migration;
pub mod model;
pub mod presence;
pub mod reading;
pub mod scan_rank;
pub mod scene;
pub mod script;
pub mod search;
pub mod stats;
pub mod sync;
pub mod taste;
pub mod time_fmt;
pub mod title_settings;
pub mod tracker;
pub mod usage;
pub mod walkthrough;
pub mod window_title;

mod width;

pub use clock::Timestamp;
pub use ids::{DeviceId, EntityId};
