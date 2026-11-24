//! Output sink, failure list and final summary (spec §4, §9).

use crate::config::Config;
use std::fmt::Display;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, ExitCode, ExitStatus, Stdio};

/// Failure entries such as `owner/name (clone)`, in occurrence order.
pub type Failures = Vec<String>;

enum Sink {
    Live,
    Buffered(String),
}

/// Output sink for one repo's work: live to stdout/stderr, or buffered (merged) for `-j > 1`.
pub struct Log(Sink);

impl Log {
    pub fn live() -> Self {
        Self(Sink::Live)
    }

    pub fn buffered() -> Self {
        Self(Sink::Buffered(String::new()))
    }

    /// Print a line to stdout.
    pub fn out(&mut self, msg: impl Display) {
        match &mut self.0 {
            Sink::Live => println!("{msg}"),
            Sink::Buffered(buf) => push_line(buf, &msg),
        }
    }

    /// Print a line to stderr (merged into the buffer when buffered).
    pub fn err(&mut self, msg: impl Display) {
        match &mut self.0 {
            Sink::Live => eprintln!("{msg}"),
            Sink::Buffered(buf) => push_line(buf, &msg),
        }
    }

    /// Print a blank line then `==== title ====`.
    pub fn header(&mut self, title: impl Display) {
        self.out("");
        self.out(format_args!("==== {title} ===="));
    }

    /// Run a child with stdin closed: output streams live, or is captured into the buffer.
    pub fn run(&mut self, cmd: &mut Command) -> io::Result<ExitStatus> {
        cmd.stdin(Stdio::null());
        match &mut self.0 {
            Sink::Live => cmd.status(),
            Sink::Buffered(buf) => {
                let out = cmd.output()?;
                buf.push_str(&String::from_utf8_lossy(&out.stdout));
                buf.push_str(&String::from_utf8_lossy(&out.stderr));
                Ok(out.status)
            }
        }
    }

    /// Append an already-captured block: printed when live, added to the buffer otherwise.
    pub fn write_block(&mut self, block: &str) {
        match &mut self.0 {
            Sink::Live => print_block(block),
            Sink::Buffered(buf) => buf.push_str(block),
        }
    }

    /// The buffered text (empty for a live log).
    pub fn into_buffer(self) -> String {
        match self.0 {
            Sink::Live => String::new(),
            Sink::Buffered(buf) => buf,
        }
    }
}

fn push_line(buf: &mut String, msg: &dyn Display) {
    buf.push_str(&msg.to_string());
    buf.push('\n');
}

/// Print a buffered block to stdout in one write.
pub fn print_block(block: &str) {
    let _ = io::stdout().lock().write_all(block.as_bytes());
}

/// Print the final summary and return the exit code (1 if any failure).
pub fn summary(cfg: &Config, processed: usize, failures: &Failures) -> ExitCode {
    println!();
    println!(
        "Processed {processed} repo(s): sources in {}",
        cfg.dest_dir.display()
    );
    if cfg.build {
        println!(
            "Binaries in {} (macOS skipped: no cross toolchain from Linux)",
            cfg.build_dir.display()
        );
        if Path::new(&cfg.rustc_wrapper)
            .file_name()
            .is_some_and(|n| n == "sccache")
        {
            println!();
            let _ = io::stdout().flush();
            let _ = Command::new(&cfg.rustc_wrapper)
                .arg("--show-stats")
                .status();
        }
    }
    if failures.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!("The following steps failed:");
    for entry in failures {
        eprintln!("  {entry}");
    }
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_log_merges_streams_in_order() {
        let mut log = Log::buffered();
        log.out("a");
        log.err("b");
        log.header("x");
        assert_eq!(log.into_buffer(), "a\nb\n\n==== x ====\n");
    }

    #[test]
    fn buffered_log_appends_block() {
        let mut log = Log::buffered();
        log.out("a");
        log.write_block("b\nc\n");
        assert_eq!(log.into_buffer(), "a\nb\nc\n");
    }

    #[test]
    fn buffered_run_captures_child_output() {
        let mut log = Log::buffered();
        let status = log
            .run(Command::new("sh").args(["-c", "echo hi; echo oops >&2"]))
            .unwrap();
        assert!(status.success());
        assert_eq!(log.into_buffer(), "hi\noops\n");
    }
}
