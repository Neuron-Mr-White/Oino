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
        MessageSquare,
        Shield,
        Palette,
        EllipsisVertical,
        ALargeSmall,
        Wrench,
        Maximize,
        CornerDownRight,
        // round5 #5/#13: the blank-icon audit (sidebar Automations + cron use
        // `CalendarClock`; `CircleSlash`, `CornerDownLeft`, `FolderTree` were referenced
        // but never embedded — gpui-kit's default set lacks them too).
        CalendarClock,
        CircleSlash,
        CornerDownLeft,
        FolderTree,
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

/// The icon's asset path in kebab-case (`CalendarClock` → `icons/calendar-clock.svg`).
#[cfg(test)]
fn asset_path(variant: &str) -> String {
    let chars: Vec<char> = variant.chars().collect();
    let mut out = String::from("icons/");
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let after_lower_or_digit =
                i > 0 && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit());
            let acronym_end = i > 0
                && chars[i - 1].is_ascii_uppercase()
                && chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            if after_lower_or_digit || acronym_end {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_digit() && i > 0 && chars[i - 1].is_ascii_lowercase() {
            out.push('-');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out.push_str(".svg");
    out
}

#[cfg(test)]
mod tests {
    use super::{OinoAssets, asset_path};
    use gpui_kit::AssetSource as _;

    /// round5 #5/#13: every `IconName::` variant the app's source references resolves to
    /// embedded bytes (a blank icon renders nothing, so the audit runs against the
    /// vocabulary in the code, like the literal-styles scan).
    #[test]
    fn every_referenced_icon_is_embedded() {
        fn kebab(variant: &str) -> String {
            asset_path(variant)
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut variants: Vec<String> = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let mut rest = text.as_str();
                while let Some(at) = rest.find("IconName::") {
                    rest = &rest[at + "IconName::".len()..];
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() && !variants.contains(&name) {
                        variants.push(name);
                    }
                }
            }
        }
        assert!(
            variants.len() > 50,
            "the scan found no icon vocabulary ({})",
            variants.len()
        );
        for name in &variants {
            let path = kebab(name);
            let embedded = OinoAssets
                .load(&path)
                .ok()
                .flatten()
                .map(|b| !b.is_empty())
                .unwrap_or(false);
            assert!(embedded, "{name} renders blank: {path} is not embedded");
        }
    }

    #[test]
    fn kebab_paths_match_lucide_names() {
        assert_eq!(asset_path("CalendarClock"), "icons/calendar-clock.svg");
        assert_eq!(asset_path("ALargeSmall"), "icons/a-large-small.svg");
        assert_eq!(asset_path("Maximize2"), "icons/maximize-2.svg");
        assert_eq!(
            asset_path("MessageSquareDashed"),
            "icons/message-square-dashed.svg"
        );
    }
}
