//! The unipi suite as a whole (the umbrella `@pi-unipi/unipi` package, audit-unipi §1.2).
//!
//! Oino loads every unipi package as its own component; the umbrella's job (which modules
//! exist, in what order they load, which settings namespace each owns) becomes this static
//! table. The bundle (`oino-bundle`) and the info screen read it; packages use it to name
//! their peers without string literals.
//!
//! Round 5 (D1) removed the presentational packages `footer`, `command-enchantment`,
//! `input-shortcuts` and `app-bridge`; they are listed in [`NOT_PORTED`] with their reasons.

/// unipi release the ports track.
pub const UNIPI_VERSION: &str = "3.0.0";

/// Prefix of every unipi extension id (`unipi.`).
pub const EXTENSION_PREFIX: &str = "unipi.";

/// Port batch (unipi-port §1, §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Batch {
    /// Leaf packages.
    B1,
    /// Process and delegation.
    B2,
    /// Context, memory and long-running work.
    B3,
    /// Presentation.
    B4,
}

/// One ported unipi package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Package {
    /// Extension id (`unipi.<name>`).
    pub id: &'static str,
    /// Directory under `extensions/unipi/`.
    pub dir: &'static str,
    /// unipi npm package name.
    pub npm: &'static str,
    /// Settings namespaces the package registers (unipi names, unchanged).
    pub namespaces: &'static [&'static str],
    /// Port batch.
    pub batch: Batch,
}

macro_rules! packages {
    ($($konst:ident = $id:literal, $dir:literal, $npm:literal, [$($ns:literal),*], $batch:ident;)*) => {
        /// Package constants, in bundled load order.
        pub mod pkg {
            use super::{Batch, Package};
            $(
                #[doc = concat!("`", $id, "`.")]
                pub const $konst: Package = Package {
                    id: $id,
                    dir: $dir,
                    npm: $npm,
                    namespaces: &[$($ns),*],
                    batch: Batch::$batch,
                };
            )*
        }

        /// Every ported package in bundled load order (umbrella order with `unipi.core`
        /// first, unipi-port §1).
        pub const LOAD_ORDER: &[Package] = &[$(pkg::$konst),*];
    };
}

packages! {
    CORE = "unipi.core", "core", "@pi-unipi/core", ["decision-model"], B1;
    WORKFLOW = "unipi.workflow", "workflow", "@pi-unipi/workflow", ["permission"], B3;
    LONG_HORIZON = "unipi.long-horizon", "long-horizon", "@pi-unipi/long-horizon", ["long-horizon"], B3;
    MEMORY = "unipi.memory", "memory", "@pi-unipi/memory", ["memory"], B3;
    DREAM = "unipi.dream", "dream", "@pi-unipi/dream", ["dream"], B3;
    UTILITY = "unipi.utility", "utility", "@pi-unipi/utility", ["utility", "image"], B1;
    SKILL_REGISTRY = "unipi.skill-registry", "skill-registry", "@pi-unipi/skill-registry", ["skills"], B3;
    INFO_SCREEN = "unipi.info-screen", "info-screen", "@pi-unipi/info-screen", ["info-screen"], B4;
    SUBAGENTS = "unipi.subagents", "subagents", "@pi-unipi/subagents", ["subagents"], B2;
    BACKGROUND_TASKS = "unipi.background-tasks", "background-tasks", "@pi-unipi/background-tasks", ["background-tasks"], B2;
    BTW = "unipi.btw", "btw", "@pi-unipi/btw", [], B2;
    WEB_API = "unipi.web-api", "web-api", "@pi-unipi/web-api", ["web-api"], B1;
    ASK_USER = "unipi.ask-user", "ask-user", "@pi-unipi/ask-user", ["ask-user"], B1;
    MCP = "unipi.mcp", "mcp", "@pi-unipi/mcp", ["mcp"], B3;
    NOTIFY = "unipi.notify", "notify", "@pi-unipi/notify", ["notify"], B1;
    KANBOARD = "unipi.kanboard", "kanboard", "@pi-unipi/kanboard", ["kanboard"], B3;
    COMPACTOR = "unipi.compactor", "compactor", "@pi-unipi/compactor", ["compactor"], B3;
    UPDATER = "unipi.updater", "updater", "@pi-unipi/updater", ["updater"], B4;
    FUSION = "unipi.fusion", "fusion", "@pi-unipi/fusion", ["fusion"], B2;
    WATCHDOG = "unipi.watchdog", "watchdog", "@pi-unipi/watchdog", ["watchdog"], B2;
    CRON = "unipi.cron", "cron", "Oino-native (unipi-port §3.24)", ["cron"], B2;
}

