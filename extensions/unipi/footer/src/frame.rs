//! The glance frame (unipi `src/glance-editor.ts`, `segments/long-horizon.ts`, core
//! `kanboardGlanceLabel`) as `setFrame` segments.
//!
//! unipi drew the frame itself; Oino frontends lay out segments and drop the lowest
//! `priority` first when narrow, never writing the last column. Priorities reproduce unipi's
//! responsive rules: badges leave in the order kanboard → fusion → plan/permission → mode (the
//! permission label before PLAN); the thinking slot shrinks before the model; the branch, model
//! and brand only truncate.

use serde_json::{Value, json};
use unipi_kit::topics::{FusionStatus, KanboardStatus, LhState};

use crate::settings::{Badges, IconStyle, Rainbow};

/// Frame key.
pub const FRAME_KEY: &str = "glance";

/// Segment priorities (higher survives longer).
pub mod priority {
    /// UNIPI brand.
    pub const BRAND: i32 = 100;
    /// Git branch (truncates only).
    pub const BRANCH: i32 = 95;
    /// Workspace name.
    pub const WORKSPACE: i32 = 90;
    /// Context use.
    pub const CONTEXT: i32 = 88;
    /// Model (truncates only).
    pub const MODEL: i32 = 85;
    /// Thinking level (shrinks first in the bottom cluster).
    pub const THINKING: i32 = 60;
    /// Long-horizon mode badge.
    pub const MODE: i32 = 50;
    /// PLAN badge.
    pub const PLAN: i32 = 42;
    /// Permission mode label.
    pub const PERMISSION: i32 = 40;
    /// Fusion pair (falls back to the model).
    pub const FUSION: i32 = 30;
    /// Kanboard claims.
    pub const KANBOARD: i32 = 20;
}

/// Long-horizon mode labels.
pub const MODE_LABELS: &[(&str, &str)] = &[
    ("goal", "Goal Mode"),
    ("ralph", "Ralph Mode"),
    ("swarm", "Swarm Mode"),
    ("graph", "Graph Mode"),
    ("none", "Regular Mode"),
];

fn mode_label(id: &str) -> String {
    MODE_LABELS
        .iter()
        .find(|(k, _)| *k == id)
        .map_or_else(|| id.to_owned(), |(_, l)| (*l).to_owned())
}

/// unipi `lhModeLabel`: a parked owner while regular shows `<Mode> · paused`.
#[must_use]
pub fn lh_mode_label(state: Option<&LhState>) -> Option<String> {
    let s = state?;
    if let Some(p) = s.paused.as_deref().filter(|_| s.mode == "none") {
        return Some(format!("{} · paused", mode_label(p)));
    }
    Some(mode_label(&s.mode))
}

/// unipi `kanboardGlanceLabel`: `▣ T-1`, `▣ T-1 +2`, `▣ T-1 · autowork`, `▣ autowork`.
#[must_use]
pub fn kanboard_label(status: Option<&KanboardStatus>) -> Option<String> {
    let s = status?;
    if s.claims.is_empty() && !s.autowork {
        return None;
    }
    let claims = match s.claims.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        [first, rest @ ..] => Some(format!("{first} +{}", rest.len())),
    };
    let body: Vec<String> = claims
        .into_iter()
        .chain(s.autowork.then(|| "autowork".to_owned()))
        .collect();
    Some(format!("▣ {}", body.join(" · ")))
}

