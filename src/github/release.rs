//! Release a built app (spec §7).

use crate::build::kind::read_metadata;
use crate::context::Ctx;
use crate::git::rev_parse;
use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use octocrab::Octocrab;
use serde_json::{Value, json};
use std::path::Path;

const RELEASES_PER_PAGE: usize = 100;
const MAX_RELEASES: usize = 1000;

/// Release notes body (no trailing newline).
fn release_notes(name: &str, version: &str, branch: &str, sha: &str) -> String {
    format!(
        "Automated build of `{name}` {version} from {branch}@{sha}.\n\n\
         - `{name}-linux-x86_64`: x86_64-unknown-linux-gnu, glibc 2.35\n\
         - `{name}-windows-x86_64.exe`: x86_64-pc-windows-msvc"
    )
}

/// The release with `tag_name == tag` in a listing.
fn find_release<'a>(releases: &'a [Value], tag: &str) -> Option<&'a Value> {
    releases.iter().find(|r| r["tag_name"] == tag)
}

/// What a git ref or tag object points at.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// A commit sha: the final target.
    Commit(String),
    /// An annotated tag object sha, still to be peeled.
    Tag(String),
}

/// Read `object.{type,sha}` of a `git/ref` or `git/tags` response.
fn target_of(response: &Value) -> Result<Target> {
    let object = &response["object"];
    let sha = object["sha"].as_str().context("ref response has no sha")?;
    Ok(match object["type"].as_str() {
        Some("tag") => Target::Tag(sha.to_owned()),
        _ => Target::Commit(sha.to_owned()),
    })
}

fn is_not_found(err: &octocrab::Error) -> bool {
    matches!(err, octocrab::Error::GitHub { source, .. } if source.status_code.as_u16() == 404)
}

/// All releases of `repo` (up to 1000), as `GET /repos/{repo}/releases` objects.
async fn list_releases(gh: &Octocrab, repo: &str) -> Result<Vec<Value>> {
    let mut releases = Vec::new();
    for page in 1..=MAX_RELEASES / RELEASES_PER_PAGE {
        let batch: Vec<Value> = gh
            .get(
                format!("/repos/{repo}/releases"),
                Some(&[("per_page", RELEASES_PER_PAGE), ("page", page)]),
            )
            .await?;
        let last = batch.len() < RELEASES_PER_PAGE;
        releases.extend(batch);
        if last {
            break;
        }
    }
    Ok(releases)
}

