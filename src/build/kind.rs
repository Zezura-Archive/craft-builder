//! What a repo builds: binary, library, Windows-only library or nothing (spec §6.4).

use crate::gitenv::GitEnv;
use crate::report::Log;
use anyhow::{Context, Result, bail};
use cargo_metadata::{Metadata, Package, TargetKind};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildKind {
    /// A package named like the repo has a `bin` target of that name.
    Bin,
    /// Library targets, built with `--lib`.
    Lib,
    /// Every library package declares only `windows` docs.rs targets.
    WindowsLib,
    /// No binary or library targets.
    None,
}

/// `None` without a `Cargo.toml`, else [`read_metadata`] and [`classify`].
pub fn repo_build_kind(dir: &Path, name: &str, env: &GitEnv, log: &mut Log) -> Result<BuildKind> {
    if !dir.join("Cargo.toml").exists() {
        return Ok(BuildKind::None);
    }
    Ok(classify(&read_metadata(dir, env, log)?, name))
}

/// Run `cargo metadata --no-deps --format-version 1 --locked` in `dir` (all workspace members).
///
/// Uses `env.apply_cargo`; a nonzero exit is an error carrying cargo's stderr. Shared with release.
pub fn read_metadata(dir: &Path, env: &GitEnv, _log: &mut Log) -> Result<Metadata> {
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--no-deps", "--format-version", "1", "--locked"])
        .current_dir(dir)
        .stdin(Stdio::null());
    env.apply_cargo(&mut cmd);
    let out = cmd.output().context("failed to run cargo metadata")?;
    if !out.status.success() {
        bail!(
            "cargo metadata failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    }
    serde_json::from_slice(&out.stdout).context("failed to parse cargo metadata output")
}

/// Classify per the jq filter of spec §6.4.
///
/// `Bin` if a package named `name` has a target named `name` with kind `bin`.
/// Else, over packages with a `lib`/`rlib`/`proc-macro` target: none means `None`;
/// if every one has a non-empty `metadata.docs.rs.targets` whose entries all
/// contain `windows`, `WindowsLib`; else `Lib` (no docs.rs targets counts as `Lib`).
pub fn classify(meta: &Metadata, name: &str) -> BuildKind {
    let is_bin = meta
        .packages
        .iter()
        .filter(|p| p.name == name)
        .flat_map(|p| &p.targets)
        .any(|t| t.name == name && t.kind.contains(&TargetKind::Bin));
    if is_bin {
        return BuildKind::Bin;
    }
    let mut libs = meta
        .packages
        .iter()
        .filter(|p| has_lib_target(p))
        .peekable();
    if libs.peek().is_none() {
        BuildKind::None
    } else if libs.all(windows_only_docs_targets) {
        BuildKind::WindowsLib
    } else {
        BuildKind::Lib
    }
}

fn has_lib_target(pkg: &Package) -> bool {
    pkg.targets.iter().flat_map(|t| &t.kind).any(|k| {
        matches!(
            k,
            TargetKind::Lib | TargetKind::RLib | TargetKind::ProcMacro
        )
    })
}

/// `package.metadata.docs.rs.targets` is non-empty and every entry contains `windows`.
fn windows_only_docs_targets(pkg: &Package) -> bool {
    pkg.metadata
        .pointer("/docs/rs/targets")
        .and_then(|t| t.as_array())
        .is_some_and(|t| {
            !t.is_empty()
                && t.iter()
                    .all(|e| e.as_str().is_some_and(|s| s.contains("windows")))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn target(name: &str, kind: &str) -> Value {
        json!({"name": name, "kind": [kind], "crate_types": [kind], "src_path": "/x/src/lib.rs",
               "edition": "2021", "doctest": true, "test": true, "doc": true})
    }

    fn package(name: &str, targets: Vec<Value>, docs_targets: Option<Value>) -> Value {
        let metadata =
            docs_targets.map_or(Value::Null, |t| json!({"docs": {"rs": {"targets": t}}}));
        json!({"name": name, "version": "0.1.0", "id": format!("path+file:///x/{name}#0.1.0"),
               "source": null, "dependencies": [], "targets": targets, "features": {},
               "manifest_path": "/x/Cargo.toml", "metadata": metadata, "authors": [],
               "categories": [], "keywords": [], "edition": "2021", "publish": null})
    }

    fn meta(packages: Vec<Value>) -> Metadata {
        serde_json::from_value(
            json!({"packages": packages, "workspace_members": [], "workspace_default_members": [],
                   "resolve": null, "target_directory": "/x/target", "version": 1,
                   "workspace_root": "/x", "metadata": null}),
        )
        .unwrap()
    }

    #[test]
    fn bin() {
        let m = meta(vec![package("app", vec![target("app", "bin")], None)]);
        assert_eq!(classify(&m, "app"), BuildKind::Bin);
    }

    #[test]
    fn bin_with_other_name_is_not_bin() {
        let m = meta(vec![package("app", vec![target("tool", "bin")], None)]);
        assert_eq!(classify(&m, "app"), BuildKind::None);
    }

    #[test]
    fn lib_without_docs_targets() {
        let m = meta(vec![package("app", vec![target("app", "lib")], None)]);
        assert_eq!(classify(&m, "app"), BuildKind::Lib);
    }

    #[test]
    fn windows_only_docs_targets() {
        let docs = json!(["x86_64-pc-windows-msvc", "i686-pc-windows-msvc"]);
        let m = meta(vec![package("w", vec![target("w", "rlib")], Some(docs))]);
        assert_eq!(classify(&m, "w"), BuildKind::WindowsLib);
    }

    #[test]
    fn mixed_docs_targets_is_lib() {
        let win = json!(["x86_64-pc-windows-msvc"]);
        let mixed = json!(["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]);
        let m = meta(vec![
            package("a", vec![target("a", "lib")], Some(win.clone())),
            package("b", vec![target("b", "proc-macro")], Some(mixed)),
        ]);
        assert_eq!(classify(&m, "a"), BuildKind::Lib);
        let m = meta(vec![
            package("a", vec![target("a", "lib")], Some(win)),
            package("b", vec![target("b", "lib")], None),
        ]);
        assert_eq!(classify(&m, "a"), BuildKind::Lib);
    }

    #[test]
    fn no_lib_targets_is_none() {
        let m = meta(vec![package("t", vec![target("t", "test")], None)]);
        assert_eq!(classify(&m, "other"), BuildKind::None);
    }

    #[test]
    fn read_metadata_and_classify_on_temp_crate() {
        let dir = std::env::temp_dir().join(format!("craft-builder-kind-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let cfg = crate::config::Config::from_lookup(
            &crate::cli::Options::default(),
            Path::new("/home/u"),
            |_| None,
        );
        let env = GitEnv::new(&cfg, "tok");
        let kind = repo_build_kind(&dir, "demo", &env, &mut Log::live());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(kind.unwrap(), BuildKind::Bin);
    }
}