/// unipi `renderFusionStatus` as spans: lead lit, sidekick muted (flipped while busy), split
/// and savings dim.
#[must_use]
pub fn fusion_spans(f: &FusionStatus) -> Vec<Value> {
    let lead = if f.lead_effort.is_empty() {
        f.lead_name.clone()
    } else {
        format!("{} {}", f.lead_name, f.lead_effort)
    };
    let side = if f.sidekick_effort.is_empty() {
        f.sidekick_name.clone()
    } else {
        format!("{} {}", f.sidekick_name, f.sidekick_effort)
    };
    let calls = f.lead_tool_calls.unwrap_or(0) + f.sidekick_tool_calls.unwrap_or(0);
    let split = if calls > 0 {
        format!(" · {}/{calls} calls", f.sidekick_tool_calls.unwrap_or(0))
    } else {
        String::new()
    };
    let saved = f
        .saved_usd
        .filter(|s| *s > 0.005)
        .map(|s| format!(" · saved ${s:.2}"))
        .unwrap_or_default();
    let mut spans = if f.busy == Some(true) {
        vec![
            json!({"text": format!("Fusion · {lead}"), "dim": true}),
            json!({"text": " "}),
            json!({"text": format!("◆ {side} · working"), "tone": "success", "bold": true}),
        ]
    } else {
        vec![
            json!({"text": format!("Fusion · {lead}"), "tone": "success", "bold": true}),
            json!({"text": " "}),
            json!({"text": format!("◆ {side}"), "dim": true}),
        ]
    };
    let tail = format!("{split}{saved}");
    if !tail.is_empty() {
        spans.push(json!({"text": tail, "dim": true}));
    }
    spans
}

/// Plain text of spans.
#[must_use]
pub fn spans_text(spans: &[Value]) -> String {
    spans
        .iter()
        .filter_map(|s| s.get("text").and_then(Value::as_str))
        .collect()
}

/// Frame `fmtTokens`: `1.0M`, `200k`, `999`.
#[must_use]
pub fn fmt_window(n: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "display")]
    let f = n as f64;
    if n >= 1_000_000 {
        format!("{:.1}M", f / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", (f / 1000.0).round())
    } else {
        n.to_string()
    }
}

/// Everything the frame shows (unipi `GlanceStatus`).
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// Workspace directory name.
    pub workspace: String,
    /// Long-horizon mode label.
    pub lh_mode: Option<String>,
    /// Plan mode on.
    pub plan_mode: bool,
    /// Permission mode.
    pub permission_mode: Option<String>,
    /// Git branch.
    pub branch: Option<String>,
    /// Context use (%).
    pub context_pct: Option<f64>,
    /// Context window (tokens).
    pub context_window: u64,
    /// Model display name.
    pub model_name: String,
    /// Thinking level.
    pub thinking: Option<String>,
    /// Fusion pair.
    pub fusion: Option<FusionStatus>,
    /// Kanboard snapshot.
    pub kanboard: Option<KanboardStatus>,
    /// Rainbow mode.
    pub rainbow: Rainbow,
    /// Badge toggles.
    pub badges: Badges,
    /// Icon style.
    pub icon_style: IconStyle,
    /// Emit tones.
    pub color: bool,
}

struct Seg {
    v: Value,
    color: bool,
}

impl Seg {
    fn new(text: impl Into<String>, priority: i32, align: &str, color: bool) -> Self {
        Self {
            v: json!({"text": text.into(), "priority": priority, "align": align}),
            color,
        }
    }
    fn id(mut self, id: &str) -> Self {
        self.v["id"] = json!(id);
        self
    }
    fn tone(mut self, tone: &str) -> Self {
        if self.color {
            self.v["tone"] = json!(tone);
        }
        self
    }
    fn icon(mut self, icon: Option<&str>) -> Self {
        if let Some(i) = icon {
            self.v["icon"] = json!(i);
        }
        self
    }
    fn set(mut self, key: &str, value: Value) -> Self {
        self.v[key] = value;
        self
    }
    fn done(self) -> Value {
        self.v
    }
}

/// `(nerd icon name, emoji glyph, text label)` per frame slot.
fn glyph(style: IconStyle, slot: &str) -> (Option<&'static str>, &'static str) {
    let (icon, emoji) = match slot {
        "brand" => ("bot", "🤖"),
        "branch" => ("branch", "🔀"),
        _ => ("folder", "📁"),
    };
    match style {
        IconStyle::Nerd => (Some(icon), ""),
        IconStyle::Emoji => (None, emoji),
        IconStyle::Text => (None, ""),
    }
}

fn prefixed(style: IconStyle, slot: &str, text: &str) -> (Option<&'static str>, String) {
    let (icon, emoji) = glyph(style, slot);
    let t = if emoji.is_empty() {
        text.to_owned()
    } else {
        format!("{emoji} {text}")
    };
    (icon, t)
}

