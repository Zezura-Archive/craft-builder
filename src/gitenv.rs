//! Environment for child processes (git, cargo), applied per `Command` (spec §2.5, §6.2).
//!
//! The process environment is never mutated. git2 ignores `GIT_CONFIG_*`; these
//! entries are for child `git` and for cargo's CLI git fetches.

use crate::config::Config;
use std::process::Command;

/// Env var carrying the token to the credential helper.
pub const TOKEN_VAR: &str = "CRAFT_BUILDER_GH_TOKEN";

const CREDENTIAL_HELPER: &str = r#"!f() { test "$1" = get && echo username=x-access-token && echo "password=$CRAFT_BUILDER_GH_TOKEN"; }; f"#;

/// Variables cleared on every child unless set again with [`GitEnv::set`].
const CLEARED: [&str; 2] = ["CRAFT_FONTS_DIR", "CRAFT_FONTS_REQUIRED"];

#[derive(Clone, Debug)]
pub struct GitEnv {
    inherited_count: usize,
    config: Vec<(String, String)>,
    vars: Vec<(String, String)>,
}

impl GitEnv {
    /// Base env: token, HTTPS rewrites, credential helper, cargo target dir and wrapper.
    pub fn new(cfg: &Config, token: &str) -> Self {
        let inherited = std::env::var("GIT_CONFIG_COUNT")
            .ok()
            .and_then(|c| c.trim().parse().ok())
            .unwrap_or(0);
        Self::with_inherited_count(cfg, token, inherited)
    }

    /// Like [`GitEnv::new`] with an explicit inherited `GIT_CONFIG_COUNT`.
    pub fn with_inherited_count(cfg: &Config, token: &str, inherited_count: usize) -> Self {
        let mut env = Self {
            inherited_count,
            config: Vec::new(),
            vars: Vec::new(),
        };
        env.push_config("url.https://github.com/.insteadOf", "git@github.com:");
        env.push_config("url.https://github.com/.insteadOf", "ssh://git@github.com/");
        env.push_config("credential.https://github.com.helper", "");
        env.push_config("credential.https://github.com.helper", CREDENTIAL_HELPER);
        env.set(TOKEN_VAR, token);
        env.set("CARGO_TARGET_DIR", cfg.cargo_target_dir.to_string_lossy());
        env.set("RUSTC_WRAPPER", &cfg.rustc_wrapper);
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

    pub fn apply(&self, cmd: &mut Command) {
        for var in CLEARED {
            cmd.env_remove(var);
        }
        cmd.envs(self.vars.iter().map(|(k, v)| (k, v)));
        cmd.env(
            "GIT_CONFIG_COUNT",
            (self.inherited_count + self.config.len()).to_string(),
        );
        for (i, (key, value)) in self.config.iter().enumerate() {
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

    fn env_of(env: &GitEnv) -> std::collections::HashMap<String, Option<String>> {
        let mut cmd = Command::new("true");
        env.apply(&mut cmd);
        cmd.get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect()
    }

    fn base(inherited: usize) -> GitEnv {
        let cfg = Config::from_lookup(&Options::default(), Path::new("/home/u"), |_| None);
        GitEnv::with_inherited_count(&cfg, "tok", inherited)
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
        assert_eq!(vars["GIT_CONFIG_KEY_7"].as_deref(), Some("a.b"));
        assert!(!vars.contains_key("GIT_CONFIG_KEY_2"));
        assert!(!vars.contains_key("GIT_CONFIG_KEY_8"));
    }

    #[test]
    fn base_vars_and_cleared_fonts_vars() {
        let vars = env_of(&base(0));
        assert_eq!(vars["GIT_CONFIG_COUNT"].as_deref(), Some("4"));
        assert_eq!(vars[TOKEN_VAR].as_deref(), Some("tok"));
        assert_eq!(vars["RUSTC_WRAPPER"].as_deref(), Some("sccache"));
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
}
