//! Token, org and fork permission checks (spec §2.4, §3.5).

use crate::repo::RepoItem;
use anyhow::{Result, bail};
use octocrab::Octocrab;
use serde_json::Value;

const REQUIRED_SCOPES: [&str; 3] = ["repo", "workflow", "read:org"];
const REFRESH_HINT: &str = "  gh auth refresh -h github.com -s repo,workflow,read:org";

/// Required scopes absent from an `x-oauth-scopes` header value.
///
/// `admin:org` and `write:org` also satisfy `read:org`.
fn missing_scopes(header: &str) -> Vec<&'static str> {
    let granted: Vec<&str> = header.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    REQUIRED_SCOPES
        .into_iter()
        .filter(|&need| {
            !granted.iter().any(|&have| {
                have == need || (need == "read:org" && matches!(have, "admin:org" | "write:org"))
            })
        })
        .collect()
}

/// Error text for an org membership request that failed with `membership`.
fn org_access_error(org: &str, user: &str, membership: &str) -> String {
    let advice = if membership.contains("OAuth App access restrictions") {
        "Approve GitHub CLI for the org (Settings -> Third-party access -> OAuth app policy), or re-authorize gh and grant the org:".to_owned()
    } else {
        format!(
            "Check you're a member of '{org}' (and FORK_OWNER is right), then re-authorize gh, granting the org:"
        )
    };
    format!("can't access org '{org}' as {user}:\n  {membership}\n{advice}\n{REFRESH_HINT}")
}

/// Check the token and org membership; return the authenticated user's login.
///
/// Spec §2.4 steps 1-4, via `GET /user` (read the `x-oauth-scopes` header) and
/// `GET /user/memberships/orgs/{fork_owner}`. The `Err` message is the spec's
/// text without `Error: `, multi-line where the spec has several lines. The
/// "no OAuth scopes" warning goes to stderr directly.
pub async fn check_access(gh: &Octocrab, fork_owner: &str) -> Result<String> {
    let user: Value = gh.get("/user", None::<&()>).await.ok().unwrap_or_default();
    let Some(login) = user["login"].as_str().filter(|l| !l.is_empty()) else {
        bail!("failed to look up the authenticated GitHub user.");
    };

    let query_failed = || format!("failed to query GitHub as {login}.");
    let response = gh
        ._get("/user")
        .await
        .map_err(|_| anyhow::anyhow!(query_failed()))?;
    if !response.status().is_success() {
        bail!(query_failed());
    }
    let scopes = response
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    if scopes.is_empty() {
        eprintln!(
            "WARNING: gh's token reports no OAuth scopes (fine-grained or app token?); skipping the scope check."
        );
    } else {
        let missing = missing_scopes(scopes);
        if !missing.is_empty() {
            bail!(
                "gh's token is missing scope(s): {}. Fix with:\n{REFRESH_HINT}",
                missing.join(" ")
            );
        }
    }

    if !fork_owner.eq_ignore_ascii_case(login) {
        let route = format!("/user/memberships/orgs/{fork_owner}");
        let membership: Value = match gh.get(route, None::<&()>).await {
            Ok(value) => value,
            Err(octocrab::Error::GitHub { source, .. }) => {
                let text = format!(
                    "gh: {} (HTTP {})",
                    source.message,
                    source.status_code.as_u16()
                );
                bail!(org_access_error(fork_owner, login, &text));
            }
            Err(err) => bail!(org_access_error(fork_owner, login, &err.to_string())),
        };
        let state = membership["state"].as_str().unwrap_or_default();
        if state != "active" {
            bail!(
                "your membership of '{fork_owner}' is '{state}', not active. Accept the invitation on GitHub first."
            );
        }
    }
    Ok(login.to_owned())
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

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn all_scopes_present() {
        assert!(missing_scopes("repo, workflow, read:org").is_empty());
        assert!(missing_scopes("repo,workflow,read:org,gist").is_empty());
    }

    #[test]
    fn admin_and_write_org_satisfy_read_org() {
        assert!(missing_scopes("repo, workflow, admin:org").is_empty());
        assert!(missing_scopes("repo workflow write:org").is_empty());
    }

    #[test]
    fn reports_missing_scopes_in_order() {
        assert_eq!(missing_scopes("gist"), ["repo", "workflow", "read:org"]);
        assert_eq!(missing_scopes("repo, read:org"), ["workflow"]);
    }

    #[test]
    fn org_error_depends_on_oauth_restrictions() {
        let plain = org_access_error("O", "me", "gh: Not Found (HTTP 404)");
        assert_eq!(
            plain,
            "can't access org 'O' as me:\n  gh: Not Found (HTTP 404)\nCheck you're a member of 'O' (and FORK_OWNER is right), then re-authorize gh, granting the org:\n  gh auth refresh -h github.com -s repo,workflow,read:org"
        );
        let restricted = org_access_error("O", "me", "OAuth App access restrictions");
        assert!(restricted.contains("Approve GitHub CLI for the org"));
    }
}
