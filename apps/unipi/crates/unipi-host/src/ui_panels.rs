//! The UniPi reserved-panel table (ui-protocol §8): the oino SDK ships an *empty*
//! reserved registry — every first-party panel is embedder registration
//! (`oino_core::ui::register_reserved`) — and this module registers UniPi's nine panels
//! for every session/registry of the host process. Idempotent: called from every host
//! entry point (`serve`, `run`, the CLI commands), the same spec simply replaces itself.

use oino_core::ui::ReservedPanelSpec;
use oino_ui::slot::Slot;

/// Registers every UniPi reserved panel (call once at host startup; safe to repeat).
pub fn register() {
    for spec in specs() {
        oino_core::ui::register_reserved(spec);
    }
}

/// The reserved-panel table of the UniPi distribution (§6.4 orders included).
#[must_use]
pub fn specs() -> Vec<ReservedPanelSpec> {
    vec![
        ReservedPanelSpec {
            allowed: vec![Slot::Fullscreen, Slot::SidePane],
            orders: vec![(Slot::Fullscreen, 1000), (Slot::SidePane, 960)],
            ..spec(
                "unipi.kanboard/board",
                "unipi.kanboard",
                "board",
                Slot::Fullscreen,
                "Board",
            )
        },
        spec(
            "unipi.cron/jobs",
            "unipi.cron",
            "cron",
            Slot::Fullscreen,
            "Cron",
        ),
        ReservedPanelSpec {
            allowed: vec![Slot::SidePane, Slot::Fullscreen],
            ..spec("host/files", "host", "files", Slot::SidePane, "Files")
        },
        spec("host/git", "host", "git", Slot::SidePane, "Git"),
        spec(
            "unipi.browser/browser",
            "unipi.browser",
            "browser",
            Slot::SidePane,
            "Browser",
        ),
        ReservedPanelSpec {
            allowed: vec![Slot::BottomPane, Slot::SidePane],
            orders: vec![(Slot::BottomPane, 1000), (Slot::SidePane, 970)],
            ..spec(
                "host/terminal",
                "host",
                "terminal",
                Slot::BottomPane,
                "Terminal",
            )
        },
        spec(
            "unipi.info-screen/dashboard",
            "unipi.info-screen",
            "sessionInfo",
            Slot::Overlay,
            "Session",
        ),
        spec(
            "unipi.long-horizon/visualize",
            "unipi.long-horizon",
            "lhStatus",
            Slot::SidePane,
            "Progress",
        ),
        spec(
            "unipi.file-summary/turn-files",
            "unipi.file-summary",
            "turnFiles",
            Slot::TurnFooter,
            "Changed files",
        ),
    ]
}

/// A spec whose single default order covers every allowed slot.
fn spec(id: &str, owner: &str, component: &str, slot: Slot, title: &str) -> ReservedPanelSpec {
    ReservedPanelSpec::new(id, owner, component, slot, title, panel_order(id))
}

/// The §6.4 default orders of the fixed UniPi slots (a single order per panel).
fn panel_order(id: &str) -> i32 {
    match id {
        "host/files" => 1000,
        "host/git" | "unipi.cron/jobs" => 990,
        "unipi.browser/browser" => 980,
        "host/terminal" | "unipi.kanboard/board" => 1000,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_core::ui::reserved;

    #[test]
    fn registers_the_nine_panels() {
        register();
        assert_eq!(specs().len(), 9);
        for spec in specs() {
            let found = reserved(&spec.id).expect("registered");
            assert_eq!(*found, spec);
        }
        // Owner gating data (manifest-side) is synced by the same registration.
        assert_eq!(
            oino_extension_api::manifest::reserved_owner("unipi.kanboard/board").as_deref(),
            Some("unipi.kanboard")
        );
        assert_eq!(
            oino_extension_api::manifest::reserved_owner("host/terminal").as_deref(),
            Some("host")
        );
    }

    #[test]
    fn orders_match_the_fixed_table() {
        register();
        assert_eq!(
            reserved("unipi.kanboard/board")
                .unwrap()
                .order_for(Slot::Fullscreen),
            1000
        );
        assert_eq!(
            reserved("unipi.kanboard/board")
                .unwrap()
                .order_for(Slot::SidePane),
            960
        );
        assert_eq!(
            reserved("host/terminal")
                .unwrap()
                .order_for(Slot::BottomPane),
            1000
        );
        assert_eq!(
            reserved("host/terminal").unwrap().order_for(Slot::SidePane),
            970
        );
        assert_eq!(
            reserved("host/files").unwrap().order_for(Slot::SidePane),
            1000
        );
        assert_eq!(
            reserved("unipi.cron/jobs")
                .unwrap()
                .order_for(Slot::Fullscreen),
            990
        );
        assert_eq!(
            reserved("unipi.browser/browser")
                .unwrap()
                .order_for(Slot::SidePane),
            980
        );
    }
}
