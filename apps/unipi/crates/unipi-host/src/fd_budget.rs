//! The host's file-descriptor budget (v2 QA "accept failed: No file descriptors
//! available", os error 24).
//!
//! Every cached services set in the session hub holds one wasm instance per
//! extension, and every instance holds real descriptors: two log files
//! (stdout/stderr append), its preopen directories (`ext-global`,
//! `ext-workspace`, `ext-session`, `tmp`, `cache`, the project cwd, agent
//! subdirs…). With a realistic install (31 extensions) one set costs ~290 fds;
//! the hub's default `maxServices` of 4 therefore reserves ~1200 descriptors —
//! more than the 1024 `RLIMIT_NOFILE` a desktop session gives GUI-launched
//! processes, so four concurrent project sessions exhaust the table and even
//! `app.sock`'s `accept` fails.
//!
//! [`raise_soft_to_hard`] lifts the soft limit where the hard limit allows it
//! (systemd user slices often allow 524288), and [`services_cap`] turns
//! whatever limit is left into a hub `maxServices` that fits the budget.

use oino_client::hub::DEFAULT_MAX_SERVICES;

/// Estimated descriptors held by one cached services set: measured at ~290 with
/// a 31-extension install (2 log fds + ~7 preopen dirs per wasm instance).
const FD_PER_SERVICES_SET: u64 = 300;

/// Descriptors kept out of the budget for everything else the host does while
/// sessions are open: app.sock connections, git watchers, session files and
/// leases, the endpoint, transient fs-op sockets.
const FD_MARGIN: u64 = 250;

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

/// How many concurrent services sets fit in `soft_limit` descriptors, clamped
/// to at least one and at most the hub's default (`maxServices`, 4).
///
/// A desktop session's 1024 (soft == hard, unraisable) gives 2; a systemd user
/// slice's 524288 keeps the default 4.
#[must_use]
pub fn services_cap(soft_limit: u64) -> usize {
    if soft_limit == u64::MAX {
        return DEFAULT_MAX_SERVICES;
    }
    let fits = soft_limit
        .saturating_sub(FD_MARGIN)
        .saturating_div(FD_PER_SERVICES_SET);
    usize::try_from(fits)
        .unwrap_or(DEFAULT_MAX_SERVICES)
        .clamp(1, DEFAULT_MAX_SERVICES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_fits_the_descriptor_budget() {
        // A desktop 1024 limit (soft == hard) leaves room for 2 sets, not the default 4.
        assert_eq!(services_cap(1024), 2);
        // Generous user slices keep the setting's default.
        assert_eq!(services_cap(65_536), DEFAULT_MAX_SERVICES);
        assert_eq!(services_cap(u64::MAX), DEFAULT_MAX_SERVICES);
        // Degenerate limits still allow one project.
        assert_eq!(services_cap(400), 1);
        assert_eq!(services_cap(0), 1);
    }

    #[test]
    fn soft_limit_is_reportable() {
        // Must not panic; on Linux it is at least 1024.
        let soft = soft_limit();
        assert!(soft >= 1024 || soft == u64::MAX);
    }
}
