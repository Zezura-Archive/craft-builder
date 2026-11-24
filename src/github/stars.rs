//! GitHub star lists via GraphQL (spec §3).

use crate::config::Config;
use crate::repo::RepoItem;
use anyhow::Result;
use octocrab::Octocrab;

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
pub async fn read_star_lists(_gh: &Octocrab, _cfg: &Config) -> Result<Vec<RepoItem>> {
    todo!("spec §3")
}
