//! Reading the current chapter out of a running engine, read-only.
//!
//! # What this is
//!
//! Some engines keep their whole scene table in memory: a pointer to the scene
//! manager, an index for the scene being played, and a blob of UTF-16 names. This
//! module contains the offsets for the builds that have been verified, and the
//! code that reads them.
//!
//! # What this is not
//!
//! It never writes to the game. Every access is a read of another process's
//! memory, the same operation a debugger performs, and nothing here changes
//! behaviour, patches files, or bypasses protection.
//!
//! # Why the offsets are version-gated
//!
//! An offset chain is a claim about one specific binary. Applying yesterday's
//! offsets to a patched build returns the wrong data silently, which is worse
//! than returning nothing. Every profile therefore names the build it was
//! verified against, and only a matching build is trusted; an unknown build
//! reports that per-chapter playtime is unavailable rather than guessing.

use serde::{Deserialize, Serialize};

/// Where a build keeps its scene table.
///
/// All values are offsets from the module's load address (its base), not file
/// offsets, because the engine is read while it is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneProfile {
    /// Engine family, matching `chapter::is_chapter`.
    pub engine: &'static str,
    /// The build this chain was verified against, as the binary reports it.
    pub verified_build: &'static str,
    /// Module that holds the scene manager.
    pub module: &'static str,
    /// Offset of the global holding the scene-manager pointer.
    pub manager_ptr: u32,
    /// Offset in the manager of the current scene index.
    pub scene_index: u32,
    /// Offset in the manager of the name pair table pointer.
    pub name_table: u32,
    /// Offset in the manager of the UTF-16 name blob pointer.
    pub name_blob: u32,
    /// Stride of one `{offset, length}` pair, in bytes.
    pub pair_stride: u32,
}

/// The profile verified against Summer Pockets REFLECTION BLUE.
///
/// Every offset below was confirmed by reading the live process and resolving
/// real scene names, not inferred from a signature:
///
/// * index 0 resolved to `00_シナリオフロー`
/// * index 1 resolved to `10_プロローグ0725`
/// * the scene playing at the title screen resolved to `_rb_titlemenu`
///
/// The resolver in the binary reads the pair table as `{u32 offset, u32 length}`
/// and the blob as UTF-16 at `blob + 2 * offset`.
pub const SIGLUS_SPRB: SceneProfile = SceneProfile {
    engine: "siglus",
    verified_build: "SiglusEngine.exe",
    module: "SiglusEngine.exe",
    manager_ptr: 0x7A_C2_BC,
    scene_index: 0x64,
    name_table: 0x2C,
    name_blob: 0x30,
    pair_stride: 8,
};

/// Every profile this build knows about.
pub const PROFILES: &[SceneProfile] = &[SIGLUS_SPRB];

/// Find the profile for an engine and module, if one exists.
#[must_use]
pub fn profile_for(engine: &str, module: &str) -> Option<&'static SceneProfile> {
    PROFILES.iter().find(|profile| {
        profile.engine.eq_ignore_ascii_case(engine) && profile.module.eq_ignore_ascii_case(module)
    })
}

/// What a chapter read produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterRead {
    /// A scene name was read. It may still be an interface screen; the caller
    /// interprets it with `chapter::interpret_siglus`.
    Scene(String),
    /// The engine is at no scene yet (still loading, or between scenes).
    NoScene,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verified_profile_is_found_by_engine_and_module() {
        let found = profile_for("siglus", "SiglusEngine.exe").expect("profile");
        assert_eq!(found.scene_index, 0x64);
        assert_eq!(found.manager_ptr, 0x7A_C2_BC);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(profile_for("SIGLUS", "siglusengine.EXE").is_some());
    }

    #[test]
    fn an_unknown_module_has_no_profile() {
        assert!(profile_for("siglus", "OtherGame.exe").is_none());
    }

    #[test]
    fn an_unknown_engine_has_no_profile() {
        assert!(profile_for("kirikiri", "SiglusEngine.exe").is_none());
    }

    #[test]
    fn every_profile_names_the_build_it_was_verified_against() {
        // A profile without provenance cannot be trusted, so it must not exist.
        for profile in PROFILES {
            assert!(
                !profile.verified_build.is_empty(),
                "{} has no verified build",
                profile.engine
            );
        }
    }
}
