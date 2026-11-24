//! One cargo build for one target (spec §6.8).

use super::kind::BuildKind;
use crate::config::Config;
use crate::gitenv::GitEnv;
use crate::report::Log;
use anyhow::Result;
use std::path::Path;

/// A build target and how to invoke cargo for it.
pub struct Target {
    /// Asset label, e.g. `linux-x86_64`.
    pub label: &'static str,
    /// Directory name under the cargo target dir (no glibc suffix).
    pub triple: &'static str,
    /// Binary suffix, `""` or `.exe`.
    pub suffix: &'static str,
    /// Cargo subcommand and target, before `--release`.
    pub cargo_args: &'static [&'static str],
    /// Extra env for this target's build only.
    pub extra_env: &'static [(&'static str, &'static str)],
}

pub const LINUX: Target = Target {
    label: "linux-x86_64",
    triple: "x86_64-unknown-linux-gnu",
    suffix: "",
    cargo_args: &["zigbuild", "--target", "x86_64-unknown-linux-gnu.2.35"],
    extra_env: &[(
        "CFLAGS_x86_64_unknown_linux_gnu",
        "-fno-function-sections -fno-data-sections",
    )],
};

pub const WINDOWS: Target = Target {
    label: "windows-x86_64",
    triple: "x86_64-pc-windows-msvc",
    suffix: ".exe",
    cargo_args: &["xwin", "build", "--target", "x86_64-pc-windows-msvc"],
    extra_env: &[],
};

/// Build `dir` for `target`; copy the binary to `build_dir/<name>/` for `Bin` repos.
///
/// Spec §6.8. Runs in `dir`: `cargo <target.cargo_args> --release [--locked] <selection>`
/// with `env.apply` plus `target.extra_env`. Selection: `-p NAME --bin NAME` for `Bin`,
/// else `--lib` plus `--workspace` unless the root `Cargo.toml` has a line starting
/// `default-members`. `--locked` iff `git::is_tracked(dir, "Cargo.lock")`. Prints
/// `Building {name} for {label}...`. Lib kinds then print `Built {name}'s libraries for
/// {label}.` and return. Bin: `<cargo_target_dir>/<triple>/release/<name><suffix>` must
/// exist (`build succeeded but {out} is missing.`), is copied to
/// `<build_dir>/<name>/<name>-<label><suffix>`, then print `Built: <path>`.
pub fn build_target(
    _cfg: &Config,
    _env: &GitEnv,
    _kind: BuildKind,
    _dir: &Path,
    _name: &str,
    _target: &Target,
    _log: &mut Log,
) -> Result<()> {
    todo!("spec §6.8")
}
