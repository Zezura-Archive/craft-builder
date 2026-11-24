//! One cargo build for one target (spec §6.8).

use super::kind::BuildKind;
use crate::config::Config;
use crate::git;
use crate::gitenv::GitEnv;
use crate::report::Log;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

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
/// with `env.apply_cargo` plus `target.extra_env`. Selection: `-p NAME --bin NAME` for `Bin`,
/// else `--lib` plus `--workspace` unless the root `Cargo.toml` has a line starting
/// `default-members`. `--locked` iff `git::is_tracked(dir, "Cargo.lock")`. Prints
/// `Building {name} for {label}...`. Lib kinds then print `Built {name}'s libraries for
/// {label}.` and return. Bin: `<cargo_target_dir>/<triple>/release/<name><suffix>` must
/// exist (`build succeeded but {out} is missing.`), is copied to
/// `<build_dir>/<name>/<name>-<label><suffix>`, then print `Built: <path>`.
pub fn build_target(
    cfg: &Config,
    env: &GitEnv,
    kind: BuildKind,
    dir: &Path,
    name: &str,
    target: &Target,
    log: &mut Log,
) -> Result<()> {
    let workspace = !std::fs::read_to_string(dir.join("Cargo.toml"))
        .unwrap_or_default()
        .lines()
        .any(|line| line.starts_with("default-members"));
    let locked = git::is_tracked(dir, "Cargo.lock").unwrap_or(false);
    let args = cargo_args(target, kind, name, workspace, locked);

    log.out(format_args!("Building {name} for {}...", target.label));
    let mut cmd = Command::new("cargo");
    cmd.args(&args).current_dir(dir);
    env.apply_cargo(&mut cmd);
    cmd.envs(target.extra_env.iter().copied());
    let status = log.run(&mut cmd).context("failed to run cargo")?;
    if !status.success() {
        bail!("cargo {} failed ({status})", args[0]);
    }

    if kind != BuildKind::Bin {
        log.out(format_args!(
            "Built {name}'s libraries for {}.",
            target.label
        ));
        return Ok(());
    }

    let file = format!("{name}{}", target.suffix);
    let out = cfg
        .cargo_target_dir
        .join(target.triple)
        .join("release")
        .join(&file);
    if !out.is_file() {
        bail!("build succeeded but {} is missing.", out.display());
    }
    let dest_dir = cfg.build_dir.join(name);
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(format!("{name}-{}{}", target.label, target.suffix));
    std::fs::copy(&out, &dest)
        .with_context(|| format!("failed to copy {} to {}", out.display(), dest.display()))?;
    log.out(format_args!("Built: {}", dest.display()));
    Ok(())
}

/// Cargo arguments: the target's subcommand, `--release`, `--locked` if asked, the selection.
fn cargo_args(
    target: &Target,
    kind: BuildKind,
    name: &str,
    workspace: bool,
    locked: bool,
) -> Vec<String> {
    let mut args: Vec<String> = target.cargo_args.iter().map(|a| a.to_string()).collect();
    args.push("--release".into());
    if locked {
        args.push("--locked".into());
    }
    if kind == BuildKind::Bin {
        args.extend(["-p", name, "--bin", name].map(String::from));
    } else {
        args.push("--lib".into());
        if workspace {
            args.push("--workspace".into());
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_selection_locked() {
        assert_eq!(
            cargo_args(&LINUX, BuildKind::Bin, "app", true, true),
            [
                "zigbuild",
                "--target",
                "x86_64-unknown-linux-gnu.2.35",
                "--release",
                "--locked",
                "-p",
                "app",
                "--bin",
                "app"
            ]
        );
    }

    #[test]
    fn lib_with_workspace_unlocked() {
        assert_eq!(
            cargo_args(&WINDOWS, BuildKind::Lib, "l", true, false),
            [
                "xwin",
                "build",
                "--target",
                "x86_64-pc-windows-msvc",
                "--release",
                "--lib",
                "--workspace"
            ]
        );
    }

    #[test]
    fn lib_with_default_members_has_no_workspace() {
        let args = cargo_args(&WINDOWS, BuildKind::WindowsLib, "l", false, true);
        assert_eq!(args[4..], ["--release", "--locked", "--lib"]);
    }
}