/// Top and bottom segments plus frame-wide options as a `setFrame` request.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
pub fn frame_request(st: &Status) -> Value {
    let c = st.color;
    let text_style = st.icon_style == IconStyle::Text;
    let mut top = Vec::new();
    let (icon, brand) = prefixed(st.icon_style, "brand", "UNIPI");
    let mut b = Seg::new(brand, priority::BRAND, "left", c)
        .id("brand")
        .icon(icon)
        .set("bold", json!(true));
    if st.rainbow != Rainbow::Off {
        b = b.set("effect", json!("rainbow"));
    }
    top.push(b.done());
    if st.badges.mode
        && let Some(m) = &st.lh_mode
    {
        let t = if text_style {
            format!("mode:{m}")
        } else {
            m.clone()
        };
        top.push(
            Seg::new(t, priority::MODE, "left", c)
                .id("mode")
                .tone("accent")
                .done(),
        );
    }
    if st.badges.kanboard
        && let Some(k) = kanboard_label(st.kanboard.as_ref())
    {
        let t = if text_style {
            k.replacen("▣ ", "kanboard:", 1)
        } else {
            k
        };
        top.push(
            Seg::new(t, priority::KANBOARD, "left", c)
                .id("kanboard")
                .tone("c10")
                .done(),
        );
    }
    if let Some(br) = &st.branch {
        let (icon, t) = if text_style {
            (None, format!("branch:{br}"))
        } else {
            prefixed(st.icon_style, "branch", br)
        };
        top.push(
            Seg::new(t, priority::BRANCH, "left", c)
                .id("branch")
                .icon(icon)
                .tone("muted")
                .done(),
        );
    }
    if st.badges.plan_permission {
        if st.plan_mode {
            top.push(
                Seg::new("PLAN", priority::PLAN, "right", c)
                    .id("plan")
                    .tone("warning")
                    .set("bold", json!(true))
                    .set("style", json!("badge"))
                    .done(),
            );
        }
        if let Some(p) = &st.permission_mode {
            let tone = match p.as_str() {
                "full" => "warning",
                "ask" => "success",
                _ => "muted",
            };
            top.push(
                Seg::new(p.clone(), priority::PERMISSION, "right", c)
                    .id("permission")
                    .tone(tone)
                    .set("bold", json!(p == "ask"))
                    .done(),
            );
        }
    }

    let mut bottom = Vec::new();
    let (icon, ws) = if text_style {
        (None, format!("workspace:{}", st.workspace))
    } else {
        prefixed(st.icon_style, "workspace", &st.workspace)
    };
    bottom.push(
        Seg::new(ws, priority::WORKSPACE, "left", c)
            .id("workspace")
            .icon(icon)
            .tone("muted")
            .done(),
    );
    let pct = st.context_pct.map_or_else(
        || "?%".to_owned(),
        |p| format!("{}%", p.clamp(0.0, 100.0).round()),
    );
    let win = if st.context_window > 0 {
        format!("/{}", fmt_window(st.context_window))
    } else {
        String::new()
    };
    let ctx_tone = match st.context_pct {
        Some(p) if p > 90.0 => "error",
        Some(p) if p > 70.0 => "warning",
        _ => "default",
    };
    bottom.push(
        Seg::new(format!("{pct}{win}"), priority::CONTEXT, "right", c)
            .id("context")
            .tone(ctx_tone)
            .done(),
    );
    let thinking = st
        .thinking
        .as_deref()
        .filter(|t| *t != "off")
        .map(|t| format!("thinking:{t}"));
    let model_seg = (!st.model_name.is_empty()).then(|| {
        Seg::new(st.model_name.clone(), priority::MODEL, "right", c)
            .id("model")
            .tone("assistant")
            .done()
    });
    let thinking_seg = thinking.clone().map(|t| {
        Seg::new(t, priority::THINKING, "right", c)
            .id("thinking")
            .tone("thinking")
            .done()
    });
    match st.fusion.as_ref().filter(|_| st.badges.fusion) {
        Some(f) => {
            let spans = fusion_spans(f);
            let fallback_text = [
                Some(st.model_name.clone()).filter(|m| !m.is_empty()),
                thinking,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" │ ");
            let mut seg = Seg::new(spans_text(&spans), priority::FUSION, "right", c)
                .id("fusion")
                .set("spans", Value::Array(spans));
            if !fallback_text.is_empty() {
                seg = seg.set(
                    "fallback",
                    Seg::new(fallback_text, priority::MODEL, "right", c)
                        .id("model")
                        .tone("assistant")
                        .done(),
                );
            }
            bottom.push(seg.done());
        }
        None => bottom.extend(model_seg.into_iter().chain(thinking_seg)),
    }

    let hot = matches!(st.thinking.as_deref(), Some("max" | "xhigh"));
    let mut req =
        json!({"method": "setFrame", "frameKey": FRAME_KEY, "top": top, "bottom": bottom});
    if st.rainbow == Rainbow::Always && hot {
        req["effect"] = json!("rainbow");
    }
    req
}

