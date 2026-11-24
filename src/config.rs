//! Environment variables and defaults (spec §1, §2.2).

use crate::cli::{Mode, Options};
use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

pub const CRAFT_FONTS_REPO: &str = "storytold/craft-fonts";

#[derive(Clone, Debug)]
pub struct Config {
    pub home: PathBuf,
    pub build: bool,
    pub release: bool,
    pub force: bool,
    pub jobs: usize,
    /// `--only` names as given (case preserved).
    pub only: Vec<String>,
    pub dest_dir: PathBuf,
    pub build_dir: PathBuf,
    pub cargo_target_dir: PathBuf,
    /// Empty means no wrapper.
    pub rustc_wrapper: String,
    pub craft_fonts_cache: PathBuf,
    pub sccache_dir: PathBuf,
    pub star_list: String,
    /// Empty means read no libraries list.
    pub libs_star_list: String,
    pub fork_owner: String,
}

impl Config {
    pub fn from_env(opts: &Options) -> Result<Self> {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("HOME is not set."))?;
        Ok(Self::from_lookup(opts, &home, |k| std::env::var(k).ok()))
    }

    /// Build from an env lookup (injectable for tests).
    pub fn from_lookup(opts: &Options, home: &Path, get: impl Fn(&str) -> Option<String>) -> Self {
        let nonempty = |key: &str| get(key).filter(|v| !v.is_empty());
        let path_or = |key: &str, default: PathBuf| nonempty(key).map_or(default, PathBuf::from);
        let cache = home.join(".cache/craft-forks");
        let sccache_base =
            nonempty("XDG_CACHE_HOME").map_or_else(|| home.join(".cache"), PathBuf::from);

        Self {
            home: home.to_owned(),
            build: matches!(opts.mode, Mode::Build | Mode::Release),
            release: opts.mode == Mode::Release,
            force: opts.force,
            jobs: opts.jobs,
            only: opts.only.clone(),
            dest_dir: opts
                .dest_dir
                .clone()
                .unwrap_or_else(|| home.join("repos/craft-forks")),
            build_dir: path_or("BUILD_DIR", home.join("builds/craft-forks")),
            cargo_target_dir: path_or("CARGO_TARGET_DIR", cache.join("target")),
            rustc_wrapper: get("RUSTC_WRAPPER").unwrap_or_else(|| "sccache".into()),
            craft_fonts_cache: path_or("CRAFT_FONTS_CACHE", cache.join("craft-fonts")),
            sccache_dir: path_or("SCCACHE_DIR", sccache_base.join("sccache")),
            star_list: nonempty("STAR_LIST").unwrap_or_else(|| "zezura-archive-craft".into()),
            libs_star_list: get("LIBS_STAR_LIST")
                .unwrap_or_else(|| "zezura-archive-craft-libs".into()),
            fork_owner: nonempty("FORK_OWNER").unwrap_or_else(|| "Zezura-Archive".into()),
        }
    }

    /// Create, canonicalise and check the writable directories (spec §2.2).
    pub fn prepare_dirs(&mut self) -> Result<()> {
        self.dest_dir = writable_dir(&self.dest_dir, "destination directory")?;
        if self.build {
            self.build_dir = writable_dir(&self.build_dir, "build directory")?;
            self.cargo_target_dir = writable_dir(&self.cargo_target_dir, "cargo target directory")?;
        }
        Ok(())
    }
}

fn writable_dir(dir: &Path, what: &str) -> Result<PathBuf> {
    let not_writable = || format!("{what} is not writable: {}", dir.display());
    std::fs::create_dir_all(dir).with_context(not_writable)?;
    let resolved = dir.canonicalize().with_context(not_writable)?;
    let probe = resolved.join(format!(".craft-builder-probe-{}", std::process::id()));
    std::fs::File::create(&probe).with_context(not_writable)?;
    let _ = std::fs::remove_file(probe);
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(env: &[(&str, &str)]) -> Config {
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(&Options::default(), Path::new("/home/u"), |k| {
            env.get(k).cloned()
        })
    }

    #[test]
    fn defaults() {
        let c = config(&[]);
        assert_eq!(c.star_list, "zezura-archive-craft");
        assert_eq!(c.libs_star_list, "zezura-archive-craft-libs");
        assert_eq!(c.fork_owner, "Zezura-Archive");
        assert_eq!(c.dest_dir, Path::new("/home/u/repos/craft-forks"));
        assert_eq!(c.build_dir, Path::new("/home/u/builds/craft-forks"));
        assert_eq!(
            c.cargo_target_dir,
            Path::new("/home/u/.cache/craft-forks/target")
        );
        assert_eq!(
            c.craft_fonts_cache,
            Path::new("/home/u/.cache/craft-forks/craft-fonts")
        );
        assert_eq!(c.sccache_dir, Path::new("/home/u/.cache/sccache"));
        assert!(c.build && c.release);
    }

    #[test]
    fn libs_star_list_empty_means_none_unset_means_default() {
        assert_eq!(config(&[("LIBS_STAR_LIST", "")]).libs_star_list, "");
        assert_eq!(config(&[]).libs_star_list, "zezura-archive-craft-libs");
    }

    #[test]
    fn star_list_empty_means_default() {
        assert_eq!(
            config(&[("STAR_LIST", "")]).star_list,
            "zezura-archive-craft"
        );
        assert_eq!(config(&[("STAR_LIST", "mine")]).star_list, "mine");
    }

    #[test]
    fn rustc_wrapper_empty_stays_empty_unset_is_sccache() {
        assert_eq!(config(&[("RUSTC_WRAPPER", "")]).rustc_wrapper, "");
        assert_eq!(config(&[]).rustc_wrapper, "sccache");
        assert_eq!(
            config(&[("RUSTC_WRAPPER", "ccache")]).rustc_wrapper,
            "ccache"
        );
    }

    #[test]
    fn sccache_dir_follows_xdg_cache_home() {
        assert_eq!(
            config(&[("XDG_CACHE_HOME", "/x")]).sccache_dir,
            Path::new("/x/sccache")
        );
        assert_eq!(
            config(&[("SCCACHE_DIR", "/s")]).sccache_dir,
            Path::new("/s")
        );
    }
}
