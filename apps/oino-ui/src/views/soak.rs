//! Soak driver (`OINO_DESKTOP_SOAK=<seconds>`, docs/dev/v1/stability.md): cycles through the
//! app's heavy paths — new sessions, long streamed replies, side tabs (files, tree, review,
//! browser), terminals, theme switches, palette, settings, board, split panes, reopening
//! listed sessions — and logs a `soak` line (target `oino_soak`) every 10 s with frame-time
//! stats, then quits. `apps/oino-desktop/scripts/soak.sh` samples RSS / threads / fds from
//! outside and judges the run.
//!
//! - `OINO_SOAK_STEP_MS` (default 700): pause between actions.
//! - `OINO_SOAK_PROMPT` (default `stream a long answer|fix the parser`): prompts sent to the
//!   active pane in turn (`|`-separated; the demo backend streams a long reply for `stream…`).
//! - `OINO_SOAK_BROWSER=1`: also open the embedded browser tab.
//! - `OINO_SOAK_TERMINAL=0`: skip terminals.

use std::time::{Duration, Instant};

use gpui_kit::{Context, Window};

use crate::design::perf::FrameProbe;
use crate::design::theme::OinoTheme;
use crate::host::ThemePref;
use crate::views::session_pane::{SessionPane, SideTab};
use crate::views::shell::{Route, Workspace};

/// Soak parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoakConfig {
    /// Run length.
    pub duration: Duration,
    /// Pause between actions.
    pub step: Duration,
    /// Prompt sent to the active pane.
    pub prompt: String,
    /// Exercise the embedded browser.
    pub browser: bool,
    /// Exercise terminals.
    pub terminal: bool,
}

/// Interval of `soak` log lines.
const REPORT_EVERY: Duration = Duration::from_secs(10);

/// A soak requested on the command line (`--soak <seconds>`), read by [`SoakConfig::get`].
#[derive(Clone, Copy, Debug)]
pub struct Requested(pub u64);

impl gpui_kit::Global for Requested {}

impl SoakConfig {
    /// From [`Requested`] or `OINO_DESKTOP_SOAK`, plus the `OINO_SOAK_*` knobs; `None` when not
    /// soaking.
    #[must_use]
    pub fn get(cx: &gpui_kit::App) -> Option<Self> {
        let secs = cx.try_global::<Requested>().map(|r| r.0).or_else(|| {
            std::env::var("OINO_DESKTOP_SOAK")
                .ok()
                .and_then(|v| v.parse().ok())
        })?;
        let step = std::env::var("OINO_SOAK_STEP_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(700);
        Some(Self {
            duration: Duration::from_secs(secs),
            step: Duration::from_millis(step),
            prompt: std::env::var("OINO_SOAK_PROMPT")
                .unwrap_or_else(|_| "stream a long answer|fix the parser".to_owned()),
            browser: std::env::var("OINO_SOAK_BROWSER").is_ok_and(|v| v == "1"),
            terminal: !std::env::var("OINO_SOAK_TERMINAL").is_ok_and(|v| v == "0"),
        })
    }
}

/// One soak action (the cycle order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// New session (panes beyond the live cap are released).
    NewSession,
    /// Prompt the active pane (a long streamed reply).
    Prompt,
    /// Type into the composer (editor events, completions).
    Type,
    /// Side pane: Files.
    Files,
    /// Side pane: Tree.
    Tree,
    /// Toggle light / dark.
    Theme,
    /// Open the command palette.
    Palette,
    /// Close it.
    ClosePalette,
    /// Bottom frame with a terminal.
    Terminal,
    /// Settings page.
    Settings,
    /// Back from settings.
    Back,
    /// Side pane: Review, or the browser.
    Review,
    /// Split (or close the last split pane at four).
    Split,
    /// Close the bottom frame and its terminals.
    CloseBottom,
    /// Reopen a listed session.
    Reopen,
    /// Board, then back.
    Board,
    /// Abort, close the side pane, refresh the list.
    Settle,
}

/// The cycle.
pub const CYCLE: [Action; 17] = [
    Action::NewSession,
    Action::Prompt,
    Action::Type,
    Action::Files,
    Action::Tree,
    Action::Theme,
    Action::Palette,
    Action::ClosePalette,
    Action::Terminal,
    Action::Settings,
    Action::Back,
    Action::Review,
    Action::Split,
    Action::CloseBottom,
    Action::Reopen,
    Action::Board,
    Action::Settle,
];

/// Action of step `n`.
#[must_use]
pub fn action(n: u64) -> Action {
    let len = CYCLE.len() as u64;
    CYCLE[usize::try_from(n % len).unwrap_or(0)]
}

/// Starts the soak loop on the window of `ws`.
pub fn start(config: SoakConfig, window: &mut Window, cx: &mut Context<Workspace>) {
    tracing::info!(target: "oino_soak", ?config, "soak started");
    cx.spawn_in(window, async move |this, cx| {
        let started = Instant::now();
        let mut last_report = started;
        let mut n = 0u64;
        loop {
            cx.background_executor().timer(config.step).await;
            let done = started.elapsed() >= config.duration;
            let report = done || last_report.elapsed() >= REPORT_EVERY;
            let alive = this.update_in(cx, |ws, window, cx| {
                if !done {
                    ws.soak_step(action(n), n, &config, window, cx);
                }
                if report {
                    ws.soak_report(started.elapsed(), n, cx);
                }
                if done {
                    tracing::info!(target: "oino_soak", steps = n, "soak done");
                    cx.quit();
                }
            });
            if alive.is_err() || done {
                break;
            }
            if action(n) == Action::Type {
                // Outside the workspace update: dispatching a key draws the window, which
                // renders the workspace (a double lease inside `update_in`).
                let typed = cx.update(|window, cx| {
                    for key in ["s", "o", "a", "k", "space", "/", "backspace"] {
                        if let Ok(k) = gpui_kit::Keystroke::parse(key) {
                            window.dispatch_keystroke(k, cx);
                        }
                    }
                });
                if typed.is_err() {
                    break;
                }
            }
            if report {
                last_report = Instant::now();
            }
            n += 1;
        }
    })
    .detach();
}

