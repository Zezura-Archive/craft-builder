//! The craft-fonts checkout some repos embed (spec §6.6, §6.7).

use crate::clean;
use crate::config::Config;
use crate::git;
use crate::gitenv::GitEnv;
use crate::report::Log;
use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

const CRAFT_FONTS_REPO: &str = "storytold/craft-fonts";

/// The craft-fonts ref `dir` releases with, from `.github/workflows/release.yml`.
///
/// `Ok(None)` if the workflow is missing or has no ref (nothing is printed).
pub fn craft_fonts_ref(dir: &Path) -> Result<Option<String>> {
    let workflow = dir.join(".github/workflows/release.yml");
    if !workflow.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&workflow)
        .with_context(|| format!("failed to read {}", workflow.display()))?;
    resolve_ref(&text, &workflow)
}

/// Find and clean the craft-fonts ref in workflow `text` (spec §6.6, regexes and order verbatim).
///
/// `workflow` is only for messages. Errors: `{workflow} uses craft-fonts ref {ref}, but
/// defines no {var}.` and `can't resolve craft-fonts ref '{ref}' in {workflow}.`
pub fn resolve_ref(text: &str, workflow: &Path) -> Result<Option<String>> {
    const VALUE: &str = r"\s*(.*\S)";
    static INPUT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"^\s*craft-fonts-ref:{VALUE}")).unwrap());
    static REPO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"repository:\s*storytold/craft-fonts(?:[\s#]|$)").unwrap());
    static REF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"^\s*ref:{VALUE}")).unwrap());
    static STEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*-\s").unwrap());
    static ENV_REF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\$\{\{\s*env\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}$").unwrap());

    let lines: Vec<&str> = text.lines().collect();
    let mut found = None;
    let mut in_fonts_step = false;
    for line in &lines {
        if let Some(c) = INPUT.captures(line) {
            found = Some(c[1].to_owned());
            break;
        }
        if STEP.is_match(line) {
            in_fonts_step = false;
        }
        if REPO.is_match(line) {
            in_fonts_step = true;
        } else if in_fonts_step && let Some(c) = REF.captures(line) {
            found = Some(c[1].to_owned());
            break;
        }
    }
    let Some(raw) = found else { return Ok(None) };

    let mut git_ref = clean_ref(&raw).to_owned();
    if let Some(c) = ENV_REF.captures(&git_ref) {
        let var = &c[1];
        let var_re = Regex::new(&format!(r#"^\s*{var}:\s*["']?([^\s"'#]+)"#)).unwrap();
        let resolved = lines
            .iter()
            .find_map(|line| var_re.captures(line).map(|c| c[1].to_owned()));
        let Some(resolved) = resolved else {
            bail!(
                "{} uses craft-fonts ref {git_ref}, but defines no {var}.",
                workflow.display()
            );
        };
        git_ref = resolved;
    }

    static SAFE_REF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9A-Za-z][0-9A-Za-z._/-]{0,199}$").unwrap());
    if !SAFE_REF.is_match(&git_ref) {
        bail!(
            "can't resolve craft-fonts ref '{git_ref}' in {}.",
            workflow.display()
        );
    }
    Ok(Some(git_ref))
}

/// Drop a trailing ` #comment`, then one pair of `"` and one pair of `'`.
fn clean_ref(raw: &str) -> &str {
    let mut r = raw;
    if let Some(i) = first_comment(r) {
        r = &r[..i];
    }
    for quote in ['"', '\''] {
        r = r.strip_prefix(quote).unwrap_or(r);
        r = r.strip_suffix(quote).unwrap_or(r);
    }
    r
}

/// Byte index of the first whitespace character directly followed by `#`.
fn first_comment(s: &str) -> Option<usize> {
    s.char_indices()
        .find(|&(i, c)| c.is_whitespace() && s[i + c.len_utf8()..].starts_with('#'))
        .map(|(i, _)| i)
}

/// Point `cfg.craft_fonts_cache` at `git_ref` of storytold/craft-fonts (spec §6.7).
///
/// Recreate the cache when it is not a git checkout, or `dest_dir/craft-fonts` is a clone
/// whose git common dir differs: a detached `git worktree add` of that clone (after
/// `worktree prune`), else `git clone` of https://github.com/storytold/craft-fonts.git.
/// Fetch the ref from origin unless it is a local 40-hex commit (then use `FETCH_HEAD`'s
/// sha), then `git -c advice.detachedHead=false checkout --quiet --force --detach`.
/// Uses the git CLI through `env.apply`; prints the messages of §6.7 and
/// `Using storytold/craft-fonts@{ref}`. The caller sets `CRAFT_FONTS_DIR`/`_REQUIRED`.
pub fn prepare_craft_fonts(cfg: &Config, env: &GitEnv, git_ref: &str, log: &mut Log) -> Result<()> {
    let dir = &cfg.craft_fonts_cache;
    let synced = cfg.dest_dir.join("craft-fonts");

    let want_git = if synced.join(".git").is_dir() {
        Some(git::git_common_dir(&synced).context("failed to find the git dir of craft-fonts")?)
    } else {
        None
    };
    let have_git = dir
        .join(".git")
        .exists()
        .then(|| git::git_common_dir(dir))
        .flatten();

    if have_git.is_none() || (want_git.is_some() && have_git != want_git) {
        recreate(
            cfg,
            dir,
            want_git.is_some().then_some(synced.as_path()),
            env,
            log,
        )?;
    }

    let mut git_ref = git_ref.to_owned();
    let is_local_sha = git_ref.len() == 40
        && git_ref
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && git::has_commit(dir, &git_ref);
    if !is_local_sha {
        run_git(
            dir,
            &["fetch", "--quiet", "--", "origin", &git_ref],
            env,
            log,
        )?;
        git_ref = git_output(dir, &["rev-parse", "FETCH_HEAD"], env)?;
    }

    run_git(
        dir,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "--force",
            "--detach",
            &git_ref,
        ],
        env,
        log,
    )?;
    log.out(format!("Using {CRAFT_FONTS_REPO}@{git_ref}"));
    Ok(())
}

