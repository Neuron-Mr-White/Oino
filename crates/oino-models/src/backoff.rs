//! When a dynamic provider's `list-models` may run again (v3-perf §4.5).
//!
//! Process-wide, keyed by provider id: every services set of a host shares one schedule, so a
//! dead local server (ollama, lmstudio) is probed at most once per backoff step no matter how
//! many projects wake. A failure waits 1 min, then 5 min, then 30 min (and stays at 30); a
//! success refreshes again after [`FRESH_FOR`]. The picker never waits for any of it: it reads
//! the registry (filled from the on-disk [`crate::ModelCache`]).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// Waits after the 1st, 2nd and 3rd+ consecutive failure.
pub const FAILURE_STEPS: [Duration; 3] = [
    Duration::from_mins(1),
    Duration::from_mins(5),
    Duration::from_mins(30),
];

/// How long a successful list (or a fresh cache entry) counts as current.
pub const FRESH_FOR: Duration = Duration::from_hours(1);

#[derive(Clone, Copy, Debug)]
struct Slot {
    failures: u32,
    next: Instant,
    in_flight: bool,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Process-wide refresh schedule of dynamic providers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListSchedule;

impl ListSchedule {
    /// Claims the refresh of `provider` when it is due (`cached_age`: how old the on-disk list
    /// is, `None` = no cache). `false` = skip: refreshed recently, backing off after a failure,
    /// or another services set is listing it right now. A `true` must be followed by
    /// [`Self::finish`].
    #[must_use]
    pub fn claim(provider: &str, cached_age: Option<Duration>, now: Instant) -> bool {
        let mut slots = slots().lock().unwrap_or_else(PoisonError::into_inner);
        let slot = slots.entry(provider.to_owned()).or_insert_with(|| Slot {
            failures: 0,
            // First sight in this process: a fresh cache entry defers the first probe.
            next: match cached_age {
                Some(age) if age < FRESH_FOR => now + FRESH_FOR.saturating_sub(age),
                _ => now,
            },
            in_flight: false,
        });
        if slot.in_flight || now < slot.next {
            return false;
        }
        slot.in_flight = true;
        true
    }

    /// Records the outcome of a claimed refresh and schedules the next one.
    pub fn finish(provider: &str, ok: bool, now: Instant) {
        let mut slots = slots().lock().unwrap_or_else(PoisonError::into_inner);
        let slot = slots.entry(provider.to_owned()).or_insert(Slot {
            failures: 0,
            next: now,
            in_flight: false,
        });
        slot.in_flight = false;
        if ok {
            slot.failures = 0;
            slot.next = now + FRESH_FOR;
        } else {
            let step = (slot.failures as usize).min(FAILURE_STEPS.len() - 1);
            slot.failures = slot.failures.saturating_add(1);
            slot.next = now + FAILURE_STEPS[step];
        }
    }

    /// Forgets `provider`'s schedule (a new key or `/login`: the next claim runs at once).
    pub fn reset(provider: &str) {
        slots()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(provider);
    }

    /// Consecutive failures recorded for `provider` (diagnostics, tests).
    #[must_use]
    pub fn failures(provider: &str) -> u32 {
        slots()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(provider)
            .map_or(0, |slot| slot.failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_1_5_30_minutes_then_stay() {
        let p = "backoff-test-dead";
        ListSchedule::reset(p);
        let t0 = Instant::now();
        assert!(ListSchedule::claim(p, None, t0));
        // In flight: a second services set does not probe concurrently.
        assert!(!ListSchedule::claim(p, None, t0));
        ListSchedule::finish(p, false, t0);
        assert!(!ListSchedule::claim(p, None, t0 + Duration::from_secs(59)));
        let t1 = t0 + Duration::from_secs(60);
        assert!(ListSchedule::claim(p, None, t1));
        ListSchedule::finish(p, false, t1);
        assert!(!ListSchedule::claim(p, None, t1 + Duration::from_secs(299)));
        let t2 = t1 + Duration::from_secs(300);
        assert!(ListSchedule::claim(p, None, t2));
        ListSchedule::finish(p, false, t2);
        assert!(!ListSchedule::claim(
            p,
            None,
            t2 + Duration::from_secs(1799)
        ));
        let t3 = t2 + Duration::from_mins(30);
        assert!(ListSchedule::claim(p, None, t3));
        ListSchedule::finish(p, false, t3);
        assert!(!ListSchedule::claim(
            p,
            None,
            t3 + Duration::from_secs(1799)
        ));
        assert!(ListSchedule::claim(p, None, t3 + Duration::from_mins(30)));
        assert_eq!(ListSchedule::failures(p), 4);
        ListSchedule::finish(p, true, t3);
        assert_eq!(ListSchedule::failures(p), 0);
        ListSchedule::reset(p);
    }

    #[test]
    fn a_fresh_cache_defers_the_first_probe() {
        let p = "backoff-test-fresh";
        ListSchedule::reset(p);
        let now = Instant::now();
        assert!(!ListSchedule::claim(p, Some(Duration::from_secs(10)), now));
        assert!(ListSchedule::claim(p, None, now + FRESH_FOR));
        ListSchedule::reset(p);
        let stale = FRESH_FOR + Duration::from_secs(1);
        assert!(ListSchedule::claim(p, Some(stale), now));
        ListSchedule::reset(p);
    }
}
