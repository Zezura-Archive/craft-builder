//! GitHub star lists via GraphQL (spec §3).

use crate::config::Config;
use crate::repo::RepoItem;
use anyhow::{Result, bail};
use octocrab::Octocrab;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::sync::LazyLock;

const LISTS_QUERY: &str = "
query {
  viewer {
    lists(first: 100) {
      nodes { id name }
    }
  }
}";

const ITEMS_QUERY: &str = "
query($id: ID!, $endCursor: String) {
  node(id: $id) {
    ... on UserList {
      items(first: 100, after: $endCursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          ... on Repository {
            nameWithOwner
            isFork
            isArchived
            viewerPermission
            defaultBranchRef { name }
            parent { nameWithOwner defaultBranchRef { name } }
          }
        }
      }
    }
  }
}";

#[derive(Deserialize)]
struct ListsData {
    viewer: Viewer,
}

#[derive(Deserialize)]
struct Viewer {
    lists: Lists,
}

#[derive(Deserialize)]
struct Lists {
    nodes: Vec<ListNode>,
}

#[derive(Deserialize)]
struct ListNode {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct ItemsData {
    node: Option<ListItemsNode>,
}

#[derive(Deserialize)]
struct ListItemsNode {
    items: Items,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Items {
    page_info: PageInfo,
    nodes: Vec<Option<RepoNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

/// A `Repository` node; non-repository nodes arrive as `{}` and lack `nameWithOwner`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoNode {
    name_with_owner: Option<String>,
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

/// Classify a node; `None` for non-repository nodes (spec §3.3).
fn classify(node: RepoNode) -> Option<RepoItem> {
    let repo = node.name_with_owner?;
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
        repo,
        is_fork,
        archived: node.is_archived,
        permission: node.viewer_permission.unwrap_or_else(|| "NONE".into()),
        branch,
        parent,
    };
    match validate(&item) {
        Ok(()) => Some(item),
        Err(reason) => {
            eprintln!("WARNING: skipping {}: {reason}", item.repo);
            None
        }
    }
}

/// Names end up in paths, URLs and git config keys, so only plain ones pass.
fn validate(item: &RepoItem) -> Result<(), String> {
    static SLUG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$").unwrap());
    let plain = |s: &str| SLUG.is_match(s) && !s.split('/').any(|seg| seg == "." || seg == "..");
    if !plain(&item.repo) {
        return Err("unsafe repository name".into());
    }
    if !item.parent.is_empty() && !plain(&item.parent) {
        return Err(format!("unsafe parent name '{}'", item.parent));
    }
    if item
        .branch
        .contains(|c: char| matches!(c, '#' | '?' | '%') || c.is_whitespace())
    {
        return Err(format!("unsafe branch name '{}'", item.branch));
    }
    Ok(())
}

/// Repo items of one page, dropping non-repository nodes.
fn classify_nodes(nodes: Vec<Option<RepoNode>>) -> Vec<RepoItem> {
    nodes.into_iter().flatten().filter_map(classify).collect()
}

/// Every repo item of the list, following `endCursor` pagination.
async fn read_list_items(gh: &Octocrab, id: &str) -> octocrab::Result<Vec<RepoItem>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let data: ItemsData = gh
            .graphql(&json!({
                "query": ITEMS_QUERY,
                "variables": { "id": id, "endCursor": cursor },
            }))
            .await?;
        let Some(node) = data.node else { break };
        items.extend(classify_nodes(node.items.nodes));
        match node.items.page_info {
            PageInfo {
                has_next_page: true,
                end_cursor: Some(next),
            } => cursor = Some(next),
            _ => break,
        }
    }
    Ok(items)
}

/// Append `listed` to `items`, skipping repos already seen (exact name, first wins).
fn add_unique(items: &mut Vec<RepoItem>, seen: &mut HashSet<String>, listed: Vec<RepoItem>) {
    items.extend(listed.into_iter().filter(|i| seen.insert(i.repo.clone())));
}

