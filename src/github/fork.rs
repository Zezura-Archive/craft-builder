//! Fork sync on GitHub (`gh repo sync` equivalent, spec §4).

use crate::repo::RepoItem;
use anyhow::Result;
use octocrab::Octocrab;

/// Update the fork's default branch from its parent on GitHub.
///
/// Without `force`: fast-forward via `POST /repos/{fork}/merge-upstream`
/// `{"branch"}`; a 409 (diverged) is an error. With `force`: hard-set the fork
/// branch to the parent's head (`PATCH /repos/{fork}/git/refs/heads/{branch}`
/// `{"sha","force":true}`), doing nothing if identical. Precondition:
/// `item.is_fork` and a non-empty `item.branch`.
pub async fn sync_fork(_gh: &Octocrab, _item: &RepoItem, _force: bool) -> Result<()> {
    todo!("spec §4 gh repo sync")
}
