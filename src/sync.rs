//! Phase 1: sync one repo (spec §4).

use crate::context::Ctx;
use crate::git;
use crate::github::fork::sync_fork;
use crate::repo::RepoItem;
use crate::report::Log;
use tokio::task::block_in_place;

/// GitHub fork sync, clone if missing, then local fast-forward.
///
/// On failure returns the failure entry, e.g. `owner/name (clone)`; warnings
/// and error details have already gone to `log`. The `==== repo ====` header is
/// the caller's.
pub async fn sync_repo(ctx: &Ctx, item: &RepoItem, log: &mut Log) -> Result<(), String> {
    let (repo, branch) = (&item.repo, &item.branch);
    let dest = item.dest(&ctx.cfg.dest_dir);

    if branch.is_empty() {
        log.err(format!(
            "WARNING: {repo} has no default branch (empty repo?); skipping."
        ));
        return Err(format!("{repo} (no default branch)"));
    }

    if item.is_fork {
        let parent = &item.parent;
        if item.archived {
            log.err(format!(
                "WARNING: {repo} is archived (read-only) on GitHub; cannot sync it from {parent}."
            ));
            return Err(format!("{repo} (archived on GitHub)"));
        }
        log.out(format!(
            "Syncing {repo}:{branch} from {parent}:{branch} on GitHub..."
        ));
        if let Err(err) = sync_fork(&ctx.gh, item, ctx.cfg.force).await {
            log.err(format!("Error: {err:#}"));
            log.err(format!(
                "WARNING: failed to sync {repo} from {parent} (diverged? retry with --force)."
            ));
            return Err(format!("{repo} (GitHub sync)"));
        }
    }

    if !dest.exists() {
        log.out(format!("Cloning into {}...", dest.display()));
        if let Err(err) = block_in_place(|| git::clone_repo(item, &dest, &ctx.token, log)) {
            log.err(format!("Error: {err:#}"));
            log.err(format!("WARNING: failed to clone {repo}"));
            let _ = std::fs::remove_dir_all(&dest);
            return Err(format!("{repo} (clone)"));
        }
    } else if !dest.join(".git").is_dir() {
        log.err(format!(
            "WARNING: {} exists but is not a git repository; skipping.",
            dest.display()
        ));
        return Err(format!("{repo} (not a git repo: {})", dest.display()));
    }

    if let Err(err) =
        block_in_place(|| git::update_local(&dest, branch, ctx.cfg.force, &ctx.token, log))
    {
        log.err(format!("Error: {err:#}"));
        log.err(format!(
            "WARNING: failed to update local {branch} in {}",
            dest.display()
        ));
        return Err(format!("{repo} (local update)"));
    }
    log.out(format!("Up to date: {} ({branch})", dest.display()));
    Ok(())
}
