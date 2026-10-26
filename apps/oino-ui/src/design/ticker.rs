//! One app-wide ticker (design.md §2.10): drives every spinner at 12 fps and stops when no
//! visible element asked for a frame during the last tick (idle CPU ≈ 0).

use std::time::Duration;

use gpui_kit::{App, Global};

use crate::design::theme::OinoTheme;
use crate::design::tokens::motion::TICK_MS;

/// Ticker state (GPUI global).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ticker {
    frame: u32,
    demand: bool,
    running: bool,
}

impl Global for Ticker {}

impl Ticker {
    /// The current spinner frame; marks demand so the next tick refreshes the windows.
    /// With reduce motion the frame is always 0 and nothing ticks.
    pub fn frame(cx: &mut App) -> u32 {
        if OinoTheme::get(cx).reduce_motion {
            return 0;
        }
        let state = cx.default_global::<Self>();
        state.demand = true;
        let frame = state.frame;
        if !state.running {
            state.running = true;
            Self::start(cx);
        }
        frame
    }

    /// The current frame without asking for more (static renders).
    #[must_use]
    pub fn peek(cx: &App) -> u32 {
        cx.try_global::<Self>().map_or(0, |t| t.frame)
    }

    fn start(cx: &mut App) {
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(TICK_MS))
                    .await;
                let keep = cx.update(|cx| {
                    let state = cx.default_global::<Self>();
                    state.frame = state.frame.wrapping_add(1);
                    let keep = state.demand;
                    state.demand = false;
                    if !keep {
                        state.running = false;
                    }
                    if keep {
                        cx.refresh_windows();
                    }
                    keep
                });
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
}
