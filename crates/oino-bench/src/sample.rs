//! Process RSS/CPU sampling for B-1 (core.md §4): Linux reads `/proc/self/status`
//! (`VmRSS`, `VmHWM`) and `/proc/self/stat` (utime+stime); other OSes report `unsupported`.

/// One process snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sample {
    /// Resident set size, KiB (`/proc/self/status` `VmRSS`).
    pub(crate) rss_kib: Option<u64>,
    /// Resident set size high-water mark, KiB (`VmHWM`).
    pub(crate) vm_hwm_kib: Option<u64>,
    /// User+system CPU in clock ticks (`/proc/self/stat`, process-wide).
    pub(crate) cpu_ticks: Option<u64>,
}

impl Sample {
    /// Samples the current process; every field is `None` off Linux.
    #[must_use]
    pub(crate) fn take() -> Self {
        let status = std::fs::read_to_string("/proc/self/status").ok();
        let stat = std::fs::read_to_string("/proc/self/stat").ok();
        Self {
            rss_kib: status.as_deref().and_then(|s| status_kib(s, "VmRSS:")),
            vm_hwm_kib: status.as_deref().and_then(|s| status_kib(s, "VmHWM:")),
            cpu_ticks: stat.as_deref().and_then(cpu_ticks),
        }
    }
}

/// Clock-ticks-per-second resolver for `/proc/self/stat` deltas.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    /// `CLK_TCK` (Linux `USER_HZ`; 100 when detection fails).
    pub(crate) ticks_per_sec: u64,
}

impl Clock {
    /// Detects `CLK_TCK` via `sysconf(3)`, falling back to 100 (Linux `USER_HZ`).
    #[must_use]
    pub(crate) fn detect() -> Self {
        let ticks = nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK)
            .ok()
            .flatten()
            .and_then(|v| u64::try_from(v).ok())
            .filter(|v| *v > 0)
            .unwrap_or(100);
        Self {
            ticks_per_sec: ticks,
        }
    }

    /// CPU (user+system) milliseconds between two samples; `None` off Linux.
    #[must_use]
    pub(crate) fn ms_between(self, before: Sample, after: Sample) -> Option<u64> {
        let ticks = after.cpu_ticks?.checked_sub(before.cpu_ticks?)?;
        let ms = (u128::from(ticks) * 1_000) / u128::from(self.ticks_per_sec);
        Some(u64::try_from(ms).unwrap_or(u64::MAX))
    }
}

/// `"<key>  <num> kB"` from `/proc/self/status`, as KiB.
fn status_kib(status: &str, key: &str) -> Option<u64> {
    status
        .lines()
        .find(|line| line.starts_with(key))?
        .strip_prefix(key)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// utime+stime (fields 14–15) of `/proc/self/stat`, in ticks.
fn cpu_ticks(stat: &str) -> Option<u64> {
    // The comm field may contain spaces; everything after its closing `)` starts at
    // field 3 (`state`), so utime/stime are tokens 11/12.
    let (_, rest) = stat.rsplit_once(')')?;
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = tokens.get(11)?.parse().ok()?;
    let stime: u64 = tokens.get(12)?.parse().ok()?;
    Some(utime + stime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_kib() {
        let status = "Name:\toino-bench\nVmRSS:\t     123 kB\nVmHWM:\t     456 kB\nThreads:\t9\n";
        assert_eq!(status_kib(status, "VmRSS:"), Some(123));
        assert_eq!(status_kib(status, "VmHWM:"), Some(456));
        assert_eq!(status_kib(status, "VmSize:"), None);
    }

    #[test]
    fn parses_stat_cpu_ticks() {
        // pid (comm) state ppid … utime stime — utime/stime are fields 14/15.
        let stat = "12345 (oino-benc) R 1 1 0 0 -1 0 0 0 0 0 77 22 0 0 0 0 0 0 0 0\n";
        assert_eq!(cpu_ticks(stat), Some(99));
        assert_eq!(cpu_ticks("12345 (oi no) R 1\n"), None);
    }

    #[test]
    fn cpu_delta_converts_ticks_to_ms() {
        let clock = Clock { ticks_per_sec: 100 };
        let a = Sample {
            cpu_ticks: Some(1_000),
            ..Sample::default()
        };
        let b = Sample {
            cpu_ticks: Some(1_050),
            ..Sample::default()
        };
        assert_eq!(clock.ms_between(a, b), Some(500));
        assert_eq!(clock.ms_between(b, a), None);
    }
}
