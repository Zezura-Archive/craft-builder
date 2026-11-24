//! Token, org and fork permission checks (spec §2.4, §3.5).

use crate::repo::RepoItem;
use anyhow::{Result, bail};
use octocrab::Octocrab;

/// Check the token and org membership; return the authenticated user's login.
///
/// Spec §2.4 steps 1-4, via `GET /user` (read the `x-oauth-scopes` header) and
/// `GET /user/memberships/orgs/{fork_owner}`. The `Err` message is the spec's
/// text without `Error: `, multi-line where the spec has several lines. The
/// "no OAuth scopes" warning goes to stderr directly.
pub async fn check_access(_gh: &Octocrab, _fork_owner: &str) -> Result<String> {
    todo!("spec §2.4")
}

/// Forks in `items` must be writable by `gh_user` (spec §3.5); forks only, even for `--sync`.
///
/// With `release`, warns on stderr about forks not owned by `fork_owner`.
pub fn check_fork_write_access(
    items: &[RepoItem],
    gh_user: &str,
    fork_owner: &str,
    release: bool,
) -> Result<()> {
    let mut lacking = Vec::new();
    for item in items.iter().filter(|i| i.is_fork) {
        if release && !item.is_owned_by(fork_owner) {
            eprintln!(
                "WARNING: fork {} is not owned by {fork_owner}; it will be synced but not released.",
                item.repo
            );
        }
        if !item.can_write() {
            lacking.push(format!("\n  - {} ({})", item.repo, item.permission));
        }
    }
    if !lacking.is_empty() {
        bail!(
            "{gh_user} lacks write access to these forks (permission in brackets):{}\nGrant {gh_user} write access to them (or move them into {fork_owner}), or drop them from the star lists.",
            lacking.concat()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fork(repo: &str, permission: &str) -> RepoItem {
        RepoItem {
            repo: repo.into(),
            is_fork: true,
            archived: false,
            permission: permission.into(),
            branch: "main".into(),
            parent: "Up/x".into(),
        }
    }

    #[test]
    fn lists_forks_without_write_access() {
        let items = [
            fork("O/a", "WRITE"),
            fork("O/b", "READ"),
            fork("O/c", "NONE"),
        ];
        let err = check_fork_write_access(&items, "me", "O", false)
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "me lacks write access to these forks (permission in brackets):\n  - O/b (READ)\n  - O/c (NONE)\nGrant me write access to them (or move them into O), or drop them from the star lists."
        );
        assert!(check_fork_write_access(&items[..1], "me", "O", true).is_ok());
    }
}
