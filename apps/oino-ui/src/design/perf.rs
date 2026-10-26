//! Frame-time measurement (design.md §2.11): a fixed ring of the last 120 frame costs with
//! p50/p99 and dropped-frame counts, an optional periodic log line, and (feature `views`) the
//! debug overlay toggled with `Ctrl+Alt+Shift+F`.
//!
//! The cost of a frame is measured from the start of the root view's `render` to the end of
//! the paint of its last child (element build + layout + prepaint + paint on the UI thread).
//! Recording is allocation-free.

use std::time::Duration;

/// Frames kept.
pub const WINDOW: usize = 120;
/// Budget of one frame at 120 Hz (ms).
pub const BUDGET_120HZ_MS: f32 = 8.3;
/// Budget of one frame at 60 Hz (ms); slower frames count as dropped.
pub const BUDGET_60HZ_MS: f32 = 16.7;

/// Whole-run histogram resolution: 0.1 ms buckets up to [`HIST_MAX_MS`].
const HIST_BUCKETS: usize = 1000;
/// Frames slower than this land in the last bucket.
const HIST_MAX_MS: f32 = 100.0;

/// Ring buffer of frame costs in milliseconds, plus a whole-run histogram (so the run p99 is
/// exact to 0.1 ms, not a median of window p99s).
#[derive(Clone, Debug)]
pub struct FrameStats {
    samples: [f32; WINDOW],
    len: usize,
    next: usize,
    total: u64,
    dropped: u64,
    over_120hz: u64,
    hist: [u32; HIST_BUCKETS],
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            samples: [0.0; WINDOW],
            len: 0,
            next: 0,
            total: 0,
            dropped: 0,
            over_120hz: 0,
            hist: [0; HIST_BUCKETS],
        }
    }
}

/// Summary of the window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Median (ms).
    pub p50: f32,
    /// 99th percentile (ms).
    pub p99: f32,
    /// Slowest frame (ms).
    pub max: f32,
    /// Frames in the window.
    pub count: usize,
    /// Frames recorded since start.
    pub total: u64,
    /// Frames over the 60 Hz budget since start.
    pub dropped: u64,
}

impl FrameStats {
    /// Records one frame.
    pub fn record(&mut self, cost: Duration) {
        let ms = cost.as_secs_f32() * 1000.0;
        self.samples[self.next] = ms;
        self.next = (self.next + 1) % WINDOW;
        self.len = (self.len + 1).min(WINDOW);
        self.total += 1;
        if ms > BUDGET_60HZ_MS {
            self.dropped += 1;
        }
        if ms > BUDGET_120HZ_MS {
            self.over_120hz += 1;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped bucket index"
        )]
        let bucket = ((ms.clamp(0.0, HIST_MAX_MS) * 10.0) as usize).min(HIST_BUCKETS - 1);
        self.hist[bucket] = self.hist[bucket].saturating_add(1);
    }

    /// Whole-run percentile `q` (0..1) in ms, from the histogram.
    #[must_use]
    pub fn run_percentile(&self, q: f64) -> f32 {
        let total: u64 = self.hist.iter().map(|c| u64::from(*c)).sum();
        if total == 0 {
            return 0.0;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "rank within the frame count"
        )]
        let rank = ((q * total as f64).ceil() as u64).max(1);
        let mut seen = 0u64;
        for (i, c) in self.hist.iter().enumerate() {
            seen += u64::from(*c);
            if seen >= rank {
                #[allow(clippy::cast_precision_loss, reason = "bucket index ≤ 1000")]
                return (i + 1) as f32 / 10.0;
            }
        }
        HIST_MAX_MS
    }

    /// Frames over the 120 Hz budget since start.
    #[must_use]
    pub fn over_120hz(&self) -> u64 {
        self.over_120hz
    }

    /// Samples in recording order (oldest first) into `out`; returns how many.
    pub fn ordered(&self, out: &mut [f32; WINDOW]) -> usize {
        let start = if self.len < WINDOW { 0 } else { self.next };
        for (i, slot) in out.iter_mut().take(self.len).enumerate() {
            *slot = self.samples[(start + i) % WINDOW];
        }
        self.len
    }

    /// Percentiles over the window (stack copy, no allocation).
    #[must_use]
    pub fn summary(&self) -> Summary {
        let mut sorted = self.samples;
        let n = self.len;
        let slice = &mut sorted[..n];
        slice.sort_unstable_by(f32::total_cmp);
        let at = |q: f32| -> f32 {
            if n == 0 {
                return 0.0;
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                reason = "index into at most 120 samples"
            )]
            let i = ((q * (n as f32 - 1.0)).round() as usize).min(n - 1);
            slice[i]
        };
        Summary {
            p50: at(0.5),
            p99: at(0.99),
            max: slice.last().copied().unwrap_or(0.0),
            count: n,
            total: self.total,
            dropped: self.dropped,
        }
    }

    /// Whether a log line is due (every full window).
    #[must_use]
    pub fn log_due(&self) -> bool {
        self.total > 0 && self.total.is_multiple_of(WINDOW as u64)
    }
}

#[cfg(feature = "views")]
pub use overlay::*;

#[cfg(feature = "views")]
mod overlay {
    use std::time::Instant;

    use gpui_kit::{
        AnyElement, App, Global, IntoElement, ParentElement as _, Styled as _, canvas, div, px,
    };

