//! Icons (design.md §2.9): Lucide via gpui-kit assets. [`OinoAssets`] embeds only the icons
//! the app vocabulary uses and falls back to gpui-kit's default component set (never the
//! full catalog).

use std::borrow::Cow;

use gpui_kit::assets::{Assets, IconName};
use gpui_kit::{
    AssetSource, Hsla, Result, SharedString, Styled as _, Svg, Transformation, px, svg,
};

gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        SquarePen,
        Kanban,
        Blocks,
        PanelLeft,
        PanelRight,
        PanelBottom,
        PanelLeftOpen,
        PanelRightOpen,
        Box,
        Brain,
        Plus,
        ArrowUp,
        Square,
        ListOrdered,
        ShieldCheck,
        MessageCircleQuestionMark,
        ListChecks,
        Bot,
        Users,
        MessageSquareDashed,
        Terminal,
        BrainCircuit,
        Moon,
        Bell,
        CircleArrowDown,
        Smartphone,
        SquareKanban,
        FileText,
        FilePen,
        SquareTerminal,
        FolderOpen,
        Globe,
        Plug,
        Hand,
        Sparkles,
        ShieldOff,
        GitBranch,
        Folder,
        Archive,
        CircleAlert,
        TriangleAlert,
        Info,
        CircleX,
        Check,
        X,
        Copy,
        Pencil,
        RotateCcw,
        ChevronDown,
        ChevronRight,
        ChevronsRight,
        ChevronLeft,
        ChevronUp,
        Lightbulb,
        ArrowUpToLine,
        ArrowDown,
        Clock,
        Settings,
        Zap,
        Lock,
        Cloud,
        Search,
        File,
        Dot,
        LoaderCircle,
        Pin,
        Paperclip,
        Ellipsis,
        Maximize2,
        Minimize2,
        Play,
        Pause,
        CircleCheck,
        CircleDashed,
        ArrowLeft,
        Keyboard,
        Monitor,
        Sun,
        Minus,
        Funnel,
        Trash,
        ExternalLink,
        Eye,
        EyeOff,
        RefreshCw,
        Download,
        ListTree,
        FileCode,
        Layers,
        SlidersHorizontal,
        CaseSensitive,
        GripVertical,
        CircleDot,
        Hourglass,
        LogOut,
        ArrowRight,
        ArrowUpDown,
        GitFork,
        TextCursorInput,
        Code,
        SquareSplitHorizontal,
        Columns2,
        LayoutGrid,
        BookOpen,
        Power,
        FolderSearch,
        RotateCw,
        AppWindow,
        Clipboard,
    ]
);

/// The app's asset source: the vocabulary above, then gpui-kit's component icons.
#[derive(Clone, Copy, Debug, Default)]
pub struct OinoAssets;

impl AssetSource for OinoAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match AppIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = AppIcons.list(path)?;
        all.extend(Assets.list(path)?);
        all.sort();
        all.dedup();
        Ok(all)
    }
}

/// An icon element of `size` px in `color`.
#[must_use]
pub fn icon(name: IconName, size: f32, color: Hsla) -> Svg {
    svg()
        .path(name.path())
        .flex_shrink_0()
        .size(px(size))
        .text_color(color)
}

/// A spinner glyph (`loader-circle`) rotated to ticker `frame` (12 steps per turn).
#[must_use]
pub fn spinner(frame: u32, size: f32, color: Hsla) -> Svg {
    let step = f32::from(u16::try_from(frame % 12).unwrap_or(0));
    icon(IconName::LoaderCircle, size, color).with_transformation(Transformation::rotate(
        gpui_kit::radians(step * std::f32::consts::TAU / 12.0),
    ))
}

/// The Lucide icon for an OUS icon name (ui-protocol §6.5).
#[must_use]
pub fn ous_icon(name: &str) -> Option<IconName> {
    Some(match crate::design::tones::ous_icon(name)? {
        "check" => IconName::Check,
        "x" => IconName::X,
        "dot" => IconName::Dot,
        "triangle-alert" => IconName::TriangleAlert,
        "info" => IconName::Info,
        "clock" => IconName::Clock,
        "settings" => IconName::Settings,
        "git-branch" => IconName::GitBranch,
        "zap" => IconName::Zap,
        "bot" => IconName::Bot,
        "folder" => IconName::Folder,
        "file" => IconName::File,
        "search" => IconName::Search,
        "cloud" => IconName::Cloud,
        "lock" => IconName::Lock,
        _ => return None,
    })
}
