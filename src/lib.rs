//! craft-builder: sync, build and release the craft forks.
//!
//! Error convention: fallible steps return `anyhow::Result`. The error's
//! `Display` is the spec's message without its `Error: ` prefix; the caller
//! prints it (`Error: {e:#}`) and then its own `WARNING:` line.

#[cfg(not(target_os = "linux"))]
compile_error!("craft-builder runs only on Linux; it still builds forks for Linux and Windows.");

pub mod build;
pub mod clean;
pub mod cli;
pub mod config;
pub mod context;
pub mod git;
pub mod gitenv;
pub mod github;
pub mod repo;
pub mod report;
pub mod sync;
pub mod token;
pub mod tools;
