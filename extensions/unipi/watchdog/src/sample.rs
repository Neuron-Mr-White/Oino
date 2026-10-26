//! Process-activity sample of a running bash call (unipi `src/proc-sample.ts`).
//!
//! unipi read `/proc` directly (CPU ticks, I/O, wchan, sockets, stdin). Oino guests read the
//! host's `process.process-stats` / `process-tree` (cpu time, read/write bytes, state, start
//! time) for the bash pid from `control.running-bash`; two snapshots 5 s apart give the CPU
//! percentage and I/O deltas. Fields the host does not expose (`comm`, `wchan`, sockets, fd 0)
//! stay empty, so the sleeper veto also uses the command text ([`ProcSample::sleeper_hint`]).

/// One process of the sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcRow {
    /// Pid.
    pub pid: u32,
    /// Parent pid.
    pub ppid: u32,
    /// Command name (empty when the host does not report it).
    pub comm: String,
    /// State letter (`R`, `S`, …).
    pub state: String,
    /// Wait channel (empty when unknown).
    pub wchan: String,
    /// CPU over the window (%).
    pub cpu_pct: f64,
    /// Read + write bytes over the window.
    pub io: u64,
    /// Child count within the sample.
    pub children: usize,
}

/// Totals over the group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupTotals {
    /// CPU (%).
    pub cpu_pct: f64,
    /// Read bytes.
    pub read_bytes: u64,
    /// Write bytes.
    pub write_bytes: u64,
    /// Any process running (`R`).
    pub any_running: bool,
}

impl GroupTotals {
    /// unipi `read_bytes + write_bytes + rchar + wchar` (host exposes read/write only).
    #[must_use]
    pub fn io_total(&self) -> u64 {
        self.read_bytes + self.write_bytes
    }
}

/// unipi `ProcSample`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcSample {
    /// Processes (after the window).
    pub processes: Vec<ProcRow>,
    /// Totals.
    pub group_totals: GroupTotals,
    /// Cumulative I/O of all processes.
    pub cumulative_io: u64,
}

impl ProcSample {
    /// unipi `p.comm === "sleep" || p.wchan.includes("nanosleep")`.
    #[must_use]
    pub fn has_sleeper(&self) -> bool {
        self.processes
            .iter()
            .any(|p| p.comm == "sleep" || p.wchan.contains("nanosleep"))
    }

    /// Marks a sleeper found from the command text (host stats carry no `comm`).
    pub fn sleeper_hint(&mut self, command: &str) {
        let sleeps = command
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|w| w == "sleep");
        if sleeps && !self.has_sleeper() {
            self.processes.push(ProcRow {
                comm: "sleep".into(),
                ..ProcRow::default()
            });
        }
    }
}

/// One `process-stats` reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stat {
    /// Pid.
    pub pid: u32,
    /// Parent.
    pub ppid: u32,
    /// State letter.
    pub state: char,
    /// CPU time (ms).
    pub cpu_ms: u64,
    /// Read bytes.
    pub read_bytes: u64,
    /// Write bytes.
    pub write_bytes: u64,
    /// Start time (ms) — distinguishes a reused pid.
    pub start_time_ms: u64,
}

/// Builds the sample from readings `window_ms` apart.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "rates")]
pub fn from_stats(before: &[Stat], after: &[Stat], window_ms: u64) -> Option<ProcSample> {
    if after.is_empty() {
        return None;
    }
    let seconds = (window_ms.max(1) as f64) / 1000.0;
    let mut totals = GroupTotals::default();
    let mut cumulative = 0;
    let mut processes = Vec::new();
    for a in after {
        let old = before
            .iter()
            .find(|b| b.pid == a.pid && b.start_time_ms == a.start_time_ms);
        let (read, write, cpu) = old.map_or((0, 0, 0.0), |o| {
            (
                a.read_bytes.saturating_sub(o.read_bytes),
                a.write_bytes.saturating_sub(o.write_bytes),
                a.cpu_ms.saturating_sub(o.cpu_ms) as f64 / 1000.0 / seconds * 100.0,
            )
        });
        totals.read_bytes += read;
        totals.write_bytes += write;
        totals.cpu_pct += cpu;
        totals.any_running |= a.state == 'R';
        cumulative += a.read_bytes + a.write_bytes;
        processes.push(ProcRow {
            pid: a.pid,
            ppid: a.ppid,
            comm: String::new(),
            state: a.state.to_string(),
            wchan: String::new(),
            cpu_pct: cpu,
            io: read + write,
            children: after.iter().filter(|p| p.ppid == a.pid).count(),
        });
    }
    Some(ProcSample {
        processes,
        group_totals: totals,
        cumulative_io: cumulative,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(pid: u32, cpu: u64, io: u64) -> Stat {
        Stat {
            pid,
            ppid: 1,
            state: 'S',
            cpu_ms: cpu,
            read_bytes: io,
            write_bytes: 0,
            start_time_ms: 7,
        }
    }

    #[test]
    fn sample_rates_and_sleepers() {
        let s = from_stats(&[st(10, 0, 0)], &[st(10, 2500, 100)], 5000).unwrap_or_default();
        assert!((s.group_totals.cpu_pct - 50.0).abs() < 1e-9);
        assert_eq!(s.group_totals.io_total(), 100);
        assert_eq!(s.cumulative_io, 100);
        let fresh = from_stats(&[], &[st(11, 9000, 9)], 5000).unwrap_or_default();
        assert!(
            fresh.group_totals.cpu_pct.abs() < 1e-9,
            "new pid has no delta"
        );
        assert!(from_stats(&[], &[], 5000).is_none());
        let mut s = ProcSample::default();
        assert!(!s.has_sleeper());
        s.sleeper_hint("for i in 1 2; do sleep 3; done");
        assert!(s.has_sleeper());
        let mut q = ProcSample::default();
        q.sleeper_hint("asleep_tool run");
        assert!(!q.has_sleeper());
    }
}
