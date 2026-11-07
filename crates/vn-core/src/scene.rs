//! Known engine profiles for capability reporting.
//!
//! # What this is
//!
//! A small table answering one question: is this engine family one this build
//! has a verified profile for, so capability reporting can say "known,
//! unavailable" instead of "unknown".
//!
//! # What this is not
//!
//! The in-process reader that used these scene tables was removed with the
//! hook. Reading a scene table means reading another process's memory at
//! offsets derived from injected byte patterns, and that is exactly the
//! mechanism that was removed, so the offsets themselves are absent from
//! code. Nothing here touches a game process; a profile is metadata, not an
//! access path.

/// An engine family this build has a verified profile for.
///
/// Every profile names the build it was verified against, as the binary
/// reports it: a claim about one specific binary, kept as provenance for the
/// capability report rather than as an access path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineProfile {
    /// Engine family, matching `chapter::is_chapter`.
    pub engine: &'static str,
    /// The build this profile was verified against, as the binary reports it.
    pub verified_build: &'static str,
    /// The engine module the profile was verified against.
    pub module: &'static str,
}

/// The profile verified against Summer Pockets REFLECTION BLUE.
///
/// The scene names this build once resolved live (`00_シナリオフロー`,
/// `10_プロローグ0725`, `_rb_titlemenu`) were confirmed against this engine
/// family and module.
pub const SIGLUS_SPRB: EngineProfile = EngineProfile {
    engine: "siglus",
    verified_build: "SiglusEngine.exe",
    module: "SiglusEngine.exe",
};

/// Every profile this build knows about.
pub const PROFILES: &[EngineProfile] = &[SIGLUS_SPRB];

/// Find the profile for an engine and module, if one exists.
#[must_use]
pub fn profile_for(engine: &str, module: &str) -> Option<&'static EngineProfile> {
    PROFILES.iter().find(|profile| {
        profile.engine.eq_ignore_ascii_case(engine) && profile.module.eq_ignore_ascii_case(module)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verified_profile_is_found_by_engine_and_module() {
        assert!(profile_for("siglus", "SiglusEngine.exe").is_some());
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
