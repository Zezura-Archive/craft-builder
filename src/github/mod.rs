//! GitHub API (octocrab).

pub mod access;
pub mod fork;
pub mod release;
pub mod stars;

use anyhow::Result;
use octocrab::Octocrab;

/// Authenticated client for api.github.com.
pub fn client(token: &str) -> Result<Octocrab> {
    Ok(Octocrab::builder()
        .personal_token(token.to_owned())
        .build()?)
}
