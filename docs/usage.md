# Usage

```bash
nix run . -- [options] [DEST_DIR]
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
nix run . -- -j 8 ~/archive                           # 8 parallel syncs, custom DEST_DIR
```

```bash
STAR_LIST="Other List" LIBS_STAR_LIST="" nix run .    # custom lists, no libraries list
```

## Options

At most one of `--sync`, `--build`, `--release` and `--clean` may be given.

| Option | Description |
| --- | --- |
| `DEST_DIR` | Where repos are cloned. Overrides `clone_dir` from the [config file](#config-file). Default `~/repos/craft-apps`; created if missing. |
| `-c`, `--config FILE` | Read this [config file](#config-file) instead of the default one. It must exist. |
| `--sync` | Sync only (phase 1). |
| `--build` | Sync, then build (phases 1-2). No releases. |
| `--release` | Sync, build, then release (phases 1-3). The default. |
| `--clean` | Delete `CARGO_TARGET_DIR` and `SCCACHE_DIR`, nothing else. Takes no other options or `DEST_DIR`. |
| `-f`, `--force` | Hard-reset instead of fast-forward only, on GitHub and locally. Discards fork commits that upstream lacks. |
| `--only REPO[,REPO...]` | Process only these repos. Repeatable and comma-separated. Matches a repo's name, `owner/name`, or a fork's upstream `owner/name`. A name that matches nothing is an error. |
| `-j`, `--jobs N` | Sync up to `N` repos in parallel. Default `1`; must be a positive integer. Builds and releases stay serial. With `N > 1`, each repo's output is printed as one block when it finishes. |
| `-h`, `--help` / `-V`, `--version` | Show help or the version. |

Unselected repos (`--only`) aren't synced, built or released, but their existing clones and forks still serve the selected repos' git dependencies and craft-fonts.

## Config file

`$XDG_CONFIG_HOME/craft-builder/config.toml`, else `~/.config/craft-builder/config.toml`, or the file given with `--config`. It's optional, and every key is optional:

```toml
build_dir = "~/builds/craft-apps/"   # like BUILD_DIR
clone_dir = "~/repos/craft-apps/"    # like DEST_DIR
```

A leading `~` means your home directory. Unknown keys are an error. `DEST_DIR` and `BUILD_DIR` override the file.

## Environment variables

Empty values count as unset, except where noted.

| Variable | Default | Description |
| --- | --- | --- |
| `STAR_LIST` | `zezura-archive-craft` | Star list of apps (exact match). |
| `LIBS_STAR_LIST` | `zezura-archive-craft-libs` | Star list of libraries (exact match). A missing list is an error. Set it empty to read only `STAR_LIST`. |
| `FORK_OWNER` | `Zezura-Archive` | User or org that owns the forks. Only its repos are released. Case-insensitive. |
| `BUILD_DIR` | `build_dir` from the [config file](#config-file), else `~/builds/craft-apps` | Where binaries go, in `<name>/`. |
| `CARGO_TARGET_DIR` | `~/.cache/craft-forks/target` | Cargo target dir shared by all repos. |
| `RUSTC_WRAPPER` | `sccache` | Compiler wrapper. Set it empty for none. |
| `SCCACHE_DIR` | `$XDG_CACHE_HOME/sccache`, else `~/.cache/sccache` | sccache cache. Also what `--clean` removes. |
| `CRAFT_FONTS_CACHE` | `~/.cache/craft-forks/craft-fonts` | craft-fonts checkout used by font-embedding builds. |
| `XDG_CONFIG_HOME` | `~/.config` | Where the default [config file](#config-file) is looked for. |
| `HOME` | none | Required. |

The token variables are listed under [Token](#token).

## Phases

Repos from both star lists are processed in each phase before the next starts, libraries before apps.

1. **Sync**, per repo:
   - For a fork, fast-forward its default branch on GitHub from its parent.
   - Clone into `DEST_DIR/<name>` if missing (forks also get an `upstream` remote); otherwise fetch all remotes with pruning, then tags from `origin` (your fork's tags win).
   - Fast-forward the local default branch to `origin/<branch>`.
2. **Build**, per synced repo, serially:
   - Git dependencies are taken from your copy of the repo (its local clone, else its fork) instead of the URL in `Cargo.lock`. This uses a run-only `insteadOf` rewrite; `Cargo.toml` and `Cargo.lock` are untouched.
   - Any commit pinned by `Cargo.lock` that a clone lacks is fetched into it from your fork first. If that fails the repo isn't built.
   - Fonts: if `.github/workflows/release.yml` pins a [storytold/craft-fonts](https://github.com/storytold/craft-fonts) commit, it is checked out into `CRAFT_FONTS_CACHE` and builds get `CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED=1`. If the checkout fails the repo isn't built.
   - Builds run in release mode from the working tree, so a clone that isn't on the default branch is reported and not built.
   - The repo's main binary (the bin named after the repo) is built for both targets and copied to `BUILD_DIR/<name>/`:
     - `<name>-linux-x86_64`: `cargo zigbuild` for `x86_64-unknown-linux-gnu`, pinned to glibc 2.35 so it runs on non-NixOS systems.
     - `<name>-windows-x86_64.exe`: `cargo xwin build` for `x86_64-pc-windows-msvc`. The MSVC CRT/SDK is downloaded into `~/.cache/cargo-xwin` on first use.
   - macOS is not built. Windows icon and version resources are not embedded.
   - Repos with libraries but no such binary (craft-libs, craft-fonts, egui) get their workspace libraries compiled for both targets and are not released.
   - Repos with neither are only synced.
   - sccache hit/miss stats are printed at the end of the run.
3. **Release**, per built app:
   - Only repos owned by `FORK_OWNER` and built for both targets are released.
   - The release tag is `v<version>-forked`, from the binary's version in `cargo metadata`, targeting the built commit.
   - An existing release for that tag is updated in place: its tag is moved to the built commit, and assets and notes are replaced. If the tag already points at the built commit, nothing is done.

Non-fork repos in the lists are only cloned or fetched and fast-forwarded. A fork whose branch has diverged from upstream, an archived fork, or a local clone with diverged commits is reported as failed, not overwritten. Failures are listed at the end and the exit code is 1.

`--clean` stops the sccache server (if installed), then removes `CARGO_TARGET_DIR` and `SCCACHE_DIR`. It refuses to remove `/` or `$HOME`. `DEST_DIR`, `BUILD_DIR` and `CRAFT_FONTS_CACHE` are kept.

## Token

The GitHub token is the first non-empty of:

1. `GH_TOKEN`
2. `GITHUB_TOKEN`
3. the output of `gh auth token`

`gh` is only used for that last fallback. GitHub API calls are made directly and git operations go through libgit2 over HTTPS with the token, so no SSH key is needed. Your git config and the clones' remote URLs are unchanged. The `git` CLI is still used by cargo for git dependencies and by the tool for pinned-commit fetches and the craft-fonts worktree.

## Access checks

Before syncing, the tool checks and exits with the fix if one fails (not run for `--clean`):

- The token has the `repo`, `workflow` and `read:org` scopes (`admin:org` or `write:org` also satisfy `read:org`). Skipped, with a warning, for fine-grained or app tokens, which report no scopes.
- You are `FORK_OWNER` or an active member of that org. This also catches an org that restricts OAuth app access; approve the app under the org's Settings, Third-party access.
- You have write access (`WRITE`, `MAINTAIN` or `ADMIN`) to every fork in the lists. Forks not owned by `FORK_OWNER` are flagged as sync-only.

Most scope problems are fixed with:

```bash
gh auth refresh -h github.com -s repo,workflow,read:org
```