/// Replace `dir` with a detached worktree of `synced`, or else a fresh clone.
fn recreate(
    cfg: &Config,
    dir: &Path,
    synced: Option<&Path>,
    env: &GitEnv,
    log: &mut Log,
) -> Result<()> {
    let keep = [&*cfg.dest_dir, &cfg.build_dir, &cfg.cargo_target_dir];
    ensure!(
        !clean::is_protected(dir, &cfg.home, &keep),
        "refusing to remove craft-fonts cache at {}.",
        dir.display()
    );
    if dir.is_dir() {
        std::fs::remove_dir_all(dir)?;
    } else if dir.exists() {
        std::fs::remove_file(dir)?;
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(synced) = synced {
        log.out(format!(
            "Adding a worktree of {} at {}...",
            synced.display(),
            dir.display()
        ));
        run_git(synced, &["worktree", "prune"], env, log)?;
        let dir = dir.to_string_lossy();
        run_git(
            synced,
            &["worktree", "add", "--quiet", "--detach", &dir],
            env,
            log,
        )
    } else {
        log.out(format!(
            "Cloning {CRAFT_FONTS_REPO} into {}...",
            dir.display()
        ));
        let mut cmd = Command::new("git");
        cmd.args(["clone", "--quiet"])
            .arg(format!("https://github.com/{CRAFT_FONTS_REPO}.git"))
            .arg(dir);
        run(cmd, env, log)
    }
}

/// `git -C dir args...`, output through `log`.
fn run_git(dir: &Path, args: &[&str], env: &GitEnv, log: &mut Log) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    run(cmd, env, log)
}

fn run(mut cmd: Command, env: &GitEnv, log: &mut Log) -> Result<()> {
    env.apply(&mut cmd);
    let status = log.run(&mut cmd).context("failed to run git")?;
    ensure!(status.success(), "git failed ({status})");
    Ok(())
}