/// Commit sha the remote tag points at (annotated tags peeled), `None` if absent.
async fn remote_tag_commit(gh: &Octocrab, repo: &str, tag: &str) -> Result<Option<String>> {
    let mut response: Value = match gh
        .get(format!("/repos/{repo}/git/ref/tags/{tag}"), None::<&()>)
        .await
    {
        Ok(response) => response,
        Err(err) if is_not_found(&err) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    loop {
        match target_of(&response)? {
            Target::Commit(sha) => return Ok(Some(sha)),
            Target::Tag(sha) => {
                response = gh
                    .get(format!("/repos/{repo}/git/tags/{sha}"), None::<&()>)
                    .await?;
            }
        }
    }
}

async fn delete_asset(gh: &Octocrab, repo: &str, id: u64) -> Result<()> {
    let response = gh
        ._delete(format!("/repos/{repo}/releases/assets/{id}"), None::<&()>)
        .await?;
    octocrab::map_github_error(response).await?;
    Ok(())
}

/// Delete same-named assets of `release`, then upload `assets`.
async fn upload_assets(
    gh: &Octocrab,
    item: &RepoItem,
    release: &Value,
    assets: &[(String, Bytes)],
) -> Result<()> {
    let id = release["id"].as_u64().context("release has no id")?;
    let existing = release["assets"].as_array().map_or(&[][..], Vec::as_slice);
    for (name, _) in assets {
        for asset in existing.iter().filter(|a| a["name"] == name.as_str()) {
            let asset_id = asset["id"].as_u64().context("asset has no id")?;
            delete_asset(gh, &item.repo, asset_id).await?;
        }
    }
    let releases = gh.repos(item.owner(), item.name());
    for (name, data) in assets {
        releases
            .releases()
            .upload_asset(id, name, data.clone())
            .send()
            .await
            .with_context(|| format!("failed to upload {name}"))?;
    }
    Ok(())
}

/// Publish `v<version>-forked` of a built app with its two assets.
///
/// Spec §7 `release_repo`, steps 1-11. `dir` is `item.dest(&ctx.cfg.dest_dir)`;
/// assets are `cfg.build_dir/<name>/<name>-linux-x86_64` and
/// `...-windows-x86_64.exe`. The version comes from
/// `build::kind::read_metadata(dir, env, log)`. HEAD must equal
/// `origin/<branch>`. Lists releases and the tag via the API (no `git
/// ls-remote`), moves or creates the lightweight tag first, then updates the
/// release in place or creates it (draft, upload assets, publish). Progress and
/// the `Error: ...` diagnostics go to `log`; return `Err` on any failure (the
/// caller prints the error and `WARNING: failed to release`).
pub async fn release_repo(ctx: &Ctx, env: &GitEnv, item: &RepoItem, log: &mut Log) -> Result<()> {
    let (repo, name, branch) = (&item.repo, item.name(), &item.branch);
    let dir = item.dest(&ctx.cfg.dest_dir);
    let gh = &ctx.gh;

    let metadata = read_metadata(&dir, env, log)?;
    let Some(package) = metadata.packages.iter().find(|p| p.name.as_str() == name) else {
        bail!(
            "no package '{name}' in the cargo metadata of {}.",
            dir.display()
        );
    };
    let version = package.version.to_string();

    let sha = rev_parse(&dir, "HEAD")?;
    if sha != rev_parse(&dir, &format!("origin/{branch}"))? {
        bail!(
            "{} HEAD differs from origin/{branch}; not releasing an unpushed build.",
            dir.display()
        );
    }

    let tag = format!("v{version}-forked");
    let releases = list_releases(gh, repo).await?;
    let release = find_release(&releases, &tag);
    let tag_sha = remote_tag_commit(gh, repo, &tag).await?;
    let notes = release_notes(name, &version, branch, &sha);

    if release.is_some() && tag_sha.as_deref() == Some(sha.as_str()) {
        log.out(format_args!("Already released: {repo} {tag} ({sha})"));
        return Ok(());
    }

    let asset_dir = ctx.cfg.build_dir.join(name);
    let assets = [
        format!("{name}-linux-x86_64"),
        format!("{name}-windows-x86_64.exe"),
    ]
    .into_iter()
    .map(|file| read_asset(&asset_dir, file))
    .collect::<Result<Vec<_>>>()?;

    let git_ref = format!("refs/tags/{tag}");
    match tag_sha {
        None => {
            let _: Value = gh
                .post(
                    format!("/repos/{repo}/git/refs"),
                    Some(&json!({ "ref": git_ref, "sha": sha })),
                )
                .await
                .context("failed to create the tag")?;
        }
        Some(old) if old != sha => {
            let _: Value = gh
                .patch(
                    format!("/repos/{repo}/git/{git_ref}"),
                    Some(&json!({ "sha": sha, "force": true })),
                )
                .await
                .context("failed to move the tag")?;
        }
        Some(_) => {}
    }

    let title = format!("{name} {tag}");
    let url = format!("https://github.com/{repo}/releases/tag/{tag}");
    if let Some(release) = release {
        log.out(format_args!("Updating release {tag} on {repo} to {sha}..."));
        upload_assets(gh, item, release, &assets).await?;
        let id = release["id"].as_u64().context("release has no id")?;
        let _: Value = gh
            .patch(
                format!("/repos/{repo}/releases/{id}"),
                Some(&json!({ "name": title, "body": notes })),
            )
            .await
            .context("failed to edit the release")?;
        log.out(format_args!("Updated: {url}"));
    } else {
        log.out(format_args!("Creating release {tag} on {repo} at {sha}..."));
        let created: Value = gh
            .post(
                format!("/repos/{repo}/releases"),
                Some(&json!({ "tag_name": tag, "name": title, "body": notes, "draft": true })),
            )
            .await
            .context("failed to create the release")?;
        upload_assets(gh, item, &created, &assets).await?;
        let id = created["id"].as_u64().context("release has no id")?;
        let _: Value = gh
            .patch(
                format!("/repos/{repo}/releases/{id}"),
                Some(&json!({ "draft": false })),
            )
            .await
            .context("failed to publish the release")?;
        log.out(format_args!("Released: {url}"));
    }
    Ok(())
}

/// Read one asset file from `dir`, paired with its file name.
fn read_asset(dir: &Path, file: String) -> Result<(String, Bytes)> {
    let path = dir.join(&file);
    let data = std::fs::read(&path).with_context(|| format!("can't read {}", path.display()))?;
    Ok((file, data.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_text_is_verbatim() {
        assert_eq!(
            release_notes("app", "1.2.3", "main", "abc"),
            "Automated build of `app` 1.2.3 from main@abc.\n\n\
             - `app-linux-x86_64`: x86_64-unknown-linux-gnu, glibc 2.35\n\
             - `app-windows-x86_64.exe`: x86_64-pc-windows-msvc"
        );
    }

    #[test]
    fn finds_release_by_exact_tag() {
        let releases = [
            json!({ "tag_name": "v1.0.0-forked", "id": 1 }),
            json!({ "tag_name": "v1.0.0", "id": 2 }),
        ];
        assert_eq!(find_release(&releases, "v1.0.0").unwrap()["id"], 2);
        assert!(find_release(&releases, "v2.0.0-forked").is_none());
    }

    #[test]
    fn tag_target_distinguishes_commits_from_annotated_tags() {
        let commit = json!({ "object": { "type": "commit", "sha": "c1" } });
        let tag = json!({ "object": { "type": "tag", "sha": "t1" } });
        assert_eq!(target_of(&commit).unwrap(), Target::Commit("c1".into()));
        assert_eq!(target_of(&tag).unwrap(), Target::Tag("t1".into()));
        assert!(target_of(&json!({})).is_err());
    }
}
