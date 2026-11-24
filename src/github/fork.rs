//! Fork sync on GitHub (`gh repo sync` equivalent, spec §4).

use crate::repo::RepoItem;
use anyhow::{Context, Result, bail};
use octocrab::Octocrab;
use regex::Regex;
use serde_json::{Value, json};

/// How a failed `merge-upstream` call is handled.
#[derive(Debug, PartialEq, Eq)]
enum MergeFailure {
    /// The token cannot push workflow changes.
    MissingWorkflowScope,
    /// Not fast-forwardable here: try updating the ref directly.
    UpdateRef,
    Other,
}

fn classify_merge_failure(status: u16, message: &str) -> MergeFailure {
    if !matches!(status, 409 | 422) {
        return MergeFailure::Other;
    }
    let workflow = Regex::new(r"refusing to allow.*without `?workflows?`? (scope|permission)")
        .expect("valid regex");
    if workflow.is_match(message) {
        MergeFailure::MissingWorkflowScope
    } else {
        MergeFailure::UpdateRef
    }
}

/// Status code and message of a GitHub API error.
fn api_error(err: &octocrab::Error) -> Option<(u16, &str)> {
    match err {
        octocrab::Error::GitHub { source, .. } => {
            Some((source.status_code.as_u16(), source.message.as_str()))
        }
        _ => None,
    }
}

/// Explain a failed ref update (message of `gh repo sync`).
fn ref_update_error(message: &str, repo: &str, branch: &str) -> anyhow::Error {
    if message.contains("Update is not a fast forward") {
        anyhow::anyhow!(
            "can't sync because there are diverging changes; you can use `--force` to overwrite the {branch} branch"
        )
    } else if message.contains("Reference does not exist") {
        anyhow::anyhow!("{branch} branch does not exist on {repo} repository")
    } else {
        anyhow::anyhow!("{message}")
    }
}

/// Update the fork's default branch from its parent on GitHub.
///
/// Mirrors `gh repo sync --branch B [--force]`: `POST /repos/{fork}/merge-upstream`
/// `{"branch"}` first; on a 409/422 fall back to `PATCH
/// /repos/{fork}/git/refs/heads/{branch}` `{"sha": <parent head>, "force"}`.
/// Precondition: `item.is_fork` and a non-empty `item.branch`.
pub async fn sync_fork(gh: &Octocrab, item: &RepoItem, force: bool) -> Result<()> {
    let (fork, parent, branch) = (&item.repo, &item.parent, &item.branch);
    let err = match gh
        .repos(item.owner(), item.name())
        .merge_upstream(branch)
        .await
    {
        Ok(_) => return Ok(()),
        Err(err) => err,
    };
    let Some((status, message)) = api_error(&err) else {
        return Err(err.into());
    };
    match classify_merge_failure(status, message) {
        MergeFailure::MissingWorkflowScope => bail!(
            "couldn't sync {fork}: the token lacks the `workflow` scope; run `gh auth refresh -h github.com -s workflow`"
        ),
        MergeFailure::Other => return Err(err.into()),
        MergeFailure::UpdateRef => {}
    }

    let head: Value = gh
        .get(
            format!("/repos/{parent}/git/ref/heads/{branch}"),
            None::<&()>,
        )
        .await
        .with_context(|| format!("failed to read {branch} of {parent}"))?;
    let sha = head["object"]["sha"]
        .as_str()
        .with_context(|| format!("no sha for {branch} of {parent}"))?;
    let patched: octocrab::Result<Value> = gh
        .patch(
            format!("/repos/{fork}/git/refs/heads/{branch}"),
            Some(&json!({ "sha": sha, "force": force })),
        )
        .await;
    match patched {
        Ok(_) => Ok(()),
        Err(err) => match api_error(&err) {
            Some((_, message)) => Err(ref_update_error(message, fork, branch)),
            None => Err(err.into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_and_unprocessable_fall_back_to_ref_update() {
        for status in [409, 422] {
            assert_eq!(
                classify_merge_failure(status, "There are merge conflicts"),
                MergeFailure::UpdateRef
            );
        }
    }

    #[test]
    fn workflow_scope_error_is_detected() {
        for msg in [
            "refusing to allow an OAuth App to create or update workflow `x.yml` without `workflow` scope",
            "refusing to allow a Personal Access Token to create or update workflow `x.yml` without workflow permission",
        ] {
            assert_eq!(
                classify_merge_failure(422, msg),
                MergeFailure::MissingWorkflowScope
            );
        }
    }

    #[test]
    fn other_statuses_are_returned() {
        assert_eq!(
            classify_merge_failure(404, "Not Found"),
            MergeFailure::Other
        );
        assert_eq!(classify_merge_failure(500, ""), MergeFailure::Other);
    }

    #[test]
    fn ref_update_errors_are_mapped() {
        let e = |m| ref_update_error(m, "O/r", "main").to_string();
        assert!(e("Update is not a fast forward").contains("diverging changes"));
        assert_eq!(
            e("Reference does not exist"),
            "main branch does not exist on O/r repository"
        );
        assert_eq!(e("Bad credentials"), "Bad credentials");
    }
}
