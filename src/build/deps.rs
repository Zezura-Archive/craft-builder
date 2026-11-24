//! Cargo git dependencies redirected to local clones or forks (spec §6.2, §6.5).

use crate::gitenv::GitEnv;
use crate::repo::RepoItem;
use crate::report::Log;
use anyhow::{Context, Result, ensure};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Where git dependencies come from; lives across all repos of a run.
#[derive(Debug, Default)]
pub struct Sources {
    /// Upstream `owner/name` to the clone in `dest_dir`.
    pub local: HashMap<String, PathBuf>,
    /// Upstream `owner/name` to our fork, for forks without a clone.
    pub fork: HashMap<String, String>,
}

/// Add the `insteadOf` rewrites to `env` over ALL configured apps and libraries (spec §6.2) and print the notes.
///
/// A clone in `dest_dir` gets `file://` rewrites for the `.git` and plain URLs of its
/// upstream; a fork without a clone gets one rewrite to the fork.
pub fn setup_rewrites(
    items: &[RepoItem],
    dest_dir: &Path,
    env: &mut GitEnv,
    log: &mut Log,
) -> Sources {
    let mut sources = Sources::default();
    for item in items {
        let upstream = item.upstream();
        let clone = item.dest(dest_dir);
        if clone.join(".git").is_dir() {
            log.out(format!(
                "Fetching git dependencies on {upstream} from {}.",
                clone.display()
            ));
            let key = format!("url.file://{}.insteadOf", clone.display());
            env.push_config(&key, format!("https://github.com/{upstream}.git"));
            env.push_config(&key, format!("https://github.com/{upstream}"));
            sources.local.insert(upstream.to_owned(), clone);
        } else if item.is_fork {
            log.out(format!(
                "Fetching git dependencies on {upstream} from {}.",
                item.repo
            ));
            env.push_config(
                format!("url.https://github.com/{}.insteadOf", item.repo),
                format!("https://github.com/{upstream}"),
            );
            sources.fork.insert(upstream.to_owned(), item.repo.clone());
        }
    }
    sources
}

/// Make sure the commits pinned in `dir/Cargo.lock` exist in their redirected clones (spec §6.5).
///
/// No `Cargo.lock`: nothing. For each `source = "git+https://github.com/..#<40-hex>"` line
/// (regex verbatim in the spec), once per upstream/sha: if `sources.local` has the upstream
/// and `crate::git::has_commit` is false, queue the sha; else if no local or fork source and not
/// collect it as unredirected. Any unredirected source (GitHub or not) is an error naming
/// each one. Per clone with missing shas: print `Fetching N pinned commit(s) into {dir}...`,
/// then `crate::git::fetch_cli`. Never modifies `Cargo.lock`.
pub fn prepare_git_deps(
    sources: &mut Sources,
    dir: &Path,
    env: &GitEnv,
    log: &mut Log,
) -> Result<()> {
    let lock = dir.join("Cargo.lock");
    if !lock.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&lock)
        .with_context(|| format!("failed to read {}", lock.display()))?;

    let mut seen = HashSet::new();
    let mut unredirected: Vec<String> = Vec::new();
    let mut missing: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for (upstream, sha) in text.lines().filter_map(parse_git_source) {
        if !seen.insert((upstream.clone(), sha.clone())) {
            continue;
        }
        if let Some(clone) = sources.local.get(&upstream) {
            if crate::git::has_commit(clone, &sha) {
                continue;
            }
            match missing.iter_mut().find(|(d, _)| d == clone) {
                Some((_, shas)) => shas.push(sha),
                None => missing.push((clone.clone(), vec![sha])),
            }
        } else if !sources.fork.contains_key(&upstream) && !unredirected.contains(&upstream) {
            unredirected.push(upstream);
        }
    }
    ensure!(
        unredirected.is_empty(),
        "git dependencies would come from upstream: {}; add each one's fork to `libraries` in the config.",
        unredirected.join(", ")
    );

    for (clone, shas) in missing {
        log.out(format!(
            "Fetching {} pinned commit(s) into {}...",
            shas.len(),
            clone.display()
        ));
        let refs: Vec<&str> = shas.iter().map(String::as_str).collect();
        crate::git::fetch_cli(&clone, &refs, env, log)?;
    }
    Ok(())
}

