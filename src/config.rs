//! Config file, environment variables and defaults (spec §1, §2.2).

use crate::cli::{Mode, Options};
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
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

/// `config.toml`: directories that override the defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub build_dir: Option<PathBuf>,
    pub clone_dir: Option<PathBuf>,
}

impl FileConfig {
    /// Read `--config`, else `$XDG_CONFIG_HOME/craft-builder/config.toml` if it exists.
    pub fn load(
        explicit: Option<&Path>,
        home: &Path,
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let path = match explicit {
            Some(path) => path.to_owned(),
            None => {
                let base = get("XDG_CONFIG_HOME")
                    .filter(|v| !v.is_empty())
                    .map_or_else(|| home.join(".config"), PathBuf::from);
                let path = base.join("craft-builder/config.toml");
                if !path.exists() {
                    return Ok(Self::default());
                }
                path
            }
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow!("cannot read config file {}: {e}", path.display()))?;
        let file: Self = toml::from_str(&text)
            .map_err(|e| anyhow!("invalid config file {}: {e}", path.display()))?;
        let expand = |dir: Option<PathBuf>| dir.map(|d| expand_tilde(&d, home));
        Ok(Self {
            build_dir: expand(file.build_dir),
            clone_dir: expand(file.clone_dir),
        })
    }
}

/// Replace a leading `~` with `home`.
fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_owned(),
    }
}

impl Config {
    pub fn from_env(opts: &Options) -> Result<Self> {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("HOME is not set."))?;
        let get = |k: &str| std::env::var(k).ok();
        let file = FileConfig::load(opts.config.as_deref(), &home, get)?;
        Ok(Self::from_lookup(opts, &file, &home, get))
    }

    /// Build from a config file and an env lookup (injectable for tests).
    /// Precedence: command line, then environment, then config file, then default.
    pub fn from_lookup(
        opts: &Options,
        file: &FileConfig,
        home: &Path,
        get: impl Fn(&str) -> Option<String>,
    ) -> Self {
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
                .or_else(|| file.clone_dir.clone())
                .unwrap_or_else(|| home.join("repos/craft-apps")),
            build_dir: path_or(
                "BUILD_DIR",
                file.build_dir
                    .clone()
                    .unwrap_or_else(|| home.join("builds/craft-apps")),
            ),
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
        config_with(&FileConfig::default(), env)
    }

    fn config_with(file: &FileConfig, env: &[(&str, &str)]) -> Config {
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(&Options::default(), file, Path::new("/home/u"), |k| {
            env.get(k).cloned()
        })
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("craft-builder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_file_sets_dirs_and_expands_tilde() {
        let dir = scratch_dir("cfg");
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "build_dir = \"~/builds/craft-apps/\"\nclone_dir = \"/r/apps\"\n",
        )
        .unwrap();
        let file = FileConfig::load(Some(&path), Path::new("/home/u"), |_| None).unwrap();
        let c = config_with(&file, &[]);
        assert_eq!(c.build_dir, Path::new("/home/u/builds/craft-apps"));
        assert_eq!(c.dest_dir, Path::new("/r/apps"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_file_is_found_under_xdg_config_home() {
        let dir = scratch_dir("xdg");
        std::fs::create_dir_all(dir.join("craft-builder")).unwrap();
        std::fs::write(
            dir.join("craft-builder/config.toml"),
            "clone_dir = \"/r\"\n",
        )
        .unwrap();
        let xdg = dir.to_str().unwrap().to_owned();
        let file = FileConfig::load(None, Path::new("/home/u"), |k| {
            (k == "XDG_CONFIG_HOME").then(|| xdg.clone())
        })
        .unwrap();
        assert_eq!(file.clone_dir.as_deref(), Some(Path::new("/r")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_default_config_is_fine_missing_explicit_is_an_error() {
        let home = Path::new("/nonexistent-home");
        let file = FileConfig::load(None, home, |_| None).unwrap();
        assert!(file.build_dir.is_none() && file.clone_dir.is_none());
        assert!(FileConfig::load(Some(&home.join("c.toml")), home, |_| None).is_err());
    }

    #[test]
    fn config_file_rejects_unknown_keys() {
        let dir = scratch_dir("unknown");
        let path = dir.join("config.toml");
        std::fs::write(&path, "dest_dir = \"/x\"\n").unwrap();
        assert!(FileConfig::load(Some(&path), Path::new("/h"), |_| None).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn command_line_and_env_override_config_file() {
        let file = FileConfig {
            build_dir: Some("/f/build".into()),
            clone_dir: Some("/f/clone".into()),
        };
        let opts = Options {
            dest_dir: Some("/cli".into()),
            ..Options::default()
        };
        let c = Config::from_lookup(&opts, &file, Path::new("/h"), |k| {
            (k == "BUILD_DIR").then(|| "/env".to_owned())
        });
        assert_eq!(c.dest_dir, Path::new("/cli"));
        assert_eq!(c.build_dir, Path::new("/env"));
    }

    #[test]
    fn defaults() {
        let c = config(&[]);
        assert_eq!(c.star_list, "zezura-archive-craft");
        assert_eq!(c.libs_star_list, "zezura-archive-craft-libs");
        assert_eq!(c.fork_owner, "Zezura-Archive");
        assert_eq!(c.dest_dir, Path::new("/home/u/repos/craft-apps"));
        assert_eq!(c.build_dir, Path::new("/home/u/builds/craft-apps"));
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
