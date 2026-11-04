//! Settings › Usage (owner fix 2026-11-04): token / cost statistics over every
//! session file on a host, aggregated by the host. `usage_stats` → [`UsageData`].
use serde::{Deserialize, Serialize};

/// The ranges the page offers (days); any other request is clamped to `1..=USAGE_DAYS_MAX`.
pub const USAGE_DAYS_MAX: u32 = 366;
/// Models / projects listed at most.
pub const USAGE_ROWS_MAX: usize = 50;

/// Sums over the range.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    /// All four kinds together.
    pub tokens: f64,
    /// Priced spend (unpriced models add nothing).
    pub cost: f64,
    /// Tokens spent on models that report no price: `cost` leaves them out.
    pub unpriced_tokens: f64,
    /// Assistant replies counted.
    pub turns: u64,
    /// Sessions with at least one counted reply in the range.
    pub sessions: u64,
    /// Days with at least one counted reply.
    pub active_days: u32,
}

/// One calendar day (the client's local day), oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageDay {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub tokens: f64,
    pub cost: f64,
    pub unpriced_tokens: f64,
}

/// One model's share.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageModel {
    pub name: String,
    pub tokens: f64,
    pub cost: f64,
    pub unpriced_tokens: f64,
    pub turns: u64,
}

/// One project's (session folder's) share.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageProject {
    /// The folder the sessions ran in.
    pub name: String,
    pub tokens: f64,
    pub cost: f64,
    pub unpriced_tokens: f64,
    pub sessions: u64,
}

/// `usage_stats` → the range's statistics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageData {
    pub days: u32,
    pub totals: UsageTotals,
    pub daily: Vec<UsageDay>,
    /// Largest first (tokens).
    pub by_model: Vec<UsageModel>,
    /// Largest first (tokens).
    pub by_project: Vec<UsageProject>,
}
