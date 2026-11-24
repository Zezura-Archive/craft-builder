# craft-builder

Syncs the GitHub forks listed in your star lists, cross-builds them for Linux and Windows, and releases the binaries.

craft-builder itself runs only on Linux (`x86_64-linux`).

## Build

```bash
nix build
```

## Install

Run it without installing:

```bash
nix run . -- --help
```

or install the built package from `./result/bin/craft-builder`. Both wrap the binary with the build tools on `PATH`.

## Usage

```bash
nix run . -- [options] [DEST_DIR]
```

It needs a GitHub token (`GH_TOKEN`, `GITHUB_TOKEN`, or `gh auth login`). Options, the config file, environment variables, phases and access requirements are in [docs/usage.md](docs/usage.md).

## Development

```bash
nix develop -c cargo build
```

```bash
nix develop -c cargo test
```
