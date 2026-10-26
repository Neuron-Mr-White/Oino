//! Namespace `footer` (unipi `src/config.ts`, `src/types.ts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Namespace, Section, choice, plain};

/// Namespace.
pub const NAMESPACE: &str = "footer";

/// Icon set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconStyle {
    /// Nerd Font glyphs (OUS icon names; the frontend picks glyphs).
    Nerd,
    /// Emoji glyphs in the text.
    Emoji,
    /// Plain labels (`branch:main`).
    Text,
}

/// Colour emission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Frontend decides.
    Auto,
    /// 24-bit.
    Truecolor,
    /// 256 colours.
    Ansi256,
    /// No colour (tones omitted).
    None,
}

impl ColorMode {
    /// Setting value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Truecolor => "truecolor",
            Self::Ansi256 => "256",
            Self::None => "none",
        }
    }
}

/// Rainbow parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rainbow {
    /// Brand always; whole frame at xhigh/max thinking.
    Always,
    /// Brand only.
    BrandOnly,
    /// No animation.
    Off,
}

/// Strip part toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the unipi settings object"
)]
pub struct Strip {
    /// Turns/steps.
    pub turns: bool,
    /// Model + tool time.
    pub time: bool,
    /// TTFT + tok/s.
    pub speed: bool,
    /// In/out tokens.
    pub tokens: bool,
    /// Cost.
    pub cost: bool,
    /// Compactions.
    pub compactions: bool,
    /// Cache hit %.
    pub cache: bool,
}

/// Frame badge toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the unipi settings object"
)]
pub struct Badges {
    /// Long-horizon mode label.
    pub mode: bool,
    /// PLAN badge + permission mode.
    pub plan_permission: bool,
    /// Fusion pair.
    pub fusion: bool,
    /// Kanboard claims.
    pub kanboard: bool,
}

/// Resolved footer settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FooterSettings {
    /// Footer renders at all.
    pub enabled: bool,
    /// Icon set.
    pub icon_style: IconStyle,
    /// Colour mode.
    pub color_mode: ColorMode,
    /// Rainbow parts.
    pub rainbow: Rainbow,
    /// Background-task line above the input.
    pub process_line: bool,
    /// Strip toggles.
    pub strip: Strip,
    /// Badge toggles.
    pub badges: Badges,
}

/// Defaults (unipi `DEFAULT_FOOTER_SETTINGS`).
#[must_use]
pub fn defaults() -> Value {
    json!({
        "enabled": true,
        "iconStyle": "nerd",
        "colorMode": "auto",
        "rainbow": "always",
        "processLine": false,
        "strip": {"turns": true, "time": true, "speed": true, "tokens": true, "cost": true, "compactions": true, "cache": true},
        "badges": {"mode": true, "planPermission": true, "fusion": true, "kanboard": true},
    })
}

impl FooterSettings {
    /// Validates a namespace object (unknown/legacy keys ignored, `mono` → `none`, invalid →
    /// defaults).
    #[must_use]
    pub fn from_json(v: &Value) -> Self {
        let b = |k: &str, d: bool| v.pointer(k).and_then(Value::as_bool).unwrap_or(d);
        let s = |k: &str| v.get(k).and_then(Value::as_str);
        Self {
            enabled: b("/enabled", true),
            icon_style: match s("iconStyle") {
                Some("emoji") => IconStyle::Emoji,
                Some("text") => IconStyle::Text,
                _ => IconStyle::Nerd,
            },
            color_mode: match s("colorMode") {
                Some("truecolor") => ColorMode::Truecolor,
                Some("256") => ColorMode::Ansi256,
                Some("none" | "mono") => ColorMode::None,
                _ => ColorMode::Auto,
            },
            rainbow: match s("rainbow") {
                Some("brand-only") => Rainbow::BrandOnly,
                Some("off") => Rainbow::Off,
                _ => Rainbow::Always,
            },
            process_line: b("/processLine", false),
            strip: Strip {
                turns: b("/strip/turns", true),
                time: b("/strip/time", true),
                speed: b("/strip/speed", true),
                tokens: b("/strip/tokens", true),
                cost: b("/strip/cost", true),
                compactions: b("/strip/compactions", true),
                cache: b("/strip/cache", true),
            },
            badges: Badges {
                mode: b("/badges/mode", true),
                plan_permission: b("/badges/planPermission", true),
                fusion: b("/badges/fusion", true),
                kanboard: b("/badges/kanboard", true),
            },
        }
    }
}

