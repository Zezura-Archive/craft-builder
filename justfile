set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

# List recipes
default:
    @just --list

# Sync, build and release; extra options pass through (e.g. --only filmcraft -j 8)
release *args:
    nix run . -- --release "$@"

# Sync, then build; no releases
build *args:
    nix run . -- --build "$@"

# Sync only
sync *args:
    nix run . -- --sync "$@"

# Delete CARGO_TARGET_DIR and SCCACHE_DIR
[confirm]
clean:
    nix run . -- --clean
