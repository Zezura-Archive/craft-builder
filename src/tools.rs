//! Required external commands (spec §2.3).

use crate::config::Config;
use anyhow::{Result, bail};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Find `name` on PATH (or check it directly if it contains a `/`).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let is_exec = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if name.contains('/') {
        return is_exec(Path::new(name)).then(|| PathBuf::from(name));
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| is_exec(p))
}

/// Fail on the first missing command: `git`, plus the build toolchain when building.
pub fn require_commands(cfg: &Config) -> Result<()> {
    let mut needed = vec!["git"];
    if cfg.build {
        needed.extend(["cargo", "cargo-zigbuild", "zig", "cargo-xwin"]);
        if !cfg.rustc_wrapper.is_empty() {
            needed.push(&cfg.rustc_wrapper);
        }
    }
    for cmd in needed {
        if find_in_path(cmd).is_none() {
            bail!(
                "required command '{cmd}' not found in PATH (use the Nix package or devShell of craft-builder)."
            );
        }
    }
    Ok(())
}
