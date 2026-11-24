//! Cargo git dependencies redirected to local clones or forks (spec §6.2, §6.5).

use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Where git dependencies come from; lives across all repos of a run.
#[derive(Debug, Default)]
pub struct Sources {
    /// Upstream `owner/name` to the clone in `dest_dir`.
    pub local: HashMap<String, PathBuf>,
    /// Upstream `owner/name` to our fork, for forks without a clone.
    pub fork: HashMap<String, String>,
    /// Upstreams already reported by the "no copy" note.
    pub reported: HashSet<String>,
}

/// Add the `insteadOf` rewrites to `env` over ALL star-list repos (spec §6.2) and print the notes.
///
/// A clone in `dest_dir` gets `file://` rewrites for the `.git` and plain URLs of its
/// upstream; a fork without a clone gets one rewrite to the fork.
pub fn setup_rewrites(
    items: &[RepoItem],
    dest_dir: &Path,
    env: &mut GitEnv,
    log: &mut Log,
) -> Sources {
    let mut sources = Sources::default();
    for item in items {
        let upstream = item.upstream();
        let clone = item.dest(dest_dir);
        if clone.join(".git").is_dir() {
            log.out(format!(
                "Fetching git dependencies on {upstream} from {}.",
                clone.display()
            ));
            let key = format!("url.file://{}.insteadOf", clone.display());
            env.push_config(&key, format!("https://github.com/{upstream}.git"));
            env.push_config(&key, format!("https://github.com/{upstream}"));
            sources.local.insert(upstream.to_owned(), clone);
        } else if item.is_fork {
            log.out(format!(
                "Fetching git dependencies on {upstream} from {}.",
                item.repo
            ));
            env.push_config(
                format!("url.https://github.com/{}.insteadOf", item.repo),
                format!("https://github.com/{upstream}"),
            );
            sources.fork.insert(upstream.to_owned(), item.repo.clone());
        }
    }
    sources
}

/// Make sure the commits pinned in `dir/Cargo.lock` exist in their redirected clones (spec §6.5).
///
/// No `Cargo.lock`: nothing. For each `source = "git+https://github.com/..#<40-hex>"` line
/// (regex verbatim in the spec), once per upstream/sha: if `sources.local` has the upstream
/// and `crate::git::has_commit` is false, queue the sha; else if no local or fork source and not
/// in `sources.reported`, print the `NOTE: no copy of ...` line once. Per clone with
/// missing shas: print `Fetching N pinned commit(s) into {dir}...`, then `crate::git::fetch_cli`.
/// Never modifies `Cargo.lock`.
pub fn prepare_git_deps(
    _sources: &mut Sources,
    _dir: &Path,
    _env: &GitEnv,
    _log: &mut Log,
) -> Result<()> {
    todo!("spec §6.5")
}
