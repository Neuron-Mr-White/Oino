//! Typing-burst tracker for the undo history (unipi `src/burst.ts`).
//!
//! unipi watched raw terminal input; Oino frontends report debounced `changed` editor intents
//! (ui-protocol §7.2), which this tracker treats as "input arrived". A burst records the
//! editor text twice — at open (the pre-burst text) and at close (the post-burst text). A burst
//! closes after [`BURST_IDLE_MS`] without input or is split after [`BURST_MAX_MS`]. The tracker
//! is pure: the caller records texts and arms the `oino_timer` it asks for.

/// Idle time that ends a burst.
pub const BURST_IDLE_MS: i64 = 600;
/// Longest burst before it is split.
pub const BURST_MAX_MS: i64 = 2_000;

/// What a timer firing means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerOutcome {
    /// Still typing: arm again after this many ms.
    Rearm(i64),
    /// Close the burst (record the post-burst text).
    Close,
    /// No burst open.
    Idle,
}

/// Burst state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Burst {
    open: bool,
    start: i64,
    last_input: i64,
}

impl Burst {
    /// Input arrived at `now`. Returns `Some(delay)` when a burst opened: the caller records the
    /// pre-burst text and arms a timer for `delay` ms.
    pub fn on_input(&mut self, now: i64) -> Option<i64> {
        if self.open {
            self.last_input = now;
            return None;
        }
        *self = Self {
            open: true,
            start: now,
            last_input: now,
        };
        Some(self.wait(now))
    }

    fn wait(&self, now: i64) -> i64 {
        let idle = BURST_IDLE_MS - (now - self.last_input);
        let max = BURST_MAX_MS - (now - self.start);
        idle.min(max).max(0)
    }

    /// The burst timer fired at `now`.
    pub fn on_timer(&mut self, now: i64) -> TimerOutcome {
        if !self.open {
            return TimerOutcome::Idle;
        }
        if now - self.last_input < BURST_IDLE_MS && now - self.start < BURST_MAX_MS {
            return TimerOutcome::Rearm(self.wait(now));
        }
        self.open = false;
        TimerOutcome::Close
    }

    /// Closes an open burst; `true` when one was open (the caller records the current text and
    /// cancels the timer).
    pub fn close(&mut self) -> bool {
        std::mem::take(&mut self.open)
    }

    /// Forgets the burst without recording.
    pub fn cancel(&mut self) {
        self.open = false;
    }

    /// A burst is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::EditHistory;

    /// Drives a burst with a fake clock and one timer, like unipi's `FakeClock`.
    struct Rig {
        now: i64,
        timer: Option<i64>,
        burst: Burst,
        history: EditHistory,
        texts: Vec<String>,
        reads: usize,
    }

    impl Rig {
        fn new(texts: &[&str]) -> Self {
            Self {
                now: 0,
                timer: None,
                burst: Burst::default(),
                history: EditHistory::default(),
                texts: texts.iter().rev().map(|s| (*s).to_owned()).collect(),
                reads: 0,
            }
        }
        fn read(&mut self) -> String {
            self.reads += 1;
            self.texts.pop().unwrap_or_default()
        }
        fn input(&mut self) {
            if let Some(delay) = self.burst.on_input(self.now) {
                let t = self.read();
                self.history.record(&t);
                self.timer = Some(self.now + delay);
            }
        }
        fn close(&mut self) {
            self.timer = None;
            if self.burst.close() {
                let t = self.read();
                self.history.record(&t);
            }
        }
        fn advance(&mut self, ms: i64) {
            self.now += ms;
            while let Some(at) = self.timer.filter(|at| *at <= self.now) {
                self.timer = None;
                match self.burst.on_timer(at.max(self.now)) {
                    TimerOutcome::Rearm(d) => self.timer = Some(self.now + d),
                    TimerOutcome::Close => {
                        let t = self.read();
                        self.history.record(&t);
                    }
                    TimerOutcome::Idle => {}
                }
            }
        }
    }

    #[test]
    fn hundred_keys_cost_two_reads() {
        let mut r = Rig::new(&["", &"x".repeat(100)]);
        r.input();
        for _ in 1..100 {
            r.advance(10);
            r.input();
        }
        assert_eq!(r.reads, 1);
        r.advance(BURST_IDLE_MS);
        assert_eq!(r.reads, 2);
    }

    #[test]
    fn long_typing_splits_bursts() {
        let mut r = Rig::new(&["", "a", "ab", "abc", "abcd", "abcde", "abcdef"]);
        r.input();
        for _ in 0..50 {
            r.advance(100);
            r.input();
        }
        r.advance(BURST_IDLE_MS);
        assert!((5..=8).contains(&r.reads), "{} reads", r.reads);
    }

    #[test]
    fn undo_during_burst_keeps_redo() {
        let mut r = Rig::new(&["", "typed text"]);
        r.input();
        r.advance(100);
        r.input();
        r.close();
        assert_eq!(r.history.undo("typed text").text, "");
        assert_eq!(r.history.redo("").text, "typed text");
        r.input();
        r.advance(BURST_IDLE_MS);
        assert!(r.history.undo("").ok);
    }

    #[test]
    fn close_without_burst_reads_nothing() {
        let mut r = Rig::new(&["x"]);
        r.close();
        r.close();
        assert_eq!(r.reads, 0);
    }

    #[test]
    fn cancel_drops_without_recording() {
        let mut r = Rig::new(&["", "mid-typing"]);
        r.input();
        r.burst.cancel();
        assert_eq!(r.reads, 1);
        let s = r.history.undo("mid-typing");
        assert!(s.ok && s.text.is_empty());
        assert!(!r.burst.is_open());
    }
}
