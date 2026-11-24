//! Release a built app (spec §7).

use crate::context::Ctx;
use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use anyhow::Result;

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
pub async fn release_repo(
    _ctx: &Ctx,
    _env: &GitEnv,
    _item: &RepoItem,
    _log: &mut Log,
) -> Result<()> {
    todo!("spec §7")
}
