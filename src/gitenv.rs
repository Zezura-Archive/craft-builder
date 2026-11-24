//! Environment for child processes (git, cargo), applied per `Command` (spec §2.5, §6.2).
//!
//! The process environment is never mutated. git2 ignores `GIT_CONFIG_*`; these
//! entries are for child `git` and for cargo's CLI git fetches.

use crate::config::Config;
use crate::tools::find_in_path;
use std::fmt;
use std::process::Command;

/// Env var carrying the token to the credential helper.
pub const TOKEN_VAR: &str = "CRAFT_BUILDER_GH_TOKEN";

const CREDENTIAL_HELPER: &str = r#"!f() { test "$1" = get && echo username=x-access-token && printf 'password=%s\n' "$CRAFT_BUILDER_GH_TOKEN"; }; f"#;

/// Cargo's helper when `gh` is installed: it finds the token itself, so cargo never sees ours.
const GH_CREDENTIAL_HELPER: &str = "!gh auth git-credential";

/// Variables cleared on every child unless set again with [`GitEnv::set`].
const CLEARED: [&str; 2] = ["CRAFT_FONTS_DIR", "CRAFT_FONTS_REQUIRED"];

#[derive(Clone)]
pub struct GitEnv {
    inherited_count: usize,
    token: String,
    cargo_uses_gh: bool,
    config: Vec<(String, String)>,
    vars: Vec<(String, String)>,
}

impl fmt::Debug for GitEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut vars: Vec<(&str, &str)> = self
            .vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        vars.push((TOKEN_VAR, "<redacted>"));
        f.debug_struct("GitEnv")
            .field("inherited_count", &self.inherited_count)
            .field("cargo_uses_gh", &self.cargo_uses_gh)
            .field("config", &self.config)
            .field("vars", &vars)
            .finish()
    }
}

impl GitEnv {
    /// Base env: token, HTTPS rewrites, cargo target dir and wrapper; cargo uses `gh` if installed.
    pub fn new(cfg: &Config, token: &str) -> Self {
        let inherited = std::env::var("GIT_CONFIG_COUNT")
            .ok()
            .and_then(|c| c.trim().parse().ok())
            .unwrap_or(0);
        Self::with_inherited_count(cfg, token, inherited, find_in_path("gh").is_some())
    }

    /// Like [`GitEnv::new`] with an explicit inherited `GIT_CONFIG_COUNT` and `gh` availability.
    pub fn with_inherited_count(
        cfg: &Config,
        token: &str,
        inherited_count: usize,
        gh_installed: bool,
    ) -> Self {
        let mut env = Self {
            inherited_count,
            token: token.to_owned(),
            cargo_uses_gh: gh_installed,
            config: Vec::new(),
            vars: Vec::new(),
        };
        env.push_config("url.https://github.com/.insteadOf", "git@github.com:");
        env.push_config("url.https://github.com/.insteadOf", "ssh://git@github.com/");
        env.set("CARGO_TARGET_DIR", cfg.cargo_target_dir.to_string_lossy());
        env.set("RUSTC_WRAPPER", &cfg.rustc_wrapper);
        env.set("SCCACHE_CACHE_SIZE", &cfg.sccache_cache_size);
        if cfg.build {
            env.set("CARGO_NET_GIT_FETCH_WITH_CLI", "true");
        }
        env
    }

    /// Append a `GIT_CONFIG_KEY_n`/`VALUE_n` pair after all earlier ones.
    pub fn push_config(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.config.push((key.into(), value.into()));
    }

