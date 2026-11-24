//! Phase 2: build one synced repo for Linux and Windows (spec §6).

pub mod deps;
pub mod fonts;
pub mod kind;
pub mod target;

use crate::config::Config;
use crate::git;
use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use deps::Sources;
use kind::BuildKind;
use target::{LINUX, WINDOWS};

/// Result of [`build_repo`].
#[derive(Debug, Default)]
pub struct BuildOutcome {
    /// Failure entries to record, e.g. `owner/name (build: linux-x86_64)`.
    pub failures: Vec<String>,
    /// Fully built binary repo, to be released in phase 3.
    pub releasable: bool,
}

/// Spec §6.3 steps 1-8. `env` is the shared build env (with git rewrites from
/// [`deps::setup_rewrites`]); it is cloned per repo. The `==== build ====` header is the caller's.
pub fn build_repo(
    cfg: &Config,
    env: &GitEnv,
    sources: &mut Sources,
    item: &RepoItem,
    log: &mut Log,
) -> BuildOutcome {
    let (repo, branch, name) = (&item.repo, &item.branch, item.name());
    let dir = item.dest(&cfg.dest_dir);
    let mut out = BuildOutcome::default();
    let fail =
        |out: &mut BuildOutcome, what: &str| out.failures.push(format!("{repo} (build: {what})"));

    if !matches!(git::head_branch(&dir), Ok(Some(head)) if head == *branch) {
        log.err(format!(
            "WARNING: {} is not on {branch}; not building it.",
            dir.display()
        ));
        fail(&mut out, &format!("working tree not on {branch}"));
        return out;
    }

    let kind = match kind::repo_build_kind(&dir, name, env, log) {
        Ok(kind) => kind,
        Err(err) => {
            log.err(format!("Error: {err:#}"));
            log.err(format!(
                "WARNING: failed to read the cargo metadata of {}; not building it.",
                dir.display()
            ));
            fail(&mut out, "cargo metadata");
            return out;
        }
    };
    if kind == BuildKind::None {
        log.out(format!(
            "Not building {repo}: no binary or library targets (sync only)."
        ));
        return out;
    }

    let mut env = env.clone();
    match fonts::craft_fonts_ref(&dir) {
        Err(err) => {
            log.err(format!("Error: {err:#}"));
            log.err(format!(
                "WARNING: can't tell which storytold/craft-fonts commit {repo} releases with; not building it."
            ));
            fail(&mut out, "craft-fonts ref");
            return out;
        }
        Ok(None) => {}
        Ok(Some(git_ref)) => {
            if let Err(err) = fonts::prepare_craft_fonts(cfg, &env, &git_ref, log) {
                log.err(format!("Error: {err:#}"));
                log.err(format!("WARNING: failed to prepare storytold/craft-fonts@{git_ref}; not building {repo}."));
                fail(&mut out, &format!("craft-fonts {git_ref}"));
                return out;
            }
            env.set("CRAFT_FONTS_DIR", cfg.craft_fonts_cache.to_string_lossy());
            env.set("CRAFT_FONTS_REQUIRED", "1");
        }
    }

    if let Err(err) = deps::prepare_git_deps(sources, &dir, &env, log) {
        log.err(format!("Error: {err:#}"));
        log.err(format!(
            "WARNING: failed to prepare the git dependencies {repo} pins; not building it."
        ));
        fail(&mut out, "git dependencies");
        return out;
    }

    let mut built = true;
    if kind == BuildKind::WindowsLib {
        log.out(format!(
            "Not building {repo} for Linux: it only declares Windows targets."
        ));
    } else if let Err(err) = target::build_target(cfg, &env, kind, &dir, name, &LINUX, log) {
        log.err(format!("Error: {err:#}"));
        log.err(format!("WARNING: Linux build of {repo} failed"));
        fail(&mut out, LINUX.label);
        built = false;
    }
    if let Err(err) = target::build_target(cfg, &env, kind, &dir, name, &WINDOWS, log) {
        log.err(format!("Error: {err:#}"));
        log.err(format!("WARNING: Windows build of {repo} failed"));
        fail(&mut out, WINDOWS.label);
        built = false;
    }
    out.releasable = built && kind == BuildKind::Bin;
    out
}
