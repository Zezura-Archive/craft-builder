//! Git operations on local clones (git2; the git CLI only where noted).
//!
//! git2 ignores `GIT_CONFIG_*`, so URLs go through [`rewrite_url`] and
//! network operations use [`callbacks`].

use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use anyhow::{Context, Result, bail, ensure};
use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::{
    AutotagOption, Commit, Cred, FetchOptions, FetchPrune, Oid, RemoteCallbacks, Repository,
    ResetType,
};
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let mut tried = false;
    cb.credentials(move |url, _user, _allowed| {
        if tried || !url.starts_with("https://github.com/") {
            return Err(git2::Error::from_str(&format!(
                "authentication failed for {url}"
            )));
        }
        tried = true;
        Cred::userpass_plaintext("x-access-token", token)
    });
    cb
}

/// `gh repo clone` equivalent: clone `https://github.com/{item.repo}.git` into `dest`.
///
/// For a fork, also add remote `upstream` = `https://github.com/{item.parent}.git`
/// and fetch it (restricted to the parent's default branch). Spec §4 step 3.
pub fn clone_repo(item: &RepoItem, dest: &Path, token: &str, _log: &mut Log) -> Result<()> {
    let url = format!("https://github.com/{}.git", item.repo);
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(token));
    let repo = RepoBuilder::new()
        .fetch_options(fetch)
        .clone(&url, dest)
        .with_context(|| format!("failed to clone {url}"))?;

    if item.is_fork {
        let branch = &item.branch;
        let refspec = format!("+refs/heads/{branch}:refs/remotes/upstream/{branch}");
        let parent_url = format!("https://github.com/{}.git", item.parent);
        let mut upstream = repo.remote_with_fetch("upstream", &parent_url, &refspec)?;
        let mut fetch = FetchOptions::new();
        fetch.remote_callbacks(callbacks(token));
        upstream
            .fetch::<&str>(&[], Some(&mut fetch), None)
            .with_context(|| format!("failed to fetch {parent_url}"))?;
    }
    Ok(())
}

/// Bring local `branch` in `dest` up to date with `origin` (spec §4 `update_local`).
///
/// Fetch all remotes with prune; fetch tags from origin forced (`+refs/tags/*:refs/tags/*`, no prune).
/// If HEAD is on `branch`: `force` hard-resets to `origin/<branch>`, else fast-forward
/// only (error if diverged or blocked by local changes; no-op if up to date).
/// Otherwise update `refs/heads/<branch>` from origin without checkout (ff-only unless `force`).
pub fn update_local(
    dest: &Path,
    branch: &str,
    force: bool,
    token: &str,
    _log: &mut Log,
) -> Result<()> {
    let repo = Repository::open(dest)?;
    fetch_all(&repo, token)?;
    fetch_tags(&repo, token)?;

    let target = repo
        .revparse_single(&format!("refs/remotes/origin/{branch}"))
        .with_context(|| format!("origin/{branch} not found"))?
        .peel_to_commit()?;

    if head_branch(dest)?.as_deref() == Some(branch) {
        if force {
            repo.reset(target.as_object(), ResetType::Hard, None)?;
        } else {
            fast_forward_checked_out(&repo, &target)?;
        }
    } else {
        update_branch_ref(&repo, branch, &target, force)?;
    }
    Ok(())
}

/// `git fetch --all --prune`: each remote with its configured refspecs.
fn fetch_all(repo: &Repository, token: &str) -> Result<()> {
    for name in repo.remotes()?.iter().flatten().flatten() {
        let remote = repo.find_remote(name)?;
        let Ok(url) = remote.url() else { continue };
        let refspecs: Vec<String> = remote
            .fetch_refspecs()?
            .iter()
            .flatten()
            .flatten()
            .map(str::to_owned)
            .collect();
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(callbacks(token));
        opts.prune(FetchPrune::On);
        repo.remote_anonymous(&rewrite_url(url))?
            .fetch(&refspecs, Some(&mut opts), None)
            .with_context(|| format!("failed to fetch {name}"))?;
    }
    Ok(())
}

/// `git fetch --tags --force origin`: no pruning.
fn fetch_tags(repo: &Repository, token: &str) -> Result<()> {
    let origin = repo.find_remote("origin")?;
    let url = origin.url().context("origin has no URL")?;
    let mut opts = FetchOptions::new();
    opts.remote_callbacks(callbacks(token));
    opts.download_tags(AutotagOption::None);
    repo.remote_anonymous(&rewrite_url(url))?
        .fetch(&["+refs/tags/*:refs/tags/*"], Some(&mut opts), None)
        .context("failed to fetch tags from origin")
}

