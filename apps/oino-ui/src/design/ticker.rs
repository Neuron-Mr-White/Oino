//! One app-wide ticker (design.md §2.10): drives every spinner at 12 fps and stops when no
//! visible element asked for a frame during the last tick (idle CPU ≈ 0).
//!
//! A tick notifies only the views that asked: elements rendered inside a [`Ticker::scoped`]
//! view (session panes, the mascot) register that view; anything else registers the root
//! view ([`Ticker::set_root`]). It never refreshes whole windows, which would re-render every
//! cached session view at once (the §2.11 frame budget: one visible session plus three
//! streaming in the background). Idle states draw frame 0 and ask for nothing, so the ticker
//! task ends one tick after the last animation (idle CPU ≈ 0, GA-46 input).

use std::cell::Cell;
use std::time::Duration;

use gpui_kit::{App, EntityId, Global};

use crate::design::theme::OinoTheme;
use crate::design::tokens::motion::TICK_MS;

thread_local! {
    /// The view whose elements are being built (set by [`Ticker::scoped`]).
    static SCOPE: Cell<Option<EntityId>> = const { Cell::new(None) };
}

/// Ticker state (GPUI global).
#[derive(Clone, Debug, Default)]
pub struct Ticker {
    frame: u32,
    targets: Vec<EntityId>,
    root: Option<EntityId>,
    root_demand: bool,
    running: bool,
    sub: u32,
    /// Slot of each view, assigned round-robin on first demand.
    slots: Vec<(EntityId, u32)>,
    next_slot: u32,
}

/// Sub-ticks per spinner frame: views are spread over them so animating views do not all
/// re-render in the same frame (each still animates at 12 fps).
const SLOTS: u32 = 4;

impl Ticker {
    fn slot(&mut self, id: EntityId) -> u32 {
        if let Some((_, s)) = self.slots.iter().find(|(v, _)| *v == id) {
            return *s;
        }
        let s = self.next_slot;
        self.next_slot = (self.next_slot + 1) % SLOTS;
        if self.slots.len() >= 64 {
            self.slots.remove(0);
        }
        self.slots.push((id, s));
        s
    }
}

impl Global for Ticker {}

impl Ticker {
    /// The current spinner frame; asks for the next tick for the current view.
    /// With reduce motion the frame is always 0 and nothing ticks.
    pub fn frame(cx: &mut App) -> u32 {
        if OinoTheme::get(cx).reduce_motion {
            return 0;
        }
        let scope = SCOPE.with(Cell::get);
        let state = cx.default_global::<Self>();
        match scope {
            Some(id) => {
                if !state.targets.contains(&id) {
                    state.targets.push(id);
                }
            }
            None => state.root_demand = true,
        }
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

    /// The root view notified for spinners outside any scoped view.
    pub fn set_root(cx: &mut App, root: EntityId) {
        cx.default_global::<Self>().root = Some(root);
    }

    /// Runs `f` (a view's element building) with `view` as the ticker target.
    pub fn scoped<R>(view: EntityId, f: impl FnOnce() -> R) -> R {
        let previous = SCOPE.with(|s| s.replace(Some(view)));
        let out = f();
        SCOPE.with(|s| s.set(previous));
        out
    }

    fn start(cx: &mut App) {
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(TICK_MS / u64::from(SLOTS)))
                    .await;
                let keep = cx.update(|cx| {
                    let state = cx.default_global::<Self>();
                    state.sub = (state.sub + 1) % SLOTS;
                    let sub = state.sub;
                    if sub == 0 {
                        state.frame = state.frame.wrapping_add(1);
                    }
                    let mut due = Vec::new();
                    let mut keep_targets = Vec::new();
                    for id in std::mem::take(&mut state.targets) {
                        if state.slot(id) == sub {
                            due.push(id);
                        } else {
                            keep_targets.push(id);
                        }
                    }
                    state.targets = keep_targets;
                    let root_due = sub == 0 && std::mem::take(&mut state.root_demand);
                    let root = state.root;
                    let keep = state.root_demand
                        || root_due
                        || !state.targets.is_empty()
                        || !due.is_empty();
                    state.running = keep;
                    if root_due {
                        match root {
                            Some(root) => cx.notify(root),
                            // No root registered (mobile, tests): redraw everything.
                            None => cx.refresh_windows(),
                        }
                    }
                    for id in due {
                        cx.notify(id);
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