/// unipi packages that are deliberately not ported (D22, unipi-port §1; round 5 D1).
pub const NOT_PORTED: &[(&str, &str)] = &[
    ("@pi-unipi/unipi", "replaced by bundling and LOAD_ORDER"),
    (
        "@pi-unipi/kanboard-bin",
        "replaced by the Oino installer shipping `oino-kanboard`",
    ),
    (
        "unipi.footer",
        "D1: TUI status-line chrome; the native glance line and header carry its data",
    ),
    (
        "unipi.command-enchantment",
        "D1: `/`-completion eye-candy; core completions work without it",
    ),
    (
        "unipi.input-shortcuts",
        "D1: TUI editor intents; the desktop binds chords natively",
    ),
    (
        "unipi.app-bridge",
        "D1: presentational control centre; its tabs have native surfaces",
    ),
];

/// Package by extension id.
#[must_use]
pub fn by_id(id: &str) -> Option<&'static Package> {
    LOAD_ORDER.iter().find(|p| p.id == id)
}

/// Package owning a settings namespace.
#[must_use]
pub fn namespace_owner(namespace: &str) -> Option<&'static Package> {
    LOAD_ORDER
        .iter()
        .find(|p| p.namespaces.contains(&namespace))
}

/// Load position of an extension id (`None` for non-unipi ids).
#[must_use]
pub fn load_index(id: &str) -> Option<usize> {
    LOAD_ORDER.iter().position(|p| p.id == id)
}

/// The extension id is a unipi package.
#[must_use]
pub fn is_unipi(id: &str) -> bool {
    by_id(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_order_is_umbrella_order_with_core_first() {
        let names: Vec<&str> = LOAD_ORDER.iter().map(|p| p.dir).collect();
        assert_eq!(
            names,
            [
                "core",
                "workflow",
                "long-horizon",
                "memory",
                "dream",
                "utility",
                "skill-registry",
                "info-screen",
                "subagents",
                "background-tasks",
                "btw",
                "web-api",
                "ask-user",
                "mcp",
                "notify",
                "kanboard",
                "compactor",
                "updater",
                "fusion",
                "watchdog",
                "cron",
            ]
        );
        assert!(
            LOAD_ORDER
                .iter()
                .all(|p| p.id == format!("unipi.{}", p.dir))
        );
    }

    #[test]
    fn namespaces_are_unique_and_cover_unipi_3() {
        let mut all: Vec<&str> = LOAD_ORDER
            .iter()
            .flat_map(|p| p.namespaces)
            .copied()
            .collect();
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "a namespace has two owners");
        // unipi 3.0.0 namespaces minus `cron` (dropped, D22) and the round-5 removals (D1).
        for ns in [
            "ask-user",
            "background-tasks",
            "compactor",
            "decision-model",
            "dream",
            "fusion",
            "image",
            "info-screen",
            "kanboard",
            "long-horizon",
            "mcp",
            "memory",
            "notify",
            "permission",
            "skills",
            "subagents",
            "updater",
            "utility",
            "watchdog",
            "web-api",
        ] {
            assert!(namespace_owner(ns).is_some(), "{ns}");
        }
        for ns in [
            "hints",
            "footer",
            "command-enchantment",
            "input-shortcuts",
            "app-bridge",
        ] {
            assert!(namespace_owner(ns).is_none(), "{ns} is removed (D1)");
        }
        assert_eq!(
            namespace_owner("decision-model").map(|p| p.id),
            Some("unipi.core"),
            "the shared decision-model namespace stays with core"
        );
        assert_eq!(
            namespace_owner("cron").map(|p| p.id),
            Some("unipi.cron"),
            "the Oino-native scheduler owns the cron namespace"
        );
    }

    #[test]
    fn lookups() {
        assert_eq!(load_index("unipi.core"), Some(0));
        assert_eq!(load_index("unipi.watchdog"), Some(19));
        assert!(is_unipi("unipi.btw"));
        assert!(!is_unipi("unipi.app-bridge"));
        assert!(is_unipi("unipi.cron"));
        assert_eq!(load_index("unipi.cron"), Some(20));
        assert_eq!(by_id("unipi.fusion").map(|p| p.batch), Some(Batch::B2));
    }
}