/// `setFrame` that clears the frame.
#[must_use]
pub fn clear_request() -> Value {
    json!({"method": "setFrame", "frameKey": FRAME_KEY})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            workspace: "unipi".into(),
            lh_mode: Some("Goal Mode".into()),
            plan_mode: true,
            permission_mode: Some("auto".into()),
            branch: Some("feat/x".into()),
            context_pct: Some(42.4),
            context_window: 1_000_000,
            model_name: "Opus 4.5".into(),
            thinking: Some("high".into()),
            fusion: None,
            kanboard: Some(KanboardStatus {
                claims: vec!["T-1".into(), "T-2".into()],
                autowork: false,
                cli: None,
            }),
            rainbow: Rainbow::Always,
            badges: Badges {
                mode: true,
                plan_permission: true,
                fusion: true,
                kanboard: true,
            },
            icon_style: IconStyle::Nerd,
            color: true,
        }
    }

    fn ids(v: &Value, side: &str) -> Vec<String> {
        v[side]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["id"].as_str().map(ToOwned::to_owned))
            .collect()
    }

    fn by_id<'a>(v: &'a Value, id: &str) -> &'a Value {
        ["top", "bottom"]
            .iter()
            .flat_map(|s| v[*s].as_array().into_iter().flatten())
            .find(|s| s["id"] == id)
            .unwrap_or(&Value::Null)
    }

    #[test]
    fn labels() {
        assert_eq!(lh_mode_label(None), None);
        let s = |mode: &str, paused: Option<&str>| LhState {
            mode: mode.into(),
            paused: paused.map(Into::into),
            owner: None,
            last_stop: None,
        };
        assert_eq!(
            lh_mode_label(Some(&s("goal", None))).as_deref(),
            Some("Goal Mode")
        );
        assert_eq!(
            lh_mode_label(Some(&s("none", Some("ralph")))).as_deref(),
            Some("Ralph Mode · paused")
        );
        assert_eq!(
            lh_mode_label(Some(&s("none", None))).as_deref(),
            Some("Regular Mode")
        );
        assert_eq!(lh_mode_label(Some(&s("odd", None))).as_deref(), Some("odd"));
        let k = |claims: &[&str], autowork| KanboardStatus {
            claims: claims.iter().map(|c| (*c).to_owned()).collect(),
            autowork,
            cli: None,
        };
        assert_eq!(kanboard_label(Some(&k(&[], false))), None);
        assert_eq!(
            kanboard_label(Some(&k(&["T-1"], false))).as_deref(),
            Some("▣ T-1")
        );
        assert_eq!(
            kanboard_label(Some(&k(&["T-1", "T-2", "T-3"], false))).as_deref(),
            Some("▣ T-1 +2")
        );
        assert_eq!(
            kanboard_label(Some(&k(&["T-1"], true))).as_deref(),
            Some("▣ T-1 · autowork")
        );
        assert_eq!(
            kanboard_label(Some(&k(&[], true))).as_deref(),
            Some("▣ autowork")
        );
    }

    #[test]
    fn segments_and_drop_order() {
        let f = frame_request(&status());
        assert_eq!(
            ids(&f, "top"),
            ["brand", "mode", "kanboard", "branch", "plan", "permission"]
        );
        assert_eq!(
            ids(&f, "bottom"),
            ["workspace", "context", "model", "thinking"]
        );
        assert_eq!(by_id(&f, "context")["text"], "42%/1.0M");
        assert_eq!(by_id(&f, "thinking")["text"], "thinking:high");
        assert_eq!(by_id(&f, "brand")["effect"], "rainbow");
        assert_eq!(by_id(&f, "brand")["icon"], "bot");
        assert!(f.get("effect").is_none());
        let p = |id: &str| by_id(&f, id)["priority"].as_i64().unwrap_or_default();
        // kanboard → fusion → plan/permission → mode; permission before PLAN; thinking before model.
        assert!(
            p("kanboard") < p("permission") && p("permission") < p("plan") && p("plan") < p("mode")
        );
        assert!(p("thinking") < p("model") && p("mode") < p("branch") && p("branch") < p("brand"));
    }

    #[test]
    fn toggles_text_style_and_rainbow() {
        let mut st = status();
        st.badges = Badges {
            mode: false,
            plan_permission: false,
            fusion: true,
            kanboard: false,
        };
        st.icon_style = IconStyle::Text;
        st.rainbow = Rainbow::Off;
        st.thinking = Some("off".into());
        let f = frame_request(&st);
        assert_eq!(ids(&f, "top"), ["brand", "branch"]);
        assert_eq!(by_id(&f, "branch")["text"], "branch:feat/x");
        assert_eq!(by_id(&f, "workspace")["text"], "workspace:unipi");
        assert!(by_id(&f, "brand").get("effect").is_none());
        assert_eq!(ids(&f, "bottom"), ["workspace", "context", "model"]);
        let mut st = status();
        st.icon_style = IconStyle::Text;
        st.thinking = Some("xhigh".into());
        let f = frame_request(&st);
        assert_eq!(by_id(&f, "kanboard")["text"], "kanboard:T-1 +1");
        assert_eq!(by_id(&f, "mode")["text"], "mode:Goal Mode");
        assert_eq!(f["effect"], "rainbow", "hot thinking + always");
        let mut st = status();
        st.icon_style = IconStyle::Emoji;
        st.color = false;
        let f = frame_request(&st);
        assert_eq!(by_id(&f, "branch")["text"], "🔀 feat/x");
        assert!(by_id(&f, "permission").get("tone").is_none());
    }

    #[test]
    fn fusion_replaces_model_with_fallback() {
        let mut st = status();
        st.fusion = Some(FusionStatus {
            lead_name: "Opus".into(),
            lead_effort: "high".into(),
            sidekick_name: "Haiku".into(),
            sidekick_effort: "low".into(),
            busy: Some(true),
            lead_tool_calls: Some(3),
            sidekick_tool_calls: Some(1),
            saved_usd: Some(0.42),
            ..FusionStatus::default()
        });
        let f = frame_request(&st);
        assert_eq!(ids(&f, "bottom"), ["workspace", "context", "fusion"]);
        let fu = by_id(&f, "fusion");
        assert_eq!(
            fu["text"],
            "Fusion · Opus high ◆ Haiku low · working · 1/4 calls · saved $0.42"
        );
        assert_eq!(fu["fallback"]["text"], "Opus 4.5 │ thinking:high");
        st.badges.fusion = false;
        assert_eq!(
            ids(&frame_request(&st), "bottom"),
            ["workspace", "context", "model", "thinking"]
        );
        let idle = FusionStatus {
            lead_name: "A".into(),
            lead_effort: String::new(),
            sidekick_name: "B".into(),
            sidekick_effort: String::new(),
            ..FusionStatus::default()
        };
        assert_eq!(spans_text(&fusion_spans(&idle)), "Fusion · A ◆ B");
    }

    #[test]
    fn window_format() {
        assert_eq!(fmt_window(1_000_000), "1.0M");
        assert_eq!(fmt_window(200_000), "200k");
        assert_eq!(fmt_window(999), "999");
        let mut st = status();
        st.context_pct = None;
        st.context_window = 0;
        assert_eq!(by_id(&frame_request(&st), "context")["text"], "?%");
    }
}