/// `git merge --ff-only`: move the checked-out branch and its worktree to `target`.
fn fast_forward_checked_out(repo: &Repository, target: &Commit) -> Result<()> {
    let head = repo.head()?.peel_to_commit()?.id();
    if head == target.id() || repo.graph_descendant_of(head, target.id())? {
        return Ok(());
    }
    if !repo.graph_descendant_of(target.id(), head)? {
        bail!("Not possible to fast-forward, aborting.");
    }
    repo.checkout_tree(target.as_object(), Some(CheckoutBuilder::new().safe()))
        .context("local changes would be overwritten by the fast-forward")?;
    repo.head()?
        .set_target(target.id(), "merge: fast-forward")?;
    Ok(())
}

/// `git fetch origin [+]refs/heads/B:refs/heads/B` without a checkout.
fn update_branch_ref(repo: &Repository, branch: &str, target: &Commit, force: bool) -> Result<()> {
    let name = format!("refs/heads/{branch}");
    if let Ok(existing) = repo.find_reference(&name) {
        let current = existing.peel_to_commit()?.id();
        if current == target.id() {
            return Ok(());
        }
        if !force && !repo.graph_descendant_of(target.id(), current)? {
            bail!("{branch} cannot be fast-forwarded to origin/{branch} (non-fast-forward)");
        }
    }
    repo.reference(&name, target.id(), true, "fetch: update branch")?;
    Ok(())
}

/// Short name of HEAD, or `None` if detached.
pub fn head_branch(dir: &Path) -> Result<Option<String>> {
    let repo = Repository::open(dir)?;
    if repo.head_detached()? {
        return Ok(None);
    }
    Ok(Some(repo.head()?.shorthand()?.to_owned()))
}

/// Resolve a revision (`HEAD`, `origin/main`) to a full commit sha.
pub fn rev_parse(dir: &Path, spec: &str) -> Result<String> {
    let repo = Repository::open(dir)?;
    let commit = repo.revparse_single(spec)?.peel_to_commit()?;
    Ok(commit.id().to_string())
}

/// Whether `dir` has the commit `sha` (`git cat-file -e sha^{commit}`).
pub fn has_commit(dir: &Path, sha: &str) -> bool {
    let Ok(repo) = Repository::open(dir) else {
        return false;
    };
    Oid::from_str(sha).is_ok_and(|oid| repo.find_commit(oid).is_ok())
}

/// Whether `path` is tracked in the index of `dir` (`git ls-files --error-unmatch`).
pub fn is_tracked(dir: &Path, path: &str) -> Result<bool> {
    let index = Repository::open(dir)?.index()?;
    Ok(index.get_path(Path::new(path), 0).is_some())
}

/// Absolute git common dir of the repo at `dir` (`git rev-parse --git-common-dir`), `None` on error.
pub fn git_common_dir(dir: &Path) -> Option<PathBuf> {
    let repo = Repository::open(dir).ok()?;
    repo.commondir().canonicalize().ok()
}

