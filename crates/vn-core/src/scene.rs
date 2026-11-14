//! Known engine profiles for capability reporting.
//!
//! Reports whether an engine family is known so capability status can distinguish
//! "known, unavailable" from "unknown". Profiles are static metadata only;
//! nothing here reads another process's memory.

/// Verified engine family metadata for capability reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineProfile {
    /// Engine family, matching `chapter::is_chapter`.
    pub engine: &'static str,
    /// Binary build identifier kept as provenance for the capability report.
    pub verified_build: &'static str,
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

pub const PROFILES: &[EngineProfile] = &[SIGLUS_SPRB];

/// Case-insensitive lookup by engine and module.
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