impl Workspace {
    fn soak_report(&self, elapsed: Duration, steps: u64, cx: &mut Context<Self>) {
        let busy = self
            .panes
            .iter()
            .filter(|p| p.entity.read(cx).model.is_busy())
            .count();
        let probe = cx.default_global::<FrameProbe>();
        let s = probe.stats.summary();
        tracing::info!(
            target: "oino_soak",
            elapsed_s = elapsed.as_secs(),
            steps,
            panes = self.panes.len(),
            busy,
            frames = s.total,
            window_max_ms = s.max,
            run_max_ms = probe.stats.run_max(),
            run_p99_ms = probe.stats.run_percentile(0.99),
            dropped = s.dropped,
            "soak"
        );
    }

    fn soak_step(
        &mut self,
        action: Action,
        n: u64,
        config: &SoakConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        tracing::debug!(target: "oino_soak", ?action, n, "step");
        match action {
            Action::NewSession => self.new_session(window, cx),
            Action::Prompt => {
                // `|`-separated prompts take turns.
                let prompts: Vec<&str> = config.prompt.split('|').collect();
                let pick = usize::try_from(n / CYCLE.len() as u64).unwrap_or(0) % prompts.len();
                let prompt = prompts[pick].trim().to_owned();
                self.with_pane(cx, move |p, cx| {
                    if !p.model.is_busy() {
                        p.run_prompt(&prompt, cx);
                    }
                });
            }
            Action::Type => {
                // Focus here; the keys follow outside this update (`start`).
                if let Some(p) = self.active_pane().cloned() {
                    p.update(cx, |p, cx| p.set_composer_text("", cx));
                    SessionPane::focus(&p, window, cx);
                }
            }
            Action::Files => self.open_surface("side:files", window, cx),
            Action::Tree => self.open_surface("side:tree", window, cx),
            Action::Theme => {
                let next = if OinoTheme::get(cx).dark {
                    ThemePref::Light
                } else {
                    ThemePref::Dark
                };
                self.update_settings(|s| s.theme = next, window, cx);
            }
            Action::Palette => self.open_palette("", window, cx),
            Action::ClosePalette => {
                self.palette = None;
                cx.notify();
            }
            Action::Terminal => {
                if config.terminal {
                    // An empty bottom frame opens on a new shell (`render_bottom`).
                    self.bottom_open = true;
                    self.with_pane(cx, |p, cx| {
                        p.bottom.0.clear();
                        cx.notify();
                    });
                    cx.notify();
                }
            }
            Action::Settings => self.open_settings(None, window, cx),
            Action::Back => {
                self.settings_view = None;
                self.route = Route::Sessions;
                cx.notify();
            }
            Action::Review => {
                if config.browser {
                    self.open_surface("browser:about:blank", window, cx);
                } else {
                    self.with_pane(cx, |p, cx| p.open_side(SideTab::Review, cx));
                }
            }
            Action::Split => {
                if self.layout.len() >= 4 {
                    if let Some(last) = self.layout.last().copied() {
                        self.close_pane(last, window, cx);
                    }
                } else {
                    self.split(window, cx);
                }
            }
            Action::CloseBottom => {
                self.bottom_open = false;
                self.with_pane(cx, |p, cx| {
                    #[cfg(feature = "terminal")]
                    {
                        let ids: Vec<u64> = p.terminals.iter().map(|t| t.id).collect();
                        for id in ids {
                            p.close_terminal(id, cx);
                        }
                    }
                    p.bottom.0.clear();
                    cx.notify();
                });
                cx.notify();
            }
            Action::Reopen => {
                let ids: Vec<String> = self
                    .sessions
                    .groups()
                    .into_iter()
                    .flat_map(|g| g.sessions)
                    .filter(|e| e.summary.path.is_some())
                    .map(|e| e.summary.id)
                    .collect();
                if !ids.is_empty() {
                    let pick = usize::try_from(n).unwrap_or(0) % ids.len();
                    self.open_session(&ids[pick], window, cx);
                }
            }
            Action::Board => {
                if self.route == Route::Board {
                    self.route = Route::Sessions;
                } else {
                    self.open_board(window, cx);
                }
                cx.notify();
            }
            Action::Settle => {
                if self.route == Route::Board {
                    self.route = Route::Sessions;
                }
                self.side_open = false;
                self.with_pane(cx, |p, cx| {
                    if p.model.is_busy() && n.is_multiple_of(3) {
                        p.abort(cx);
                    }
                    p.side.close();
                    cx.notify();
                });
                self.refresh_sessions(cx);
                cx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_covers_every_action_once() {
        assert_eq!(action(0), Action::NewSession);
        assert_eq!(action(CYCLE.len() as u64), Action::NewSession);
        for (i, a) in CYCLE.iter().enumerate() {
            assert_eq!(CYCLE.iter().filter(|b| *b == a).count(), 1, "{a:?} at {i}");
        }
    }
}
