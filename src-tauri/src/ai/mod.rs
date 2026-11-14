//! AI: recommendations and commentary.
//!
//! # Scope
//!
//! Two surfaces, both opt-in and both explicit about what they cost:
//!
//!   * **Suggestions** reach outward. Tavily searches the web and a chat model
//!     names games, which is the only way to recommend something the user does
//!     not already own. Those results carry citations for exactly that reason.
//!   * **Commentary** rephrases the computed taste profile, and falls back to the
//!     plain summary whenever it cannot.
//!
//! The taste profile itself is computed from VNDB tag ratings with plain
//! arithmetic: local, deterministic, and free.
//!
//! # Licensing
//!
//! Independently implemented. See the README attribution and licensing notes.

pub mod chat;
pub mod conversation;
pub mod profiles;
pub mod recommend;
pub mod tavily;

pub mod vision;