/// Read the libraries list (unless `cfg.libs_star_list` is empty), then `cfg.star_list`.
///
/// Spec §3.1-§3.3: one lists query (`POST /graphql`), then the paginated items
/// query per list. Prints `Looking up star list '<name>'...` and
/// `Found N repo(s) in '<name>'.` to stdout. Drops non-repository nodes and
/// dedups on exact `nameWithOwner` (first wins). Classifies into `RepoItem`
/// (`is_fork` needs a parent; `branch` from the parent for forks; an empty
/// branch is "none"; permission `NONE` when null). A missing list errors with
/// `no star list named '<name>' found.\nError: failed to read star list '<name>'.`;
/// a failing lists query with `failed to fetch star lists from GitHub.`
pub async fn read_star_lists(gh: &Octocrab, cfg: &Config) -> Result<Vec<RepoItem>> {
    let lists: ListsData = gh
        .graphql(&json!({ "query": LISTS_QUERY }))
        .await
        .map_err(|_| anyhow::anyhow!("failed to fetch star lists from GitHub."))?;

    let names = [&cfg.libs_star_list, &cfg.star_list]
        .into_iter()
        .filter(|name| !name.is_empty());
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for name in names {
        println!("Looking up star list '{name}'...");
        let read_failed = format!("Error: failed to read star list '{name}'.");
        let Some(list) = lists.viewer.lists.nodes.iter().find(|l| l.name == *name) else {
            bail!("no star list named '{name}' found.\n{read_failed}");
        };
        let listed = match read_list_items(gh, &list.id).await {
            Ok(listed) => listed,
            Err(err) => bail!("{err}\n{read_failed}"),
        };
        println!("Found {} repo(s) in '{name}'.", listed.len());
        add_unique(&mut items, &mut seen, listed);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn parse(nodes: Value) -> Vec<RepoItem> {
        let data: ItemsData = serde_json::from_value(json!({
            "node": { "items": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": nodes,
            }}
        }))
        .unwrap();
        classify_nodes(data.node.unwrap().items.nodes)
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
            }]
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
    fn non_repository_nodes_are_dropped() {
        let items = parse(json!([
            {}, null,
            { "nameWithOwner": "Me/a", "isFork": false, "isArchived": false,
              "viewerPermission": "ADMIN", "defaultBranchRef": { "name": "main" }, "parent": null },
        ]));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].repo, "Me/a");
    }

    #[test]
    fn dedup_is_exact_and_first_wins() {
        let (mut kept, mut seen) = (Vec::new(), HashSet::new());
        add_unique(
            &mut kept,
            &mut seen,
            parse(json!([
                { "nameWithOwner": "Me/a", "viewerPermission": "ADMIN" },
                { "nameWithOwner": "me/a", "viewerPermission": "READ" },
            ])),
        );
        add_unique(
            &mut kept,
            &mut seen,
            parse(json!([
                { "nameWithOwner": "Me/a", "viewerPermission": "READ" },
                { "nameWithOwner": "Me/b", "viewerPermission": "READ" },
            ])),
        );
        let repos: Vec<_> = kept.iter().map(|i| i.repo.as_str()).collect();
        assert_eq!(repos, ["Me/a", "me/a", "Me/b"]);
        assert_eq!(kept[0].permission, "ADMIN");
    }

    #[test]
    fn unsafe_names_are_skipped() {
        let node = |repo: &str, parent: &str, branch: &str| {
            json!({
                "nameWithOwner": repo, "isFork": true, "isArchived": false,
                "viewerPermission": "ADMIN", "defaultBranchRef": null,
                "parent": { "nameWithOwner": parent, "defaultBranchRef": { "name": branch } },
            })
        };
        let items = parse(json!([
            node("Me/ok", "Up/ok", "feat/x"),
            node("Me/..", "Up/ok", "main"),
            node("../x", "Up/ok", "main"),
            node("Me/a b", "Up/ok", "main"),
            node("Me/a", "Up/../x", "main"),
            node("Me/a", "Up", "main"),
            node("Me/b", "Up/b", "a#b"),
            node("Me/b", "Up/b", "a?b"),
            node("Me/b", "Up/b", "a%2Fb"),
            node("Me/b", "Up/b", "a b"),
        ]));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].repo, "Me/ok");
    }
}