    use super::{BUDGET_120HZ_MS, FrameStats, WINDOW};
    use crate::design::theme::{OinoTheme, TextExt as _};
    use crate::design::type_scale::TextRole;

    /// Frame probe state (GPUI global).
    #[derive(Default)]
    pub struct FrameProbe {
        /// Collected stats.
        pub stats: FrameStats,
        started: Option<Instant>,
        /// Interaction marks: `(label, start)` closed by the next painted frame
        /// (session switch, palette open, keystroke → paint, first frame).
        marks: Vec<(&'static str, Instant)>,
        /// Process start (first-frame latency), set by the binary.
        pub process_start: Option<Instant>,
        first_logged: bool,
        /// Draw the overlay.
        pub overlay: bool,
        /// Log a summary line every [`WINDOW`] frames (`OINO_FRAME_LOG=1`).
        pub log: bool,
    }

    impl Global for FrameProbe {}

    /// Starts an interaction latency measurement closed by the next painted frame (logged as
    /// `latency` with `OINO_FRAME_LOG=1`).
    pub fn mark(cx: &mut App, label: &'static str) {
        let probe = cx.default_global::<FrameProbe>();
        if probe.log && probe.marks.len() < 16 {
            probe.marks.push((label, Instant::now()));
        }
    }

    /// Marks the start of a root render.
    pub fn begin(cx: &mut App) {
        let probe = cx.default_global::<FrameProbe>();
        probe.started = Some(Instant::now());
    }

    /// An invisible element whose paint closes the frame measurement (append last).
    #[must_use]
    pub fn end_marker() -> AnyElement {
        canvas(
            |_, _, _| (),
            |_, (), _, cx| {
                let probe = cx.default_global::<FrameProbe>();
                if let Some(start) = probe.started.take() {
                    probe.stats.record(start.elapsed());
                    if probe.log && !probe.first_logged {
                        probe.first_logged = true;
                        if let Some(p) = probe.process_start {
                            tracing::info!(
                                target: "oino_frame",
                                label = "first frame",
                                ms = p.elapsed().as_secs_f32() * 1000.0,
                                "latency"
                            );
                        }
                    }
                    for (label, at) in probe.marks.drain(..) {
                        tracing::info!(
                            target: "oino_frame",
                            label,
                            ms = at.elapsed().as_secs_f32() * 1000.0,
                            "latency"
                        );
                    }
                    if probe.log && probe.stats.log_due() {
                        let s = probe.stats.summary();
                        tracing::info!(
                            target: "oino_frame",
                            p50_ms = s.p50,
                            p99_ms = s.p99,
                            max_ms = s.max,
                            run_p99_ms = probe.stats.run_percentile(0.99),
                            over_8ms = probe.stats.over_120hz(),
                            frames = s.total,
                            dropped = s.dropped,
                            "frame times"
                        );
                    }
                }
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    /// Toggles the overlay.
    pub fn toggle_overlay(cx: &mut App) {
        let probe = cx.default_global::<FrameProbe>();
        probe.overlay = !probe.overlay;
    }

    #[allow(clippy::many_single_char_names, reason = "chart geometry")]
    /// The overlay (last 120 frames as bars, p50/p99, dropped), when enabled.
    #[must_use]
    pub fn overlay(cx: &App) -> Option<AnyElement> {
        let probe = cx.try_global::<FrameProbe>()?;
        if !probe.overlay {
            return None;
        }
        let t = OinoTheme::get(cx);
        let mut samples = [0.0f32; WINDOW];
        let n = probe.stats.ordered(&mut samples);
        let s = probe.stats.summary();
        let bars = samples[..n].iter().map(|ms| {
            let h = (ms / 33.0 * 48.0).clamp(1.0, 48.0);
            let c = if *ms > super::BUDGET_60HZ_MS {
                t.danger()
            } else if *ms > BUDGET_120HZ_MS {
                t.warning()
            } else {
                t.success()
            };
            div().w(px(2.)).h(px(h)).bg(c)
        });
        Some(
            div()
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .p(px(8.))
                .rounded(px(8.))
                .bg(t.popover())
                .border_1()
                .border_color(t.border())
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .mono_text(&t, TextRole::Xs)
                        .text_color(t.fg())
                        .child(format!(
                            "p50 {:.1}ms · p99 {:.1}ms · max {:.1}ms · dropped {}/{}",
                            s.p50, s.p99, s.max, s.dropped, s.total
                        )),
                )
                .child(
                    div()
                        .h(px(48.))
                        .flex()
                        .items_end()
                        .gap(px(1.))
                        .children(bars),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_and_percentiles() {
        let mut s = FrameStats::default();
        assert_eq!(s.summary().count, 0);
        for i in 0..200u64 {
            s.record(Duration::from_micros(1000 + (i % 10) * 100));
        }
        s.record(Duration::from_millis(30));
        let sum = s.summary();
        assert_eq!(sum.count, WINDOW);
        assert_eq!(sum.total, 201);
        assert_eq!(sum.dropped, 1);
        let run = s.run_percentile(0.99);
        assert!((1.8..=2.1).contains(&run), "run p99 {run}");
        assert!((sum.max - 30.0).abs() < 0.01);
        assert!(sum.p50 >= 1.0 && sum.p50 <= 2.0);
        let mut out = [0.0; WINDOW];
        assert_eq!(s.ordered(&mut out), WINDOW);
        assert!((out[WINDOW - 1] - 30.0).abs() < 0.01, "newest last");
    }
}
