//! GitHub token lookup (GH_TOKEN, GITHUB_TOKEN, then `gh auth token`).

use anyhow::{Result, anyhow};
use std::process::Command;

pub fn resolve() -> Result<String> {
    from_env(|k| std::env::var(k).ok())
        .or_else(from_gh_cli)
        .ok_or_else(|| {
            anyhow!(
                "no GitHub token: set GH_TOKEN or GITHUB_TOKEN, or log in with 'gh auth login'."
            )
        })
}

/// First non-empty of GH_TOKEN, GITHUB_TOKEN.
pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .filter_map(|k| get(k))
        .map(|t| t.trim().to_owned())
        .find(|t| !t.is_empty())
}

fn from_gh_cli() -> Option<String> {
    let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_token_wins_and_empty_is_skipped() {
        let get = |k: &str| match k {
            "GH_TOKEN" => Some(" ".to_owned()),
            "GITHUB_TOKEN" => Some("b\n".to_owned()),
            _ => None,
        };
        assert_eq!(from_env(get).as_deref(), Some("b"));
        assert_eq!(from_env(|_| None), None);
    }
}
