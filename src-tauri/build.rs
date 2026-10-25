//! Build script.
//!
//! `tauri-build` reads `DEP_TAURI_DEV` to define the `dev` cfg alias and panics
//! when the variable is missing. Cargo only provides it when `tauri` is listed as
//! a build-dependency of this crate. It is not one: `tauri` is the runtime we
//! already depend on normally, and adding it as a build-dependency duplicates the
//! whole dependency graph for the build script.
//!
//! The Tauri CLI sets the variable itself, so `tauri dev` and `tauri build` work.
//! A plain `cargo check`, `cargo clippy` or `cargo test` does not, which is what
//! CI and editors run. Rather than mutate this process's environment (which needs
//! `unsafe` and is denied workspace-wide), the script re-runs itself as a child
//! process with the variable set, then returns.
//!
//! The value mirrors `tauri`'s own build script: a build is a dev build unless
//! `custom-protocol` is enabled, which is exactly the CLI's production condition.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("DEP_TAURI_DEV").is_none() {
        let dev = !std::env::var("CARGO_FEATURE_CUSTOM_PROTOCOL").is_ok_and(|value| value == "1");
        let outer = std::env::current_exe().expect("build script path");
        let status = Command::new(outer)
            .env("DEP_TAURI_DEV", if dev { "true" } else { "false" })
            .status()
            .expect("re-run build script with DEP_TAURI_DEV set");
        // The child did the real work and printed the directives to our stdout,
        // which cargo captures from us. Its exit code decides the build result.
        match status.code() {
            Some(0) => return,
            Some(code) => std::process::exit(code),
            None => std::process::exit(1),
        }
    }

    tauri_build::build();
}