/// Schema.
#[must_use]
pub fn namespace() -> Namespace {
    let strip = |key: &str, label: &str, d: &str| Field::boolean(key, label).description(d);
    Namespace::new(NAMESPACE, "Footer", crate::EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("general", "General")
                .field(Field::boolean("enabled", "Footer enabled").description("The glance frame, stats strip and process line. Off leaves the plain pi editor."))
                .field(
                    Field::enumeration("colorMode", "Color mode", ["auto", "truecolor", "256", "none"].into_iter().map(plain).collect())
                        .description("How much color the footer emits."),
                )
                .field(
                    Field::enumeration(
                        "iconStyle",
                        "Icon style",
                        vec![
                            choice("emoji", "emoji", "emoji glyphs, always render"),
                            choice("nerd", "nerd font", "nerd-font glyphs, need a patched font"),
                            choice("text", "text only", "no icons, plain text labels"),
                        ],
                    )
                    .description("Glyph set for the brand and frame titles."),
                ),
        )
        .section(
            Section::new("glance", "Glance")
                .description("The stats strip below the input and the frame badges.")
                .field(
                    Field::enumeration("rainbow", "Rainbow", ["always", "brand-only", "off"].into_iter().map(plain).collect())
                        .description("Which frame parts get the animated rainbow."),
                )
                .field(strip("processLine", "Background tasks line", "Extra one-liner above the input counting background tasks by status (off by default — they live in the ↓ work tray)."))
                .field(strip("strip.turns", "Strip: turns", "Turn and step counters in the stats strip."))
                .field(strip("strip.time", "Strip: time", "Model time and tool time in the stats strip."))
                .field(strip("strip.speed", "Strip: speed", "Average time to first token and tokens per second."))
                .field(strip("strip.tokens", "Strip: tokens", "Session input and output token totals."))
                .field(strip("strip.cost", "Strip: cost", "Session cost, or sub on subscription models."))
                .field(strip("strip.compactions", "Strip: compactions", "Compaction count, sizes and recency."))
                .field(strip("strip.cache", "Strip: cache", "Cache hit percentage."))
                .field(strip("badges.mode", "Badge: mode", "Long-horizon mode label beside the brand."))
                .field(strip("badges.planPermission", "Badge: plan/permission", "PLAN badge and permission mode in the top border."))
                .field(strip("badges.fusion", "Badge: fusion", "Fusion lead and sidekick in the bottom border."))
                .field(strip("badges.kanboard", "Badge: kanboard", "Kanboard claims label in the top border.")),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_legacy_values() {
        let d = FooterSettings::from_json(&defaults());
        assert_eq!(d, FooterSettings::from_json(&json!({})));
        assert!(d.enabled && !d.process_line);
        assert_eq!(d.icon_style, IconStyle::Nerd);
        assert_eq!(
            FooterSettings::from_json(&json!({"colorMode": "mono"})).color_mode,
            ColorMode::None
        );
        let old = FooterSettings::from_json(
            &json!({"preset": "x", "separator": "|", "groups": {}, "glanceMode": true}),
        );
        assert_eq!(old, d, "old classic keys are ignored");
        let bad = FooterSettings::from_json(
            &json!({"enabled": "yes", "iconStyle": 3, "rainbow": "loud", "strip": {"cost": "no", "cache": false}}),
        );
        assert!(bad.enabled);
        assert_eq!(bad.rainbow, Rainbow::Always);
        assert!(bad.strip.cost && !bad.strip.cache);
    }

    #[test]
    fn schema_matches_unipi() {
        let v = namespace().to_json();
        assert_eq!(v["sections"][0]["fields"][1]["options"][2]["value"], "256");
        assert_eq!(
            v["sections"][1]["fields"].as_array().map(Vec::len),
            Some(13)
        );
        assert_eq!(v["sections"][1]["fields"][2]["key"], "strip.turns");
        assert_eq!(v["sections"][1]["fields"][2]["default"], true);
        assert_eq!(v["sections"][1]["fields"][1]["default"], false);
    }
}
