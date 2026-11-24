//! The craft-fonts checkout some repos embed (spec §6.6, §6.7).

use crate::config::Config;
use crate::gitenv::GitEnv;
use crate::report::Log;
use anyhow::{Context, Result};
use std::path::Path;

/// The craft-fonts ref `dir` releases with, from `.github/workflows/release.yml`.
///
/// `Ok(None)` if the workflow is missing or has no ref (nothing is printed).
pub fn craft_fonts_ref(dir: &Path) -> Result<Option<String>> {
    let workflow = dir.join(".github/workflows/release.yml");
    if !workflow.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&workflow)
        .with_context(|| format!("failed to read {}", workflow.display()))?;
    resolve_ref(&text, &workflow)
}

/// Find and clean the craft-fonts ref in workflow `text` (spec §6.6, regexes and order verbatim).
///
/// `workflow` is only for messages. Errors: `{workflow} uses craft-fonts ref {ref}, but
/// defines no {var}.` and `can't resolve craft-fonts ref '{ref}' in {workflow}.`
pub fn resolve_ref(_text: &str, _workflow: &Path) -> Result<Option<String>> {
    todo!("spec §6.6")
}

/// Point `cfg.craft_fonts_cache` at `git_ref` of storytold/craft-fonts (spec §6.7).
///
/// Recreate the cache when it is not a git checkout, or `dest_dir/craft-fonts` is a clone
/// whose git common dir differs: a detached `git worktree add` of that clone (after
/// `worktree prune`), else `git clone` of https://github.com/storytold/craft-fonts.git.
/// Fetch the ref from origin unless it is a local 40-hex commit (then use `FETCH_HEAD`'s
/// sha), then `git -c advice.detachedHead=false checkout --quiet --force --detach`.
/// Uses the git CLI through `env.apply`; prints the messages of §6.7 and
/// `Using storytold/craft-fonts@{ref}`. The caller sets `CRAFT_FONTS_DIR`/`_REQUIRED`.
pub fn prepare_craft_fonts(
    _cfg: &Config,
    _env: &GitEnv,
    _git_ref: &str,
    _log: &mut Log,
) -> Result<()> {
    todo!("spec §6.7")
}