/// Trimmed stdout of `git -C dir args...`.
fn git_output(dir: &Path, args: &[&str], env: &GitEnv) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    env.apply(&mut cmd);
    let out = cmd.output().context("failed to run git")?;
    ensure!(
        out.status.success(),
        "git {} failed ({})",
        args[0],
        out.status
    );
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn resolve(text: &str) -> Result<Option<String>> {
        resolve_ref(text, &PathBuf::from("wf.yml"))
    }

    fn ok(text: &str) -> Option<String> {
        resolve(text).unwrap()
    }

    #[test]
    fn input_form() {
        assert_eq!(ok("  craft-fonts-ref: abc  \n"), Some("abc".into()));
        assert_eq!(ok("craft-fonts-ref:\nx: y"), None);
    }

    #[test]
    fn ref_inside_craft_fonts_step() {
        let wf = format!(
            "steps:\n  - uses: actions/checkout@v4\n    with:\n      ref: other\n  - uses: actions/checkout@v4\n    with:\n      repository: storytold/craft-fonts\n      ref: {SHA}\n"
        );
        assert_eq!(ok(&wf), Some(SHA.into()));
    }

    #[test]
    fn ref_before_repository_is_not_found() {
        let wf =
            "  - uses: x\n    with:\n      ref: abc\n      repository: storytold/craft-fonts\n";
        assert_eq!(ok(wf), None);
    }

    #[test]
    fn repository_on_the_step_line() {
        let wf = "  - repository: storytold/craft-fonts # fonts\n    ref: abc\n";
        assert_eq!(ok(wf), Some("abc".into()));
        assert_eq!(
            ok("  - repository: storytold/craft-fonts-x\n    ref: abc\n"),
            None
        );
    }

    #[test]
    fn next_step_ends_the_fonts_step() {
        let wf = "  - repository: storytold/craft-fonts\n  - name: n\n    ref: abc\n";
        assert_eq!(ok(wf), None);
    }

    #[test]
    fn strips_comments_and_quotes() {
        assert_eq!(ok("craft-fonts-ref: abc # pinned"), Some("abc".into()));
        assert_eq!(ok("craft-fonts-ref: \"abc\""), Some("abc".into()));
        assert_eq!(ok("craft-fonts-ref: 'abc' # c"), Some("abc".into()));
        assert!(resolve("craft-fonts-ref: a#b").is_err());
        assert!(resolve("craft-fonts-ref: \"a b\" # c").is_err());
    }

    #[test]
    fn resolves_env_reference() {
        let wf = format!(
            "env:\n  FONTS_SHA: \"{SHA}\" # pin\njobs:\n  ref: x\ncraft-fonts-ref: ${{{{ env.FONTS_SHA }}}}\n"
        );
        assert_eq!(ok(&wf), Some(SHA.into()));
    }

    #[test]
    fn missing_env_variable_errors() {
        let err = resolve("craft-fonts-ref: ${{ env.NOPE }}").unwrap_err();
        assert_eq!(
            err.to_string(),
            "wf.yml uses craft-fonts ref ${{ env.NOPE }}, but defines no NOPE."
        );
    }

    #[test]
    fn unresolvable_refs_error() {
        for value in [
            "${{ inputs.ref }}",
            "a b",
            "--upload-pack=x",
            "-x",
            "a;b",
            "a:b",
            "a$b",
            "a..b\u{e9}",
        ] {
            let err = resolve(&format!("craft-fonts-ref: {value}")).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("can't resolve craft-fonts ref '{value}' in wf.yml.")
            );
        }
    }

    #[test]
    fn safe_refs_pass_and_long_ones_fail() {
        for value in ["main", "v1.2.3", "feat/x_y-z", SHA, "0abc"] {
            assert_eq!(ok(&format!("craft-fonts-ref: {value}")), Some(value.into()));
        }
        assert!(resolve(&format!("craft-fonts-ref: a{}", "b".repeat(200))).is_err());
        assert!(resolve(&format!("craft-fonts-ref: a{}", "b".repeat(199))).is_ok());
    }

    #[test]
    fn recreate_refuses_protected_cache() {
        let cfg = crate::config::Config::from_lookup(
            &crate::cli::Options::default(),
            Path::new("/home/u"),
            |_| None,
        );
        let mut cfg_home = cfg.clone();
        cfg_home.craft_fonts_cache = cfg.home.clone();
        let env = GitEnv::with_inherited_count(&cfg, "tok", 0, true);
        for cache in [cfg.home.clone(), cfg.dest_dir.clone(), "/".into()] {
            cfg_home.craft_fonts_cache = cache.clone();
            let err = recreate(&cfg_home, &cache, None, &env, &mut Log::buffered()).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "refusing to remove craft-fonts cache at {}.",
                    cache.display()
                )
            );
        }
    }
}
