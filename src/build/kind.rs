//! What a repo builds: binary, library, Windows-only library or nothing (spec §6.4).

use crate::gitenv::GitEnv;
use crate::report::Log;
use anyhow::Result;
use cargo_metadata::Metadata;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildKind {
    /// A package named like the repo has a `bin` target of that name.
    Bin,
    /// Library targets, built with `--lib`.
    Lib,
    /// Every library package declares only `windows` docs.rs targets.
    WindowsLib,
    /// No binary or library targets.
    None,
}

/// `None` without a `Cargo.toml`, else [`read_metadata`] and [`classify`].
pub fn repo_build_kind(dir: &Path, name: &str, env: &GitEnv, log: &mut Log) -> Result<BuildKind> {
    if !dir.join("Cargo.toml").exists() {
        return Ok(BuildKind::None);
    }
    Ok(classify(&read_metadata(dir, env, log)?, name))
}

/// Run `cargo metadata --no-deps --format-version 1 --locked` in `dir` (all workspace members).
///
/// Uses `env.apply`; a nonzero exit is an error carrying cargo's stderr. Shared with release.
pub fn read_metadata(_dir: &Path, _env: &GitEnv, _log: &mut Log) -> Result<Metadata> {
    todo!("spec §6.4 cargo metadata")
}

/// Classify per the jq filter of spec §6.4.
///
/// `Bin` if a package named `name` has a target named `name` with kind `bin`.
/// Else, over packages with a `lib`/`rlib`/`proc-macro` target: none means `None`;
/// if every one has a non-empty `metadata.docs.rs.targets` whose entries all
/// contain `windows`, `WindowsLib`; else `Lib` (no docs.rs targets counts as `Lib`).
pub fn classify(_meta: &Metadata, _name: &str) -> BuildKind {
    todo!("spec §6.4 jq filter")
}
