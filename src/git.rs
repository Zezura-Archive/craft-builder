//! Git operations on local clones (git2; the git CLI only where noted).
//!
//! git2 ignores `GIT_CONFIG_*`, so URLs go through [`rewrite_url`] and
//! network operations use [`callbacks`].

use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use anyhow::Result;
use git2::{Cred, RemoteCallbacks};
use std::path::{Path, PathBuf};

/// Rewrite `git@github.com:x/y` and `ssh://git@github.com/x/y` to `https://github.com/x/y` (spec §2.5).
pub fn rewrite_url(url: &str) -> String {
    for prefix in ["git@github.com:", "ssh://git@github.com/"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return format!("https://github.com/{rest}");
        }
    }
    url.to_owned()
}

/// Credentials callback answering with `x-access-token` and the token.
pub fn callbacks(token: &str) -> RemoteCallbacks<'_> {
    let mut cb = RemoteCallbacks::new();
    cb.credentials(move |_url, _user, _allowed| Cred::userpass_plaintext("x-access-token", token));
    cb
}

/// `gh repo clone` equivalent: clone `https://github.com/{item.repo}.git` into `dest`.
///
/// For a fork, also add remote `upstream` = `https://github.com/{item.parent}.git`
/// and fetch it (restricted to the parent's default branch). Spec §4 step 3.
pub fn clone_repo(_item: &RepoItem, _dest: &Path, _token: &str, _log: &mut Log) -> Result<()> {
    todo!("spec §4 gh repo clone")
}

/// Bring local `branch` in `dest` up to date with `origin` (spec §4 `update_local`).
///
/// Fetch all remotes with prune; fetch tags from origin forced (`+refs/tags/*:refs/tags/*`, no prune).
/// If HEAD is on `branch`: `force` hard-resets to `origin/<branch>`, else fast-forward
/// only (error if diverged or blocked by local changes; no-op if up to date).
/// Otherwise update `refs/heads/<branch>` from origin without checkout (`+` when `force`,
/// so a non-ff update fails unless forced).
pub fn update_local(
    _dest: &Path,
    _branch: &str,
    _force: bool,
    _token: &str,
    _log: &mut Log,
) -> Result<()> {
    todo!("spec §4 update_local")
}

/// Short name of HEAD, or `None` if detached.
pub fn head_branch(_dir: &Path) -> Result<Option<String>> {
    todo!("git rev-parse --abbrev-ref HEAD")
}

/// Resolve a revision (`HEAD`, `origin/main`) to a full commit sha.
pub fn rev_parse(_dir: &Path, _spec: &str) -> Result<String> {
    todo!("git rev-parse")
}

/// Whether `dir` has the commit `sha` (`git cat-file -e sha^{commit}`).
pub fn has_commit(_dir: &Path, _sha: &str) -> bool {
    todo!("git cat-file -e")
}

/// Whether `path` is tracked in the index of `dir` (`git ls-files --error-unmatch`).
pub fn is_tracked(_dir: &Path, _path: &str) -> Result<bool> {
    todo!("git ls-files --error-unmatch")
}

/// Absolute git common dir of the repo at `dir` (`git rev-parse --git-common-dir`), `None` on error.
pub fn git_common_dir(_dir: &Path) -> Option<PathBuf> {
    todo!("git rev-parse --git-common-dir")
}

/// `git -C dir fetch --quiet origin <refs...>` with the git CLI (fetching raw shas needs it).
///
/// `refs` are shas, branches or tags; run through `env.apply`, output through `log`.
pub fn fetch_cli(_dir: &Path, _refs: &[&str], _env: &GitEnv, _log: &mut Log) -> Result<()> {
    todo!("git fetch --quiet origin <refs>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_ssh_urls_to_https() {
        assert_eq!(
            rewrite_url("git@github.com:o/r.git"),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            rewrite_url("ssh://git@github.com/o/r"),
            "https://github.com/o/r"
        );
        assert_eq!(
            rewrite_url("https://github.com/o/r"),
            "https://github.com/o/r"
        );
        assert_eq!(rewrite_url("git@gitlab.com:o/r"), "git@gitlab.com:o/r");
    }
}
