# Usage

```bash
nix run . -- [options]
```

`nix run` and the built package put the build tools (Rust, `cargo-zigbuild`, `zig`, `cargo-xwin`, clang/lld/llvm, `sccache`, `git`, `pkg-config`) on `PATH`. Outside them, the same tools must already be on `PATH`. `--clean` needs none of them.

Examples:

```bash
nix run . --                                          # sync + build + release
```

```bash
nix run . -- --sync                                   # sync only
```

```bash
nix run . -- --build --only filmcraft,effectcraft     # two repos, no release
```

```bash
nix run . -- -j 8                                     # 8 parallel syncs
```

## Options

At most one of `--sync`, `--build`, `--release` and `--clean` may be given.

| Option | Description |
| --- | --- |
| `-c`, `--config FILE` | Read this [config file](#config-file) instead of the default one. It must exist. |
| `--sync` | Sync only (phase 1). |
| `--build` | Sync, then build (phases 1-2). No releases. |
| `--release` | Sync, build, then release (phases 1-3). The default. |
| `--clean` | Delete `CARGO_TARGET_DIR` and `SCCACHE_DIR`, nothing else. Takes no other options. |
| `-f`, `--force` | Hard-reset instead of fast-forward only, on GitHub and locally. Discards fork commits that upstream lacks. |
| `--only REPO[,REPO...]` | Process only these apps. Repeatable and comma-separated. Matches a repo's name, `owner/name`, or a fork's upstream `owner/name`. A name that matches no app is an error. Libraries are always synced. |
| `-j`, `--jobs N` | Sync up to `N` repos in parallel. Default `1`; must be a positive integer. Builds and releases stay serial. With `N > 1`, each repo's output is printed as one block when it finishes. |
| `-h`, `--help` / `-V`, `--version` | Show help or the version. |

Unselected apps (`--only`) aren't synced, built or released, but their existing clones and forks still serve the selected repos' git dependencies and craft-fonts.

## Config file

`$XDG_CONFIG_HOME/craft-builder/config.toml`, else `~/.config/craft-builder/config.toml`, or the file given with `--config`. It must list at least one app or library; every other key is optional:

```toml
fork_owner = "Zezura-Archive"        # org that owns the forks; only its repos are released
apps = ["filmcraft", "effectcraft"]  # built and released
libraries = ["craft-libs", "egui"]   # synced first; never built or released
build_dir = "~/builds/craft-apps/"   # where binaries go, in <name>/
clone_dir = "~/repos/craft-apps/"    # where repos are cloned; created if missing
```

`apps` and `libraries` are bare repo names in `fork_owner`, and a name may appear only once across both. Libraries are forks that apps depend on through git dependencies. A leading `~` means your home directory. Unknown keys are an error. Defaults: `fork_owner` is `Zezura-Archive`, `clone_dir` is `~/repos/craft-apps`, `build_dir` is `~/builds/craft-apps`.

## Environment variables

Empty values count as unset, except where noted.

| Variable | Default | Description |
| --- | --- | --- |
| `CARGO_TARGET_DIR` | `~/.cache/craft-forks/target` | Cargo target dir shared by all repos, with `linux/` and `windows/` subdirectories so both builds can run at once. |
| `RUSTC_WRAPPER` | `sccache` | Compiler wrapper. Set it empty for none. |
| `SCCACHE_CACHE_SIZE` | `30G` | sccache's cache size limit. Takes effect when the sccache server starts; run `sccache --stop-server` after changing it. |
| `SCCACHE_DIR` | `$XDG_CACHE_HOME/sccache`, else `~/.cache/sccache` | sccache cache. Also what `--clean` removes. |
| `CRAFT_FONTS_CACHE` | `~/.cache/craft-forks/craft-fonts` | craft-fonts checkout used by font-embedding builds. |
| `XDG_CONFIG_HOME` | `~/.config` | Where the default [config file](#config-file) is looked for. |
| `HOME` | none | Required. |

On WSL, a warning is printed when `clone_dir`, `CARGO_TARGET_DIR` or `SCCACHE_DIR` is under `/mnt/`: that's a Windows drive, where git and cargo are much slower.

The token variables are listed under [Token](#token).

## Phases

Libraries are synced first, in a pass that finishes before the apps are synced. Libraries are not built or released, but apps use them for git dependencies. Sync finishes for all repos before builds start; each app's release overlaps the next app's build (see Release).

1. **Sync**, per repo:
   - For a fork, fast-forward its default branch on GitHub from its parent.
   - Clone into `clone_dir/<name>` if missing (forks also get an `upstream` remote); otherwise fetch all remotes with pruning, then tags from `origin` (your fork's tags win).
   - Fast-forward the local default branch to `origin/<branch>`.
2. **Build**, per synced app, one app at a time. An app's Linux and Windows builds run at the same time; the Windows output is printed after the Linux build finishes:
   - Git dependencies are taken from your copy of the repo (its local clone, else its fork) instead of the URL in `Cargo.lock`. This uses a run-only `insteadOf` rewrite; `Cargo.toml` and `Cargo.lock` are untouched.
   - Any commit pinned by `Cargo.lock` that a clone lacks is fetched into it from your fork first. If that fails the repo isn't built.
   - A git dependency with no copy in your config's `libraries` (including non-GitHub ones) fails the build instead of being fetched from upstream. Add its fork to `libraries`.
   - Fonts: if `.github/workflows/release.yml` pins a [storytold/craft-fonts](https://github.com/storytold/craft-fonts) commit, it is checked out into `CRAFT_FONTS_CACHE` and builds get `CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED=1`. craft-fonts comes from your synced `craft-fonts` clone; without one in `libraries`, or if the checkout fails, the repo isn't built instead of fetching from upstream.
   - Builds run in release mode from the working tree, so a clone that isn't on the default branch is reported and not built.
   - The repo's main binary (the bin named after the repo) is built for both targets and copied to `build_dir/<name>/`:
     - `<name>-linux-x86_64`: `cargo zigbuild` for `x86_64-unknown-linux-gnu`, pinned to glibc 2.35 so it runs on non-NixOS systems.
     - `<name>-windows-x86_64.exe`: `cargo xwin build` for `x86_64-pc-windows-msvc`. The MSVC CRT/SDK is downloaded into `~/.cache/cargo-xwin` on first use.
   - macOS is not built. Windows icon and version resources are not embedded.
   - Apps with libraries but no such binary get their workspace libraries compiled for both targets and are not released.
   - Apps with neither are only synced.
   - sccache hit/miss stats are printed at the end of the run.
3. **Release**, per built app. Each app is released as soon as it's built, one release at a time, while the next app builds. Release output is printed between builds, never in the middle of one:
   - Only repos owned by `fork_owner` and built for both targets are released.
   - The release tag is `v<version>-forked`, from the binary's version in `cargo metadata`, targeting the built commit.
   - An existing release for that tag is updated in place: its tag is moved to the built commit, and assets and notes are replaced. If the tag already points at the built commit, nothing is done.

Non-fork repos in the config are only cloned or fetched and fast-forwarded. A fork whose branch has diverged from upstream, an archived fork, or a local clone with diverged commits is reported as failed, not overwritten. Failures are listed at the end and the exit code is 1.

`--clean` stops the sccache server (if installed), then removes `CARGO_TARGET_DIR` and `SCCACHE_DIR`. It refuses to remove `/` or `$HOME`. `clone_dir`, `build_dir` and `CRAFT_FONTS_CACHE` are kept.

## Token

The GitHub token is the first non-empty of:

1. `GH_TOKEN`
2. `GITHUB_TOKEN`
3. the output of `gh auth token`

`gh` is only used for that last fallback. GitHub API calls are made directly and git operations go through libgit2 over HTTPS with the token, so no SSH key is needed. Your git config and the clones' remote URLs are unchanged. The `git` CLI is still used by cargo for git dependencies and by the tool for pinned-commit fetches and the craft-fonts worktree.

## Access checks

Before syncing, the tool checks and exits with the fix if one fails (not run for `--clean`):

- The token has the `repo`, `workflow` and `read:org` scopes (`admin:org` or `write:org` also satisfy `read:org`). Skipped, with a warning, for fine-grained or app tokens, which report no scopes.
- You are the `fork_owner` user or an active member of the `fork_owner` org. This also catches an org that restricts OAuth app access; approve the app under the org's Settings, Third-party access.
- You have write access (`WRITE`, `MAINTAIN` or `ADMIN`) to every fork in the config. Forks not owned by `fork_owner` are flagged as sync-only.

Most scope problems are fixed with:

```bash
gh auth refresh -h github.com -s repo,workflow,read:org
```
