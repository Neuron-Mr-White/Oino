//! Durability barrier micro-cost (core.md §4 B-1): mean µs of `SessionManager::sync()`
//! over 100 appends on a real session file (the WP-C1 fail-closed barrier — held writer,
//! `sync_all`, one-time parent-dir fsync).

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use oino_ai::{AgentMessage, UserContentField, UserMessage};
use oino_session::{CreateOptions, SessionManager};
use serde_json::json;

/// Micro-cost of one durability barrier over N appends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BarrierCost {
    /// `true` (std fsync is portable; the number is disk-dependent).
    pub(crate) supported: bool,
    /// The measured call, verbatim (kept in the JSON so method changes are explicit).
    pub(crate) impl_label: &'static str,
    /// Number of measured appends.
    pub(crate) appends: usize,
    /// Mean µs per barrier.
    pub(crate) mean_micros: f64,
    /// Fastest barrier, µs.
    pub(crate) min_micros: u64,
    /// Slowest barrier, µs.
    pub(crate) max_micros: u64,
}

impl BarrierCost {
    /// JSON fragment for the report.
    #[must_use]
    pub(crate) fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "supported": self.supported,
            "impl": self.impl_label,
            "appends": self.appends,
            "meanMicros": self.mean_micros,
            "minMicros": self.min_micros,
            "maxMicros": self.max_micros,
        })
    }
}

/// Appends `appends` entries to a fresh session file and times `SessionManager::sync()`
/// after each.
///
/// # Errors
///
/// Session-file or fsync I/O failures.
pub(crate) fn measure(cwd: &Path, session_dir: &Path, appends: usize) -> Result<BarrierCost> {
    const IMPL_LABEL: &str = "SessionManager::sync() after each append (WP-C1 barrier)";
    let mut manager =
        SessionManager::create(cwd, session_dir, CreateOptions::default()).context("session")?;
    // The first message flushes the file (header + entries) — the writer state the barrier
    // runs under.
    manager
        .append_message(AgentMessage::User(UserMessage {
            content: UserContentField::Text("barrier".into()),
            ..UserMessage::default()
        }))
        .context("seed message")?;
    let mut micros = Vec::with_capacity(appends);
    for i in 0..appends {
        manager
            .append_custom_entry("bench.barrier", Some(json!({ "i": i })))
            .context("barrier append")?;
        let start = Instant::now();
        manager
            .sync()
            .with_context(|| format!("SessionManager::sync() (append {i})"))?;
        micros.push(start.elapsed().as_micros());
    }
    let count = micros.len();
    let as_u64 = |m: u128| u64::try_from(m).unwrap_or(u64::MAX);
    let min = micros.iter().min().copied().map_or(0, as_u64);
    let max = micros.iter().max().copied().map_or(0, as_u64);
    let mean = if count == 0 {
        0.0
    } else {
        let sum: f64 = micros.iter().map(|m| as_f64(*m)).sum();
        sum / as_f64(u128::try_from(count).unwrap_or(u128::MAX))
    };
    Ok(BarrierCost {
        supported: true,
        impl_label: IMPL_LABEL,
        appends: count,
        mean_micros: mean,
        min_micros: min,
        max_micros: max,
    })
}

/// `u128` count/value as `f64` (single cast site for pedantic).
fn as_f64(value: u128) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "µs-resolution reporting")]
    {
        value as f64
    }
}