    /// Set (or replace) an environment variable for children.
    pub fn set(&mut self, key: &str, value: impl AsRef<str>) {
        let value = value.as_ref().to_owned();
        match self.vars.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = value,
            None => self.vars.push((key.to_owned(), value)),
        }
    }

    /// Our config entries, in order.
    pub fn config_entries(&self) -> &[(String, String)] {
        &self.config
    }

    /// For git children: the token and the env-var credential helper.
    pub fn apply(&self, cmd: &mut Command) {
        self.apply_with(cmd, true);
    }

    /// For cargo children: `gh`'s credential helper if installed, else as [`GitEnv::apply`].
    pub fn apply_cargo(&self, cmd: &mut Command) {
        self.apply_with(cmd, !self.cargo_uses_gh);
    }

    fn apply_with(&self, cmd: &mut Command, env_helper: bool) {
        for var in CLEARED {
            cmd.env_remove(var);
        }
        cmd.envs(self.vars.iter().map(|(k, v)| (k, v)));
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        let helper = if env_helper {
            cmd.env(TOKEN_VAR, &self.token);
            CREDENTIAL_HELPER
        } else {
            GH_CREDENTIAL_HELPER
        };
        let helpers = [
            ("credential.https://github.com.helper", ""),
            ("credential.https://github.com.helper", helper),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        let config = self.config.iter().chain(&helpers);
        cmd.env(
            "GIT_CONFIG_COUNT",
            (self.inherited_count + self.config.len() + helpers.len()).to_string(),
        );
        for (i, (key, value)) in config.enumerate() {
            let n = self.inherited_count + i;
            cmd.env(format!("GIT_CONFIG_KEY_{n}"), key);
            cmd.env(format!("GIT_CONFIG_VALUE_{n}"), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Options;
    use std::path::Path;

    type Vars = std::collections::HashMap<String, Option<String>>;

    fn env_of(env: &GitEnv) -> Vars {
        vars_of(|cmd| env.apply(cmd))
    }

    fn cargo_env_of(env: &GitEnv) -> Vars {
        vars_of(|cmd| env.apply_cargo(cmd))
    }

    fn vars_of(apply: impl Fn(&mut Command)) -> Vars {
        let mut cmd = Command::new("true");
        apply(&mut cmd);
        cmd.get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect()
    }

    fn base_gh(inherited: usize, gh: bool) -> GitEnv {
        let cfg = Config::from_lookup(
            &Options::default(),
            &Default::default(),
            Path::new("/home/u"),
            |_| None,
        );
        GitEnv::with_inherited_count(&cfg, "tok", inherited, gh)
    }

    fn base(inherited: usize) -> GitEnv {
        base_gh(inherited, true)
    }

    #[test]
    fn numbering_continues_after_inherited_count() {
        let mut env = base(3);
        env.push_config("a.b", "c");
        let vars = env_of(&env);
        assert_eq!(vars["GIT_CONFIG_COUNT"].as_deref(), Some("8"));
        assert_eq!(
            vars["GIT_CONFIG_KEY_3"].as_deref(),
            Some("url.https://github.com/.insteadOf")
        );
        assert_eq!(
            vars["GIT_CONFIG_VALUE_3"].as_deref(),
            Some("git@github.com:")
        );
        assert_eq!(vars["GIT_CONFIG_KEY_5"].as_deref(), Some("a.b"));
        assert!(!vars.contains_key("GIT_CONFIG_KEY_2"));
        assert!(!vars.contains_key("GIT_CONFIG_KEY_8"));
    }

    #[test]
    fn base_vars_and_cleared_fonts_vars() {
        let vars = env_of(&base(0));
        assert_eq!(vars["GIT_CONFIG_COUNT"].as_deref(), Some("4"));
        assert_eq!(vars["GIT_TERMINAL_PROMPT"].as_deref(), Some("0"));
        assert_eq!(vars[TOKEN_VAR].as_deref(), Some("tok"));
        assert_eq!(vars["RUSTC_WRAPPER"].as_deref(), Some("sccache"));
        assert_eq!(vars["SCCACHE_CACHE_SIZE"].as_deref(), Some("30G"));
        assert_eq!(
            vars["CARGO_NET_GIT_FETCH_WITH_CLI"].as_deref(),
            Some("true")
        );
        assert_eq!(vars["CRAFT_FONTS_DIR"], None);
    }

    #[test]
    fn set_replaces() {
        let mut env = base(0);
        env.set("CRAFT_FONTS_DIR", "/a");
        env.set("CRAFT_FONTS_DIR", "/b");
        assert_eq!(env_of(&env)["CRAFT_FONTS_DIR"].as_deref(), Some("/b"));
    }

    #[test]
    fn git_children_get_token_and_env_helper() {
        let vars = env_of(&base(0));
        assert_eq!(vars[TOKEN_VAR].as_deref(), Some("tok"));
        assert_eq!(
            vars["GIT_CONFIG_VALUE_3"].as_deref(),
            Some(CREDENTIAL_HELPER)
        );
    }

    #[test]
    fn cargo_with_gh_gets_gh_helper_and_no_token() {
        let vars = cargo_env_of(&base(0));
        assert!(!vars.contains_key(TOKEN_VAR));
        assert_eq!(vars["GIT_CONFIG_VALUE_2"].as_deref(), Some(""));
        assert_eq!(
            vars["GIT_CONFIG_VALUE_3"].as_deref(),
            Some(GH_CREDENTIAL_HELPER)
        );
        assert_eq!(vars["GIT_CONFIG_COUNT"].as_deref(), Some("4"));
        assert_eq!(vars["GIT_TERMINAL_PROMPT"].as_deref(), Some("0"));
    }

    #[test]
    fn cargo_without_gh_falls_back_to_env_helper() {
        let vars = cargo_env_of(&base_gh(0, false));
        assert_eq!(vars[TOKEN_VAR].as_deref(), Some("tok"));
        assert_eq!(
            vars["GIT_CONFIG_VALUE_3"].as_deref(),
            Some(CREDENTIAL_HELPER)
        );
    }

    #[test]
    fn debug_redacts_token() {
        let text = format!("{:?}", base(0));
        assert!(!text.contains("tok\""), "{text}");
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn helper_prints_password_literally() {
        let out = Command::new("sh")
            .args([
                "-c",
                &format!("{} get", &CREDENTIAL_HELPER[1..]),
                "sh",
                "get",
            ])
            .env(TOKEN_VAR, "-n %s\\n")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "username=x-access-token\npassword=-n %s\\n\n"
        );
    }
}
