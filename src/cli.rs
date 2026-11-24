//! Command line parsing and validation (spec §1).

use clap::{CommandFactory, Parser, error::ErrorKind};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Build,
    Release,
    Clean,
}

/// Validated command line.
#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    pub force: bool,
    /// `--only` names: split on `,`, whitespace stripped, empties dropped.
    pub only: Vec<String>,
    pub jobs: usize,
    /// `--config` file; `None` means the default location.
    pub config: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Release,
            force: false,
            only: Vec::new(),
            jobs: 1,
            config: None,
        }
    }
}

/// Sync every craft repo into `clone_dir`, build it for Linux and Windows, and release it.
#[derive(Parser, Debug)]
#[command(name = "craft-builder", version)]
struct Raw {
    /// Only sync.
    #[arg(long, overrides_with = "sync", conflicts_with_all = ["build", "release", "clean"])]
    sync: bool,
    /// Sync, then build.
    #[arg(long, overrides_with = "build", conflicts_with_all = ["sync", "release", "clean"])]
    build: bool,
    /// Sync, build, then release (default).
    #[arg(long, overrides_with = "release", conflicts_with_all = ["sync", "build", "clean"])]
    release: bool,
    /// Delete CARGO_TARGET_DIR and SCCACHE_DIR (build caches) only.
    #[arg(long, overrides_with = "clean", conflicts_with_all = ["sync", "build", "release"])]
    clean: bool,
    /// Hard-reset (GitHub and local) instead of fast-forward only.
    #[arg(short, long)]
    force: bool,
    /// Only process these repos (name, owner/name, or a fork's upstream owner/name).
    #[arg(long, value_name = "REPO[,REPO...]", allow_hyphen_values = true)]
    only: Vec<String>,
    /// Sync up to N repos in parallel; builds stay serial.
    #[arg(
        short,
        long,
        value_name = "N",
        default_value = "1",
        allow_hyphen_values = true
    )]
    jobs: String,
    /// Config file (default: ~/.config/craft-builder/config.toml).
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
}

impl Options {
    /// Parse `std::env::args`, exiting like the script: help 0, errors 1.
    pub fn parse() -> Self {
        match Self::try_parse_from(std::env::args_os()) {
            Ok(opts) => opts,
            Err(err) => {
                let code = i32::from(err.use_stderr());
                let _ = err.print();
                std::process::exit(code);
            }
        }
    }

    pub fn try_parse_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let raw = Raw::try_parse_from(args)?;
        let fail = |msg: String| Raw::command().error(ErrorKind::ValueValidation, msg);

        let mode = if raw.sync {
            Mode::Sync
        } else if raw.build {
            Mode::Build
        } else if raw.clean {
            Mode::Clean
        } else {
            Mode::Release
        };

        let jobs = parse_jobs(&raw.jobs).ok_or_else(|| {
            fail(format!(
                "--jobs needs a positive integer, got '{}'.",
                raw.jobs
            ))
        })?;

        let only_given = !raw.only.is_empty();
        let only = split_only(&raw.only);
        if only_given && only.is_empty() {
            return Err(fail("--only needs at least one repo name.".into()));
        }

        if mode == Mode::Clean && (raw.force || only_given || jobs != 1 || raw.config.is_some()) {
            return Err(fail("--clean takes no other options or arguments.".into()));
        }

        Ok(Self {
            mode,
            force: raw.force,
            only,
            jobs,
            config: raw.config,
        })
    }
}

/// `^[1-9][0-9]*$`
fn parse_jobs(s: &str) -> Option<usize> {
    let valid =
        s.starts_with(|c: char| ('1'..='9').contains(&c)) && s.bytes().all(|b| b.is_ascii_digit());
    if valid { s.parse().ok() } else { None }
}

fn split_only(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.split(','))
        .map(|piece| {
            piece
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        })
        .filter(|piece| !piece.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, clap::Error> {
        Options::try_parse_from(std::iter::once("craft-builder").chain(args.iter().copied()))
    }

    fn msg(args: &[&str]) -> String {
        parse(args).unwrap_err().to_string()
    }

    #[test]
    fn default_mode_is_release() {
        let o = parse(&[]).unwrap();
        assert_eq!(o.mode, Mode::Release);
        assert_eq!(o.jobs, 1);
    }

    #[test]
    fn same_mode_twice_is_fine() {
        assert_eq!(parse(&["--build", "--build"]).unwrap().mode, Mode::Build);
    }

    #[test]
    fn different_modes_conflict() {
        assert!(parse(&["--sync", "--build"]).is_err());
        assert!(parse(&["--release", "--clean"]).is_err());
    }

    #[test]
    fn jobs_must_be_positive_integer() {
        assert_eq!(parse(&["-j", "4"]).unwrap().jobs, 4);
        assert_eq!(parse(&["--jobs=8"]).unwrap().jobs, 8);
        for bad in ["0", "-1", "abc", "01", "", "1.5"] {
            assert!(
                msg(&["--jobs", bad]).contains("--jobs needs a positive integer"),
                "{bad}"
            );
        }
    }

    #[test]
    fn only_splits_and_strips_whitespace() {
        let o = parse(&["--only", "a/b, c d ,,e", "--only=f"]).unwrap();
        assert_eq!(o.only, ["a/b", "cd", "e", "f"]);
    }

    #[test]
    fn only_without_names_is_an_error() {
        assert!(msg(&["--only", " , ,"]).contains("--only needs at least one repo name."));
    }

    #[test]
    fn clean_takes_nothing_else() {
        assert_eq!(parse(&["--clean"]).unwrap().mode, Mode::Clean);
        assert_eq!(parse(&["--clean", "-j", "1"]).unwrap().mode, Mode::Clean);
        for args in [
            &["--clean", "-f"][..],
            &["--clean", "--only", "x"],
            &["--clean", "-j", "2"],
            &["--clean", "-c", "x.toml"],
        ] {
            assert!(msg(args).contains("--clean takes no other options or arguments."));
        }
    }

    #[test]
    fn config_short_and_long() {
        assert!(parse(&[]).unwrap().config.is_none());
        assert_eq!(
            parse(&["-c", "a.toml"]).unwrap().config,
            Some("a.toml".into())
        );
        assert_eq!(
            parse(&["--config=b.toml"]).unwrap().config,
            Some("b.toml".into())
        );
    }

    #[test]
    fn positional_arguments_are_rejected() {
        assert!(parse(&["dest"]).is_err());
        assert!(parse(&[""]).is_err());
        assert!(parse(&["--clean", "dest"]).is_err());
    }
}
