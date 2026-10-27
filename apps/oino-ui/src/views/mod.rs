//! GPUI views (feature `views`): the desktop shell (design.md §5) over the shared components.
//!
//! | Module | Screen |
//! |---|---|
//! | [`shell`] | A2 desktop shell: sidebar, session list, detail, routes, palette ([`AppShell`]) |
//! | [`session_pane`] | one session: state, pump, commands |
//! | [`settings_page`] | D8 Settings + extension settings hub |
//! | `board`, `extensions_page`, `onboarding` | D9, D12, D13 (`impl AppShell`) |
//! | [`soak`] | soak driver (`OINO_DESKTOP_SOAK`) |

#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    clippy::fn_params_excessive_bools,
    reason = "declarative GPUI element trees (session-bound render helpers stay methods for uniform call sites; callbacks are `Rc` handles cloned into child closures): one long builder chain per screen part reads better than fragments, and layout code uses short names for theme (`t`) and geometry (`w`, `h`, `r`)"
)]

pub mod board;
pub mod cron;
pub mod extensions_page;
pub mod mascot;
pub mod onboarding;
pub mod session_pane;
pub mod settings_page;
pub mod shell;
pub mod soak;

use gpui_kit::{App, KeyBinding};

pub use shell::{AppShell, Workspace};

gpui_kit::actions!(
    oino,
    [
        /// New session in the current project.
        NewSession,
        /// New session in a chosen folder.
        OpenFolder,
        /// The switch sheet (sessions, attention-first).
        SwitchSession,
        /// Sessions screen.
        GoSessions,
        /// Board screen.
        GoBoard,
        /// Extensions screen.
        GoExtensions,
        /// Automations (cron) screen.
        GoCron,
        /// Show/hide the sidebar.
        ToggleSidebar,
        /// Show/hide the side pane.
        ToggleSidePane,
        /// Show/hide the bottom frame.
        ToggleBottom,
        /// Abort the active run.
        Abort,
        /// Command palette.
        CommandPalette,
        /// Model menu.
        PickModel,
        /// Thinking menu.
        PickThinking,
        /// Settings.
        OpenSettings,
        /// Toggle light/dark.
        ToggleTheme,
        /// Session tree in the side pane.
        TreeView,
        /// Find in the conversation.
        FindInConversation,
        /// Close the find bar.
        CloseFind,
        /// Focus the composer.
        FocusComposer,
        /// Expand / collapse the status panel.
        ToggleStatusPanel,
        /// Frame-time overlay.
        ToggleFrameOverlay,
        /// Quit the app.
        Quit,
    ]
);

/// `primary!("k")` → `"cmd-k"` on Apple platforms, `"ctrl-k"` elsewhere.
#[cfg(target_os = "macos")]
macro_rules! primary {
    ($k:literal) => {
        concat!("cmd-", $k)
    };
}
/// `primary!("k")` → `"cmd-k"` on Apple platforms, `"ctrl-k"` elsewhere.
#[cfg(not(target_os = "macos"))]
macro_rules! primary {
    ($k:literal) => {
        concat!("ctrl-", $k)
    };
}

/// The platform's primary modifier in keystroke syntax: `cmd` on Apple platforms, `ctrl`
/// elsewhere (never both).
#[must_use]
pub fn primary_mod() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}

/// Registers key bindings (design.md §4 Keys tables) with the platform's primary modifier.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(primary!("n"), NewSession, None),
        KeyBinding::new(primary!("o"), SwitchSession, None),
        KeyBinding::new(primary!("shift-o"), OpenFolder, None),
        KeyBinding::new(primary!("1"), GoSessions, None),
        KeyBinding::new(primary!("2"), GoBoard, None),
        KeyBinding::new(primary!("3"), GoExtensions, None),
        KeyBinding::new(primary!("4"), GoCron, None),
        KeyBinding::new(primary!("b"), ToggleSidebar, None),
        KeyBinding::new(primary!("j"), ToggleSidePane, None),
        KeyBinding::new(primary!("`"), ToggleBottom, None),
        KeyBinding::new(primary!("."), Abort, None),
        KeyBinding::new(primary!("k"), CommandPalette, None),
        KeyBinding::new(primary!("shift-p"), CommandPalette, None),
        KeyBinding::new(primary!("m"), PickModel, None),
        KeyBinding::new(primary!("shift-m"), PickThinking, None),
        KeyBinding::new(primary!(","), OpenSettings, None),
        KeyBinding::new(primary!("shift-t"), TreeView, None),
        KeyBinding::new(primary!("f"), FindInConversation, None),
        KeyBinding::new(primary!("l"), FocusComposer, None),
        KeyBinding::new(primary!("shift-s"), ToggleStatusPanel, None),
        KeyBinding::new("ctrl-alt-shift-f", ToggleFrameOverlay, None),
        KeyBinding::new(primary!("q"), Quit, None),
    ]);
}