/// `(upstream, sha)` of a Cargo.lock `source = "git+<url>#<sha>"` line.
///
/// A GitHub URL gives `owner/name`; any other URL is returned whole.
fn parse_git_source(line: &str) -> Option<(String, String)> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^source = "git\+([^?#"]+)(\?[^#]*)?#([0-9a-f]{40})"$"#).unwrap()
    });
    static GITHUB: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^https://github\.com/([^/]+/.+)$").unwrap());
    let caps = RE.captures(line)?;
    let url = caps[1].strip_suffix(".git").unwrap_or(&caps[1]);
    let upstream = GITHUB
        .captures(url)
        .map_or(url, |c| c.get(1).unwrap().as_str());
    Some((upstream.to_owned(), caps[3].to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Options;
    use crate::config::Config;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn parse(line: &str) -> Option<(String, String)> {
        parse_git_source(line)
    }

    #[test]
    fn parses_source_lines_and_strips_dot_git() {
        let want = Some(("o/r".to_owned(), SHA.to_owned()));
        for url in ["o/r", "o/r.git"] {
            let line = format!(r#"source = "git+https://github.com/{url}#{SHA}""#);
            assert_eq!(parse(&line), want, "{url}");
        }
        let line = format!(r#"source = "git+https://github.com/o/r?branch=main#{SHA}""#);
        assert_eq!(parse(&line), want);
    }

    #[test]
    fn ignores_other_lines() {
        for line in [
            r#"source = "registry+https://github.com/rust-lang/crates.io-index""#,
            r#"source = "git+https://github.com/o/r#0123""#,
            r#" source = "git+https://github.com/o/r#0123456789abcdef0123456789abcdef01234567""#,
            "",
        ] {
            assert_eq!(parse(line), None, "{line}");
        }
    }

    #[test]
    fn non_github_urls_are_kept_whole() {
        let line = format!(r#"source = "git+https://gitlab.com/o/r.git?rev=x#{SHA}""#);
        assert_eq!(
            parse(&line),
            Some(("https://gitlab.com/o/r".to_owned(), SHA.to_owned()))
        );
    }

    fn env() -> GitEnv {
        let cfg = Config::from_lookup(
            &Options::default(),
            &Default::default(),
            Path::new("/home/u"),
            |_| None,
        );
        GitEnv::with_inherited_count(&cfg, "tok", 0, true)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("craft-builder-deps-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_lock_is_fine_and_unredirected_sources_are_an_error() {
        let dir = temp_dir("lock");
        let mut sources = Sources::default();
        let mut log = Log::buffered();
        prepare_git_deps(&mut sources, &dir, &env(), &mut log).unwrap();
        assert_eq!(log.into_buffer(), "");

        let github = format!(r#"source = "git+https://github.com/o/r#{SHA}""#);
        let gitlab = format!(r#"source = "git+https://gitlab.com/o/x#{SHA}""#);
        std::fs::write(
            dir.join("Cargo.lock"),
            format!("{github}\n{github}\n{gitlab}\n"),
        )
        .unwrap();
        let err = prepare_git_deps(&mut sources, &dir, &env(), &mut Log::buffered())
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "git dependencies would come from upstream: o/r, https://gitlab.com/o/x; add each one's fork to `libraries` in the config."
        );
    }

    #[test]
    fn fork_source_is_fine() {
        let dir = temp_dir("fork");
        let line = format!(r#"source = "git+https://github.com/o/r#{SHA}""#);
        std::fs::write(dir.join("Cargo.lock"), line).unwrap();
        let mut sources = Sources::default();
        sources.fork.insert("o/r".into(), "me/r".into());
        let mut log = Log::buffered();
        prepare_git_deps(&mut sources, &dir, &env(), &mut log).unwrap();
        assert_eq!(log.into_buffer(), "");
    }
}