/// `git -C dir fetch --quiet origin <refs...>` with the git CLI (fetching raw shas needs it).
///
/// `refs` are shas, branches or tags; run through `env.apply`, output through `log`.
pub fn fetch_cli(dir: &Path, refs: &[&str], env: &GitEnv, log: &mut Log) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(["fetch", "--quiet", "--", "origin"])
        .args(refs);
    env.apply(&mut cmd);
    let status = log.run(&mut cmd).context("failed to run git")?;
    ensure!(status.success(), "git fetch failed ({status})");
    Ok(())
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

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "craft-builder-git-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn commit(dir: &Path, file: &str, content: &str) {
        std::fs::write(dir.join(file), content).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", content]);
    }

    /// An `origin` repo with one commit on `main`, and a clone of it.
    fn origin_and_clone() -> (PathBuf, PathBuf) {
        let root = temp_dir();
        let origin = root.join("origin");
        std::fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-q"]);
        commit(&origin, "a", "one");
        let clone = root.join("clone");
        git(&root, &["clone", "-q", "origin", "clone"]);
        (origin, clone)
    }

    fn update(clone: &Path, branch: &str, force: bool) -> Result<()> {
        update_local(clone, branch, force, "tok", &mut Log::buffered())
    }

    #[test]
    fn fast_forwards_checked_out_branch() {
        let (origin, clone) = origin_and_clone();
        commit(&origin, "b", "two");
        update(&clone, "main", false).unwrap();
        assert_eq!(
            rev_parse(&clone, "HEAD").unwrap(),
            rev_parse(&origin, "HEAD").unwrap()
        );
        assert!(clone.join("b").exists());
        update(&clone, "main", false).unwrap();
    }

    #[test]
    fn updates_branch_that_is_not_checked_out() {
        let (origin, clone) = origin_and_clone();
        git(&origin, &["branch", "dev"]);
        git(&origin, &["checkout", "-q", "dev"]);
        commit(&origin, "b", "two");
        update(&clone, "dev", false).unwrap();
        assert_eq!(
            rev_parse(&clone, "dev").unwrap(),
            rev_parse(&origin, "dev").unwrap()
        );
        assert_eq!(head_branch(&clone).unwrap().as_deref(), Some("main"));
        commit(&origin, "c", "three");
        update(&clone, "dev", false).unwrap();
        assert_eq!(
            rev_parse(&clone, "dev").unwrap(),
            rev_parse(&origin, "dev").unwrap()
        );
    }

    #[test]
    fn diverged_checked_out_branch_fails_without_force_and_resets_with_it() {
        let (origin, clone) = origin_and_clone();
        commit(&clone, "local", "local");
        commit(&origin, "b", "remote");
        assert!(update(&clone, "main", false).is_err());
        assert!(clone.join("local").exists());
        update(&clone, "main", true).unwrap();
        assert_eq!(
            rev_parse(&clone, "HEAD").unwrap(),
            rev_parse(&origin, "HEAD").unwrap()
        );
        assert!(!clone.join("local").exists());
    }

    #[test]
    fn diverged_other_branch_fails_without_force_and_moves_with_it() {
        let (origin, clone) = origin_and_clone();
        git(&clone, &["checkout", "-q", "-b", "dev"]);
        commit(&clone, "local", "local");
        git(&clone, &["checkout", "-q", "main"]);
        git(&origin, &["checkout", "-q", "-b", "dev"]);
        commit(&origin, "b", "remote");
        assert!(update(&clone, "dev", false).is_err());
        update(&clone, "dev", true).unwrap();
        assert_eq!(
            rev_parse(&clone, "dev").unwrap(),
            rev_parse(&origin, "dev").unwrap()
        );
    }

    #[test]
    fn fetch_all_prunes_deleted_remote_branches() {
        let (origin, clone) = origin_and_clone();
        git(&origin, &["branch", "gone"]);
        update(&clone, "main", false).unwrap();
        assert!(rev_parse(&clone, "refs/remotes/origin/gone").is_ok());
        git(&origin, &["branch", "-D", "gone"]);
        update(&clone, "main", false).unwrap();
        assert!(rev_parse(&clone, "refs/remotes/origin/gone").is_err());
        assert!(rev_parse(&clone, "refs/remotes/origin/main").is_ok());
    }

    #[test]
    fn dirty_worktree_blocks_fast_forward() {
        let (origin, clone) = origin_and_clone();
        commit(&origin, "a", "changed");
        std::fs::write(clone.join("a"), "dirty").unwrap();
        assert!(update(&clone, "main", false).is_err());
        assert_eq!(std::fs::read_to_string(clone.join("a")).unwrap(), "dirty");
    }

    #[test]
    fn head_has_commit_tracked_and_common_dir() {
        let (_, clone) = origin_and_clone();
        assert_eq!(head_branch(&clone).unwrap().as_deref(), Some("main"));
        let sha = rev_parse(&clone, "HEAD").unwrap();
        assert!(has_commit(&clone, &sha));
        assert!(!has_commit(&clone, &"0".repeat(40)));
        assert!(!has_commit(&clone, "nonsense"));
        assert!(is_tracked(&clone, "a").unwrap());
        std::fs::write(clone.join("u"), "").unwrap();
        assert!(!is_tracked(&clone, "u").unwrap());

        let wt = clone.parent().unwrap().join("wt");
        git(
            &clone,
            &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
        );
        assert_eq!(head_branch(&wt).unwrap(), None);
        assert_eq!(git_common_dir(&wt), git_common_dir(&clone));
        assert_eq!(
            git_common_dir(&wt),
            Some(clone.join(".git").canonicalize().unwrap())
        );
        assert_eq!(git_common_dir(&clone.parent().unwrap().join("nope")), None);
    }
}
