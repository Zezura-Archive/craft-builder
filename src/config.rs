//! Config file, environment variables and defaults (spec §1, §2.2).

use crate::cli::{Mode, Options};
use crate::repo::validate_name;
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::HashSet;
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
    /// sccache's cache size limit, e.g. `30G`.
    pub sccache_cache_size: String,
    /// Owner of the listed repos (`fork_owner` in the config file).
    pub fork_owner: String,
    /// Bare repo names in `fork_owner`.
    pub apps: Vec<String>,
    /// Bare repo names in `fork_owner`; synced before apps, never built.
    pub libraries: Vec<String>,
    /// The config file that was read, if any.
    pub config_path: Option<PathBuf>,
}

/// `config.toml`: the repos to process and directories that override the defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub build_dir: Option<PathBuf>,
    pub clone_dir: Option<PathBuf>,
    pub fork_owner: Option<String>,
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub libraries: Vec<String>,
    /// Where this was read from; not a key.
    #[serde(skip)]
    pub path: Option<PathBuf>,
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
        file.validate_repos()
            .map_err(|e| anyhow!("invalid config file {}: {e}", path.display()))?;
        Ok(Self {
            build_dir: expand(file.build_dir),
            clone_dir: expand(file.clone_dir),
            path: Some(path),
            ..file
        })
    }

    /// Names must be plain, and appear once across `apps` and `libraries`.
    fn validate_repos(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for name in self.libraries.iter().chain(&self.apps) {
            validate_name(name).map_err(|reason| anyhow!("'{name}': {reason}"))?;
            if !seen.insert(name.to_lowercase()) {
                bail!("'{name}' is listed more than once in apps and libraries");
            }
        }
        Ok(())
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
    /// Precedence: command line, then config file, then default; settings without a config key come from the environment.
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
            dest_dir: file
                .clone_dir
                .clone()
                .unwrap_or_else(|| home.join("repos/craft-apps")),
            build_dir: file
                .build_dir
                .clone()
                .unwrap_or_else(|| home.join("builds/craft-apps")),
            cargo_target_dir: path_or("CARGO_TARGET_DIR", cache.join("target")),
            rustc_wrapper: get("RUSTC_WRAPPER").unwrap_or_else(|| "sccache".into()),
            craft_fonts_cache: path_or("CRAFT_FONTS_CACHE", cache.join("craft-fonts")),
            sccache_dir: path_or("SCCACHE_DIR", sccache_base.join("sccache")),
            sccache_cache_size: nonempty("SCCACHE_CACHE_SIZE").unwrap_or_else(|| "30G".into()),
            fork_owner: file
                .fork_owner
                .clone()
                .unwrap_or_else(|| "Zezura-Archive".into()),
            apps: file.apps.clone(),
            libraries: file.libraries.clone(),
            config_path: file.path.clone(),
        }
    }

    /// Fail unless the config lists at least one app or library.
    pub fn ensure_repos_listed(&self) -> Result<()> {
        if !self.apps.is_empty() || !self.libraries.is_empty() {
            return Ok(());
        }
        let place = match &self.config_path {
            Some(path) => format!("The config file {} lists none", path.display()),
            None => "No config file was found (see --config)".to_owned(),
        };
        bail!(
            "{place}. Add `apps = [\"name\"]` and/or `libraries = [\"name\"]` (repo names in the org)."
        );
    }

    /// Create, canonicalise and check the writable directories (spec §2.2).
    pub fn prepare_dirs(&mut self) -> Result<()> {
        self.dest_dir = writable_dir(&self.dest_dir, "destination directory")?;
        if self.build {
            self.build_dir = writable_dir(&self.build_dir, "build directory")?;
            self.cargo_target_dir = writable_dir(&self.cargo_target_dir, "cargo target directory")?;
        }
        if std::env::var_os("WSL_DISTRO_NAME").is_some() {
            for (what, dir) in self.windows_drive_dirs() {
                eprintln!(
                    "WARNING: the {what} {} is on a Windows drive; git and cargo are much slower there than on the WSL filesystem.",
                    dir.display()
                );
            }
        }
        Ok(())
    }

    /// The source and cache dirs this run uses that are under `/mnt` (Windows drives on WSL).
    fn windows_drive_dirs(&self) -> Vec<(&'static str, &Path)> {
        let mut dirs = vec![("destination directory", self.dest_dir.as_path())];
        if self.build {
            dirs.push(("cargo target directory", &self.cargo_target_dir));
            dirs.push(("sccache directory", &self.sccache_dir));
        }
        dirs.retain(|(_, dir)| dir.starts_with("/mnt"));
        dirs
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
        for key in ["dest_dir", "org"] {
            std::fs::write(&path, format!("{key} = \"/x\"\n")).unwrap();
            assert!(FileConfig::load(Some(&path), Path::new("/h"), |_| None).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_file_sets_dirs_and_env_does_not() {
        let file = FileConfig {
            build_dir: Some("/f/build".into()),
            clone_dir: Some("/f/clone".into()),
            ..FileConfig::default()
        };
        let c = Config::from_lookup(&Options::default(), &file, Path::new("/h"), |k| {
            matches!(k, "BUILD_DIR" | "FORK_OWNER").then(|| "/env".to_owned())
        });
        assert_eq!(c.dest_dir, Path::new("/f/clone"));
        assert_eq!(c.build_dir, Path::new("/f/build"));
        assert_eq!(c.fork_owner, "Zezura-Archive");
    }

    #[test]
    fn defaults() {
        let c = config(&[]);
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
        assert_eq!(c.sccache_cache_size, "30G");
        assert_eq!(
            config(&[("SCCACHE_CACHE_SIZE", "50G")]).sccache_cache_size,
            "50G"
        );
        assert!(c.build && c.release);
    }

    fn load_text(name: &str, text: &str) -> Result<FileConfig> {
        let dir = scratch_dir(name);
        let path = dir.join("config.toml");
        std::fs::write(&path, text).unwrap();
        let file = FileConfig::load(Some(&path), Path::new("/h"), |_| None);
        std::fs::remove_dir_all(dir).unwrap();
        file
    }

    #[test]
    fn windows_drive_dirs_are_those_under_mnt() {
        let mut c = config(&[
            ("CARGO_TARGET_DIR", "/mnt/c/target"),
            ("SCCACHE_DIR", "/mnt2/s"),
        ]);
        let names = |c: &Config| -> Vec<&str> {
            c.windows_drive_dirs()
                .into_iter()
                .map(|(what, _)| what)
                .collect()
        };
        assert_eq!(names(&c), ["cargo target directory"]);
        c.build = false;
        assert!(names(&c).is_empty());
    }

    #[test]
    fn config_file_sets_fork_owner_apps_and_libraries() {
        let file = load_text(
            "repos",
            "fork_owner = \"Acme\"\napps = [\"filmcraft\", \"effectcraft\"]\nlibraries = [\"egui\"]\n",
        )
        .unwrap();
        let c = config_with(&file, &[]);
        assert_eq!(c.fork_owner, "Acme");
        assert_eq!(c.apps, ["filmcraft", "effectcraft"]);
        assert_eq!(c.libraries, ["egui"]);
        assert!(c.config_path.is_some());
        assert!(c.ensure_repos_listed().is_ok());
    }

    #[test]
    fn duplicate_or_overlapping_names_are_errors() {
        let err = load_text("dup", "apps = [\"a\", \"a\"]\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("'a' is listed more than once"), "{err}");
        assert!(load_text("overlap", "apps = [\"a\"]\nlibraries = [\"A\"]\n").is_err());
    }

    #[test]
    fn unsafe_names_are_errors() {
        for bad in ["o/a", "..", "a b", ""] {
            assert!(
                load_text("unsafe", &format!("apps = [\"{bad}\"]\n")).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn empty_lists_error_names_the_file_or_its_absence() {
        let none = config(&[]);
        let err = none.ensure_repos_listed().unwrap_err().to_string();
        assert!(err.starts_with("No config file was found"), "{err}");
        let file = FileConfig {
            path: Some("/c/config.toml".into()),
            ..FileConfig::default()
        };
        let err = config_with(&file, &[])
            .ensure_repos_listed()
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("/c/config.toml") && err.contains("apps"),
            "{err}"
        );
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
