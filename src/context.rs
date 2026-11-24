//! Shared run context for the sync phase.

use crate::config::Config;
use octocrab::Octocrab;

pub struct Ctx {
    pub cfg: Config,
    pub gh: Octocrab,
    pub token: String,
    /// Login of the authenticated user.
    pub gh_user: String,
}
