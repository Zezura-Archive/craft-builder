//! A starred repo and the `--only` matcher (spec §3.3, §3.4).

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoItem {
    /// `nameWithOwner`.
    pub repo: String,
    /// `isFork` and the parent still exists; implies a non-empty `parent`.
    pub is_fork: bool,
    pub archived: bool,
    /// `viewerPermission`, or `NONE`.
    pub permission: String,
    /// Parent's default branch for forks, else the repo's own; empty if none.
    pub branch: String,
    /// Parent `nameWithOwner`, or empty.
    pub parent: String,
}

impl RepoItem {
    /// Text after the first `/`.
    pub fn name(&self) -> &str {
        self.repo
            .split_once('/')
            .map_or(&self.repo, |(_, name)| name)
    }

    /// Text before the first `/`.
    pub fn owner(&self) -> &str {
        self.repo.split_once('/').map_or("", |(owner, _)| owner)
    }

    /// The parent if there is one, else the repo itself.
    pub fn upstream(&self) -> &str {
        if self.parent.is_empty() {
            &self.repo
        } else {
            &self.parent
        }
    }

    /// `dest_dir/<name>`.
    pub fn dest(&self, dest_dir: &Path) -> PathBuf {
        dest_dir.join(self.name())
    }

    /// Case-insensitive owner comparison (GitHub owners are case-insensitive).
    pub fn is_owned_by(&self, owner: &str) -> bool {
        self.owner().eq_ignore_ascii_case(owner)
    }

    /// True if `wanted` equals `owner/name`, `name`, or the parent (case-insensitive).
    pub fn matches(&self, wanted: &str) -> bool {
        let w = wanted.to_lowercase();
        w == self.repo.to_lowercase()
            || w == self.name().to_lowercase()
            || (!self.parent.is_empty() && w == self.parent.to_lowercase())
    }

    /// ADMIN, MAINTAIN or WRITE.
    pub fn can_write(&self) -> bool {
        matches!(self.permission.as_str(), "ADMIN" | "MAINTAIN" | "WRITE")
    }
}

/// Fail if two items would share a `dest_dir/<name>`.
pub fn ensure_unique_names(items: &[RepoItem]) -> Result<()> {
    let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for item in items {
        if let Some(first) = seen.insert(item.name(), &item.repo) {
            bail!(
                "{first} and {} share the directory name '{}'; narrow the selection with --only.",
                item.repo,
                item.name()
            );
        }
    }
    Ok(())
}

/// Narrow `all` to the repos matching any wanted name; every name must match something.
pub fn select_only(all: &[RepoItem], wanted: &[String]) -> Result<Vec<RepoItem>> {
    let unmatched: Vec<&str> = wanted
        .iter()
        .filter(|w| !all.iter().any(|item| item.matches(w)))
        .map(String::as_str)
        .collect();
    if !unmatched.is_empty() {
        let listing: String = all.iter().map(|i| format!("\n  {}", i.repo)).collect();
        bail!(
            "--only matched nothing in the star lists for: {}\nRepos in the lists:{listing}",
            unmatched.join(" ")
        );
    }
    Ok(all
        .iter()
        .filter(|item| wanted.iter().any(|w| item.matches(w)))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(repo: &str, parent: &str) -> RepoItem {
        RepoItem {
            repo: repo.into(),
            is_fork: !parent.is_empty(),
            archived: false,
            permission: "ADMIN".into(),
            branch: "main".into(),
            parent: parent.into(),
        }
    }

    fn wanted(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn accessors() {
        let i = item("Fork/app", "Up/app");
        assert_eq!(
            (i.name(), i.owner(), i.upstream()),
            ("app", "Fork", "Up/app")
        );
        assert_eq!(item("Me/lib", "").upstream(), "Me/lib");
        assert_eq!(i.dest(Path::new("/d")), Path::new("/d/app"));
        assert!(i.is_owned_by("fork"));
    }

    #[test]
    fn matches_repo_name_or_parent_case_insensitively() {
        let all = [item("Fork/app", "Up/app"), item("Me/lib", "")];
        for w in ["fork/APP", "APP", "up/app"] {
            assert_eq!(select_only(&all, &wanted(&[w])).unwrap().len(), 1, "{w}");
        }
        assert_eq!(
            select_only(&all, &wanted(&["me/lib", "app"]))
                .unwrap()
                .len(),
            2
        );
        assert!(select_only(&all, &wanted(&["Up/lib"])).is_err());
    }

    #[test]
    fn unmatched_names_are_listed_with_all_repos() {
        let all = [item("Me/lib", "")];
        let err = select_only(&all, &wanted(&["lib", "x", "y"]))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "--only matched nothing in the star lists for: x y\nRepos in the lists:\n  Me/lib"
        );
    }

    #[test]
    fn same_name_under_different_owners_is_an_error() {
        assert!(ensure_unique_names(&[item("A/x", ""), item("B/y", "")]).is_ok());
        let err = ensure_unique_names(&[item("A/x", ""), item("B/x", "")]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "A/x and B/x share the directory name 'x'; narrow the selection with --only."
        );
    }
}
