//! The `Icon` set (§4.5.2): TUI renders Unicode / Nerd Font glyphs, the app Lucide.

use serde::{Deserialize, Serialize};

/// Icon names (§4.5.2). One per line in Appendix A order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    /// `✓` done / ok.
    #[default]
    Check,
    /// `✗` fail / close.
    X,
    /// `•` neutral dot.
    Dot,
    /// `⚠` warning.
    Warn,
    /// `ℹ` info.
    Info,
    /// `⏱` time.
    Clock,
    /// `⚙` settings.
    Gear,
    /// `⑂` git branch.
    Branch,
    /// `⚡` energy / speed.
    Bolt,
    /// Agent.
    Bot,
    /// Directory.
    Folder,
    /// File.
    File,
    /// Search.
    Search,
    /// Network.
    Cloud,
    /// Lock / secret.
    Lock,
    /// Kanban board.
    Board,
    /// Terminal.
    Terminal,
    /// Web.
    Globe,
    /// Diff.
    Diff,
    /// List.
    List,
    /// Chart.
    Chart,
    /// Play.
    Play,
    /// Pause.
    Pause,
    /// Stop.
    Stop,
    /// Refresh.
    Refresh,
    /// Plus.
    Plus,
    /// Trash.
    Trash,
}

/// All icons, in Appendix A order.
pub const ALL_ICONS: [Icon; 27] = [
    Icon::Check,
    Icon::X,
    Icon::Dot,
    Icon::Warn,
    Icon::Info,
    Icon::Clock,
    Icon::Gear,
    Icon::Branch,
    Icon::Bolt,
    Icon::Bot,
    Icon::Folder,
    Icon::File,
    Icon::Search,
    Icon::Cloud,
    Icon::Lock,
    Icon::Board,
    Icon::Terminal,
    Icon::Globe,
    Icon::Diff,
    Icon::List,
    Icon::Chart,
    Icon::Play,
    Icon::Pause,
    Icon::Stop,
    Icon::Refresh,
    Icon::Plus,
    Icon::Trash,
];
