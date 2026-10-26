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

/// Ring buffer of frame costs in milliseconds.
#[derive(Clone, Debug)]
pub struct FrameStats {
    samples: [f32; WINDOW],
    len: usize,
    next: usize,
    total: u64,
    dropped: u64,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            samples: [0.0; WINDOW],
            len: 0,
            next: 0,
            total: 0,
            dropped: 0,
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
        /// Draw the overlay.
        pub overlay: bool,
        /// Log a summary line every [`WINDOW`] frames (`OINO_FRAME_LOG=1`).
        pub log: bool,
    }

    impl Global for FrameProbe {}

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
                    if probe.log && probe.stats.log_due() {
                        let s = probe.stats.summary();
                        tracing::info!(
                            target: "oino_frame",
                            p50_ms = s.p50,
                            p99_ms = s.p99,
                            max_ms = s.max,
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
        assert!((sum.max - 30.0).abs() < 0.01);
        assert!(sum.p50 >= 1.0 && sum.p50 <= 2.0);
        let mut out = [0.0; WINDOW];
        assert_eq!(s.ordered(&mut out), WINDOW);
        assert!((out[WINDOW - 1] - 30.0).abs() < 0.01, "newest last");
    }
}
