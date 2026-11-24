//! The configured apps and libraries, looked up via GraphQL (spec §3).

use crate::config::Config;
use crate::repo::{RepoItem, RepoKind, validate};
use anyhow::{Result, anyhow, bail};
use octocrab::Octocrab;
use serde::Deserialize;
use serde_json::json;

const REPO_QUERY: &str = "
query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    isFork
    isArchived
    viewerPermission
    defaultBranchRef { name }
    parent { nameWithOwner defaultBranchRef { name } }
  }
}";

#[derive(Deserialize)]
struct RepoData {
    repository: Option<RepoNode>,
}

/// A `Repository` node.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoNode {
    name_with_owner: String,
    #[serde(default)]
    is_fork: bool,
    #[serde(default)]
    is_archived: bool,
    viewer_permission: Option<String>,
    default_branch_ref: Option<BranchRef>,
    parent: Option<Parent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Parent {
    name_with_owner: String,
    default_branch_ref: Option<BranchRef>,
}

#[derive(Deserialize)]
struct BranchRef {
    name: String,
}

fn branch_name(branch: Option<BranchRef>) -> String {
    branch.map(|b| b.name).unwrap_or_default()
}

/// Classify a node (spec §3.3); an unsafe name is an error.
fn classify(node: RepoNode, kind: RepoKind) -> Result<RepoItem> {
    let is_fork = node.is_fork && node.parent.is_some();
    let (branch, parent) = match node.parent {
        Some(parent) => (
            if is_fork {
                branch_name(parent.default_branch_ref)
            } else {
                branch_name(node.default_branch_ref)
            },
            parent.name_with_owner,
        ),
        None => (branch_name(node.default_branch_ref), String::new()),
    };
    let item = RepoItem {
        repo: node.name_with_owner,
        is_fork,
        archived: node.is_archived,
        permission: node.viewer_permission.unwrap_or_else(|| "NONE".into()),
        branch,
        parent,
        kind,
    };
    validate(&item).map_err(|reason| anyhow!("{}: {reason}", item.repo))?;
    Ok(item)
}

/// Look up `org/name`; a missing or unreadable repo is an error naming it.
async fn read_repo(gh: &Octocrab, org: &str, name: &str, kind: RepoKind) -> Result<RepoItem> {
    let data: RepoData = gh
        .graphql(&json!({
            "query": REPO_QUERY,
            "variables": { "owner": org, "name": name },
        }))
        .await
        .map_err(|err| anyhow!("cannot read repo {org}/{name}: {err}"))?;
    match data.repository {
        Some(node) => classify(node, kind),
        None => bail!("repo {org}/{name} not found."),
    }
}

/// The configured libraries, then apps, as repos of `cfg.fork_owner`.
pub async fn read_repos(gh: &Octocrab, cfg: &Config) -> Result<Vec<RepoItem>> {
    let libraries = cfg.libraries.iter().map(|n| (n, RepoKind::Library));
    let apps = cfg.apps.iter().map(|n| (n, RepoKind::App));
    let mut items = Vec::new();
    println!("Looking up repos in {}...", cfg.fork_owner);
    for (name, kind) in libraries.chain(apps) {
        items.push(read_repo(gh, &cfg.fork_owner, name, kind).await?);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn parse(nodes: Value) -> Vec<RepoItem> {
        let nodes: Vec<RepoNode> = serde_json::from_value(nodes).unwrap();
        nodes
            .into_iter()
            .map(|n| classify(n, RepoKind::App).unwrap())
            .collect()
    }

    fn node(repo: &str, parent: &str, branch: &str) -> Value {
        json!({
            "nameWithOwner": repo, "isFork": true, "isArchived": false,
            "viewerPermission": "ADMIN", "defaultBranchRef": null,
            "parent": { "nameWithOwner": parent, "defaultBranchRef": { "name": branch } },
        })
    }

    #[test]
    fn fork_takes_branch_and_parent_from_parent() {
        let items = parse(json!([{
            "nameWithOwner": "Me/app", "isFork": true, "isArchived": false,
            "viewerPermission": "ADMIN",
            "defaultBranchRef": { "name": "mine" },
            "parent": { "nameWithOwner": "Up/app", "defaultBranchRef": { "name": "trunk" } },
        }]));
        assert_eq!(
            items,
            [RepoItem {
                repo: "Me/app".into(),
                is_fork: true,
                archived: false,
                permission: "ADMIN".into(),
                branch: "trunk".into(),
                parent: "Up/app".into(),
                kind: RepoKind::App,
            }]
        );
    }

    #[test]
    fn kind_is_carried_through() {
        let node: RepoNode = serde_json::from_value(node("Me/lib", "Up/lib", "main")).unwrap();
        assert_eq!(
            classify(node, RepoKind::Library).unwrap().kind,
            RepoKind::Library
        );
    }

    #[test]
    fn non_fork_uses_own_branch_and_null_permission_is_none() {
        let items = parse(json!([{
            "nameWithOwner": "Me/lib", "isFork": false, "isArchived": true,
            "viewerPermission": null,
            "defaultBranchRef": { "name": "main" }, "parent": null,
        }]));
        assert_eq!(items[0].branch, "main");
        assert_eq!(items[0].permission, "NONE");
        assert!(!items[0].is_fork && items[0].archived);
        assert_eq!(items[0].parent, "");
    }

    #[test]
    fn fork_without_parent_is_not_a_fork() {
        let items = parse(json!([{
            "nameWithOwner": "Me/x", "isFork": true, "isArchived": false,
            "viewerPermission": "READ", "defaultBranchRef": { "name": "main" }, "parent": null,
        }]));
        assert!(!items[0].is_fork);
        assert_eq!(items[0].branch, "main");
    }

    #[test]
    fn null_default_branch_is_empty() {
        let items = parse(json!([{
            "nameWithOwner": "Me/x", "isFork": true, "isArchived": false,
            "viewerPermission": "WRITE", "defaultBranchRef": null,
            "parent": { "nameWithOwner": "Up/x", "defaultBranchRef": null },
        }, {
            "nameWithOwner": "Me/y", "isFork": false, "isArchived": false,
            "viewerPermission": "WRITE", "defaultBranchRef": null, "parent": null,
        }]));
        assert_eq!(items[0].branch, "");
        assert_eq!(items[0].parent, "Up/x");
        assert_eq!(items[1].branch, "");
    }

    #[test]
    fn unsafe_names_are_errors() {
        let ok: RepoNode = serde_json::from_value(node("Me/ok", "Up/ok", "feat/x")).unwrap();
        assert!(classify(ok, RepoKind::App).is_ok());
        for (repo, parent, branch) in [
            ("Me/..", "Up/ok", "main"),
            ("../x", "Up/ok", "main"),
            ("Me/a b", "Up/ok", "main"),
            ("Me/a", "Up/../x", "main"),
            ("Me/a", "Up", "main"),
            ("Me/b", "Up/b", "a#b"),
            ("Me/b", "Up/b", "a?b"),
            ("Me/b", "Up/b", "a%2Fb"),
            ("Me/b", "Up/b", "a b"),
        ] {
            let n: RepoNode = serde_json::from_value(node(repo, parent, branch)).unwrap();
            assert!(
                classify(n, RepoKind::App).is_err(),
                "{repo} {parent} {branch}"
            );
        }
    }
}
