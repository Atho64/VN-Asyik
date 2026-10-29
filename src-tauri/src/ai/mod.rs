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
//! # Removed: local embeddings
//!
//! A local `EmbeddingGemma` similarity index (own library plus a 198 MB VNDB
//! catalogue) was removed on 2026-10-22. It never had an in-app install or
//! indexing path, so it could not be set up without copying files by hand. The
//! code is preserved with a restore guide in `docs/archive/local-embeddings/`,
//! and the pre-removal tree is tagged `embedding-stack-before-removal-2026-10-22`.
//!
//! # Licensing
//!
//! Independently implemented. See the README attribution and licensing notes.

pub mod chat;
pub mod conversation;
pub mod profiles;
pub mod recommend;
pub mod tavily;
