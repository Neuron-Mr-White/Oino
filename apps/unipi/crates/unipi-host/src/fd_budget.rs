//! The host's resource budget for its cached services sets (v2 QA "accept failed:
//! No file descriptors available", os error 24; and RAM).
//!
//! Every cached services set in the session hub holds one wasm instance per
//! extension, and every instance holds real descriptors: two log files
//! (stdout/stderr append), its preopen directories (`ext-global`,
//! `ext-workspace`, `ext-session`, `tmp`, `cache`, the project cwd, agent
//! subdirs…). With a realistic install (31 extensions) one set costs ~290 fds
//! *and* the linear memory of those instances (which never shrinks) plus the
//! session runtime, measured at 500–800 MiB on the owner's install.
//!
//! The cap is therefore **whatever the machine can take**, not a fixed 4: more
//! open projects means more chats that stay live instead of being retired
//! behind the app. [`raise_soft_to_hard`] lifts the soft descriptor limit where
//! the hard limit allows it (systemd user slices often allow 524288),
//! [`services_cap`] turns the descriptors left and the RAM available right now
//! into a hub `maxServices`, and [`MAX_SERVICES`] is the guard against a
//! runaway (an embedding host that never evicts).

/// Ceiling on the services cache whatever the budget allows: dozens of projects
/// open at once with a running turn in each is a defect, not a workload, and the
/// per-set cost is paid in RAM whether or not anyone looks at those chats.
///
/// A machine that binds on descriptors or memory first gets the smaller number
/// (see [`services_cap`]); this only bounds a host with unlimited resources.
pub const MAX_SERVICES: usize = 32;

/// Estimated descriptors held by one cached services set: measured at ~290 with
/// a 31-extension install (2 log fds + ~7 preopen dirs per wasm instance).
const FD_PER_SERVICES_SET: u64 = 300;

/// Descriptors kept out of the budget for everything else the host does while
/// sessions are open: app.sock connections, git watchers, session files and
/// leases, the endpoint, transient fs-op sockets.
const FD_MARGIN: u64 = 250;

/// Estimated resident memory of one cached services set (extensions' wasm
/// instances and their linear memories, the session runtime, parsed history).
/// Measured at 500–800 MiB with the owner's 31-extension install; rounded up on
/// purpose.
const MEM_PER_SERVICES_SET: u64 = 768 << 20;

/// The cache may reserve half of what the OS says is available: the other half
/// belongs to the app, the browser, the tools the chats run and the OS.
const MEM_DIVISOR: u64 = 2;

/// Lifts `RLIMIT_NOFILE`'s soft limit to the hard limit, when that is allowed,
/// and returns the effective soft limit either way. Unsupported platforms
/// (Windows) report `u64::MAX` — no budget is needed there.
#[must_use]
pub fn soft_limit() -> u64 {
    #[cfg(unix)]
    {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
            return u64::MAX;
        }
        if limit.rlim_cur < limit.rlim_max {
            limit.rlim_cur = limit.rlim_max;
            // A concurrent raise of the hard limit can race us back down; the
            // result is only a lower soft limit, which the cap accounts for.
            let _ = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
        }
        soft_limit_now().min(limit.rlim_max)
    }
    #[cfg(not(unix))]
    {
        u64::MAX
    }
}

#[cfg(unix)]
fn soft_limit_now() -> u64 {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return u64::MAX;
    }
    limit.rlim_cur
}

/// How many concurrent services sets the machine can take: the descriptors left
/// over from `soft_limit` and the RAM available right now, whichever runs out
/// first, never more than [`MAX_SERVICES`] and never fewer than one.
///
/// A desktop session's 1024 (soft == hard, unraisable) gives 2. A systemd user
/// slice's 524288 descriptors alone would allow ~1700; memory decides there
/// (`available`, half of it, `MEM_PER_SERVICES_SET` each).
#[must_use]
pub fn services_cap(soft_limit: u64) -> usize {
    services_cap_for(soft_limit, available_memory())
}

/// [`services_cap`] with the available memory passed in (pure, so the arithmetic
/// is testable without the machine's own RAM).
#[must_use]
pub fn services_cap_for(soft_limit: u64, available: Option<u64>) -> usize {
    let by_fds = if soft_limit == u64::MAX {
        MAX_SERVICES
    } else {
        usize::try_from(
            soft_limit
                .saturating_sub(FD_MARGIN)
                .saturating_div(FD_PER_SERVICES_SET),
        )
        .unwrap_or(MAX_SERVICES)
    };
    let by_memory = available.map_or(MAX_SERVICES, services_cap_for_memory);
    by_fds.min(by_memory).clamp(1, MAX_SERVICES)
}

/// Sets that fit in `available` bytes of RAM (see [`MEM_PER_SERVICES_SET`]).
#[must_use]
pub fn services_cap_for_memory(available: u64) -> usize {
    let fits = available
        .saturating_div(MEM_DIVISOR)
        .saturating_div(MEM_PER_SERVICES_SET);
    usize::try_from(fits).unwrap_or(MAX_SERVICES)
}

/// Bytes the OS says it can hand out right now (`MemAvailable` on Linux), or
/// `None` where there is no cheap, honest answer (macOS, Windows): the cap then
/// rests on descriptors and [`MAX_SERVICES`] alone.
#[must_use]
pub fn available_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb = text.lines().find_map(|line| {
            let rest = line.strip_prefix("MemAvailable:")?;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })?;
        Some(kb.saturating_mul(1024))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn cap_fits_the_descriptor_and_memory_budget() {
        // A desktop 1024 limit (soft == hard) leaves room for 2 sets.
        assert_eq!(services_cap_for(1024, None), 2);
        // A user slice's descriptors alone allow far more than the runaway guard.
        assert_eq!(services_cap_for(65_536, None), MAX_SERVICES);
        assert_eq!(services_cap_for(u64::MAX, None), MAX_SERVICES);
        // Degenerate limits still allow one project.
        assert_eq!(services_cap_for(400, None), 1);
        assert_eq!(services_cap_for(0, None), 1);
    }

    #[test]
    fn memory_binds_before_a_generous_descriptor_budget() {
        // 34 GiB free: half of it, 768 MiB a set — 22 live projects on the owner's PC.
        assert_eq!(services_cap_for(524_288, Some(34 * GIB)), 22);
        // A small laptop keeps it to a couple of projects instead of trusting 32.
        assert_eq!(services_cap_for(524_288, Some(3 * GIB)), 2);
        // Less than one set's worth still opens one project (and the cache then
        // evicts by idle, never by refusing the open of an idle session).
        assert_eq!(services_cap_for(524_288, Some(512 << 20)), 1);
        // A tiny descriptor limit still wins over plenty of RAM.
        assert_eq!(services_cap_for(1_024, Some(64 * GIB)), 2);
    }

    #[test]
    fn soft_limit_and_available_memory_are_reportable() {
        // Must not panic; on Linux the soft limit is at least 1024.
        let soft = soft_limit();
        assert!(soft >= 1024 || soft == u64::MAX);
        // Present on Linux, absent elsewhere — either way the cap stays in its bounds.
        if let Some(bytes) = available_memory() {
            assert!(bytes > 0, "MemAvailable parsed: {bytes}");
        }
        let cap = services_cap(soft);
        assert!((1..=MAX_SERVICES).contains(&cap), "cap {cap}");
    }
}
