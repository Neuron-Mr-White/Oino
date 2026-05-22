#![forbid(unsafe_code)]

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Theme {
    pub bg: Color,
    pub panel_bg: Color,
    pub elevated_bg: Color,
    pub composer_bg: Color,
    pub selection_bg: Color,
    pub selected_fg: Color,
    pub fg: Color,
    pub accent: Color,
    pub success: Color,
    pub muted: Color,
    pub dim: Color,
    pub focused_border: Color,
    pub panel_border: Color,
    pub user_border: Color,
    pub assistant_border: Color,
    pub tool_border: Color,
    pub error: Style,
    pub warning: Style,
    pub placeholder: Style,
    pub footer: Style,
    pub working: Style,
    pub title: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Reset,
            panel_bg: Color::Reset,
            elevated_bg: Color::Reset,
            composer_bg: Color::Reset,
            selection_bg: Color::DarkGray,
            selected_fg: Color::Reset,
            fg: Color::Reset,
            accent: Color::Cyan,
            success: Color::Green,
            muted: Color::DarkGray,
            dim: Color::DarkGray,
            focused_border: Color::Cyan,
            panel_border: Color::DarkGray,
            user_border: Color::Blue,
            assistant_border: Color::Green,
            tool_border: Color::Yellow,
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            warning: Style::default().fg(Color::Yellow),
            placeholder: Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
            footer: Style::default().fg(Color::DarkGray),
            working: Style::default().fg(Color::Yellow),
            title: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        }
    }
}

pub(crate) fn theme_cache_hash(theme: &Theme) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    theme.hash(&mut hasher);
    hasher.finish()
}

impl Theme {
    #[must_use]
    pub fn bubble_border_for_role(&self, role: &str, is_error: bool) -> Style {
        if is_error {
            return self.error;
        }
        let color = match role {
            "user" => self.user_border,
            "assistant" => self.assistant_border,
            role if role.starts_with("tool:") => self.tool_border,
            _ => self.panel_border,
        };
        Style::default().fg(color)
    }

    #[must_use]
    pub fn from_resolved_theme(resolved: &ResolvedTheme) -> Self {
        let mut theme = Self::default();
        for (token, color) in &resolved.tokens {
            apply_color_token(&mut theme, token, *color);
        }
        theme
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
    Mono,
}

impl ThemeMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Mono => "mono",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub active: Option<String>,
    pub overrides: BTreeMap<String, String>,
}

impl ThemeSettings {
    #[must_use]
    pub fn active_id(&self) -> Option<String> {
        self.active
            .as_deref()
            .and_then(normalize_theme_id)
            .filter(|id| !id.is_empty())
    }

    pub fn set_active(&mut self, id: impl Into<String>) {
        self.active = normalize_theme_id(&id.into());
    }

    pub fn clear_active(&mut self) {
        self.active = None;
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_none() && self.overrides.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeDocument {
    pub schema_version: u16,
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub mode: ThemeMode,
    pub inherits: Option<String>,
    pub palette: BTreeMap<String, String>,
    pub tokens: BTreeMap<String, String>,
}

impl Default for ThemeDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            id: String::new(),
            display_name: String::new(),
            description: None,
            mode: ThemeMode::System,
            inherits: None,
            palette: BTreeMap::new(),
            tokens: BTreeMap::new(),
        }
    }
}

impl ThemeDocument {
    pub fn from_json_str(text: &str) -> serde_json::Result<Self> {
        serde_json::from_str(text)
    }

    #[must_use]
    pub fn normalized_id(&self) -> Option<String> {
        normalize_theme_id(&self.id)
    }

    #[must_use]
    pub fn validate(&self) -> Vec<ThemeDiagnostic> {
        let mut diagnostics = Vec::new();
        if self.schema_version != 1 {
            diagnostics.push(ThemeDiagnostic::error(format!(
                "unsupported theme schema version `{}`; expected `1`",
                self.schema_version
            )));
        }
        if self.normalized_id().is_none() {
            diagnostics.push(ThemeDiagnostic::error("theme id is required"));
        }
        if self.display_name.trim().is_empty() {
            diagnostics.push(ThemeDiagnostic::warning("theme display_name is empty"));
        }
        for (name, value) in &self.palette {
            if parse_theme_color(value).is_none() {
                diagnostics.push(ThemeDiagnostic::warning(format!(
                    "palette color `{name}` has invalid value `{value}`"
                )));
            }
        }
        for (token, value) in &self.tokens {
            let normalized = normalize_theme_token(token);
            if !is_known_theme_token(&normalized) {
                diagnostics.push(ThemeDiagnostic::warning(format!(
                    "unknown theme token `{token}` accepted for forward compatibility"
                )));
            }
            if value.trim().starts_with("$palette.") {
                let name = value.trim().trim_start_matches("$palette.");
                if !self.palette.contains_key(name) {
                    diagnostics.push(ThemeDiagnostic::warning(format!(
                        "theme token `{token}` references missing palette color `{name}`"
                    )));
                }
            } else if parse_theme_color(value).is_none() {
                diagnostics.push(ThemeDiagnostic::warning(format!(
                    "theme token `{token}` has invalid color `{value}`"
                )));
            }
        }
        diagnostics
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeDiagnosticLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDiagnostic {
    pub level: ThemeDiagnosticLevel,
    pub message: String,
}

impl ThemeDiagnostic {
    #[must_use]
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            level: ThemeDiagnosticLevel::Warning,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: ThemeDiagnosticLevel::Error,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSourceKind {
    BuiltIn,
    File,
    Extension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSourceScope {
    BuiltIn,
    Global,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeSource {
    pub kind: ThemeSourceKind,
    pub scope: ThemeSourceScope,
}

impl ThemeSource {
    pub const BUILT_IN: Self = Self {
        kind: ThemeSourceKind::BuiltIn,
        scope: ThemeSourceScope::BuiltIn,
    };

    #[must_use]
    pub const fn precedence_rank(self) -> u8 {
        match (self.kind, self.scope) {
            (ThemeSourceKind::BuiltIn, _) | (_, ThemeSourceScope::BuiltIn) => 0,
            (ThemeSourceKind::Extension, ThemeSourceScope::Global) => 1,
            (ThemeSourceKind::File, ThemeSourceScope::Global) => 2,
            (ThemeSourceKind::Extension, ThemeSourceScope::Project) => 3,
            (ThemeSourceKind::File, ThemeSourceScope::Project) => 4,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match (self.kind, self.scope) {
            (ThemeSourceKind::BuiltIn, _) | (_, ThemeSourceScope::BuiltIn) => "built-in",
            (ThemeSourceKind::File, ThemeSourceScope::Global) => "global file",
            (ThemeSourceKind::File, ThemeSourceScope::Project) => "project file",
            (ThemeSourceKind::Extension, ThemeSourceScope::Global) => "global extension",
            (ThemeSourceKind::Extension, ThemeSourceScope::Project) => "project extension",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeCatalogEntry {
    pub source: ThemeSource,
    pub document: ThemeDocument,
}

impl ThemeCatalogEntry {
    #[must_use]
    pub const fn new(source: ThemeSource, document: ThemeDocument) -> Self {
        Self { source, document }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeCatalog {
    entries: Vec<ThemeCatalogEntry>,
}

impl ThemeCatalog {
    #[must_use]
    pub fn builtins() -> Self {
        let mut catalog = Self::default();
        for document in builtin_theme_documents() {
            catalog.register(ThemeCatalogEntry::new(ThemeSource::BUILT_IN, document));
        }
        catalog
    }

    pub fn register(&mut self, entry: ThemeCatalogEntry) {
        self.entries.push(entry);
    }

    #[must_use]
    pub fn entries(&self) -> &[ThemeCatalogEntry] {
        &self.entries
    }

    #[must_use]
    pub fn candidates(&self, id: &str) -> Vec<&ThemeCatalogEntry> {
        let Some(normalized) = normalize_theme_id(id) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|entry| entry.document.normalized_id().as_deref() == Some(normalized.as_str()))
            .collect()
    }

    #[must_use]
    pub fn selected_entry(&self, id: &str) -> Option<&ThemeCatalogEntry> {
        self.candidates(id)
            .into_iter()
            .max_by_key(|entry| entry.source.precedence_rank())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveThemeScope {
    Default,
    Global,
    Project,
}

impl EffectiveThemeScope {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Global => "global",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTheme {
    pub id: String,
    pub display_name: String,
    pub mode: ThemeMode,
    pub selected_scope: EffectiveThemeScope,
    pub source: ThemeSource,
    pub tokens: BTreeMap<String, Color>,
    pub raw_tokens: BTreeMap<String, String>,
    pub diagnostics: Vec<ThemeDiagnostic>,
}

#[must_use]
pub fn resolve_effective_theme(
    catalog: &ThemeCatalog,
    global: &ThemeSettings,
    project: &ThemeSettings,
) -> ResolvedTheme {
    let (requested_id, selected_scope) = if let Some(id) = project.active_id() {
        (id, EffectiveThemeScope::Project)
    } else if let Some(id) = global.active_id() {
        (id, EffectiveThemeScope::Global)
    } else {
        ("system".into(), EffectiveThemeScope::Default)
    };

    let (entry, mut diagnostics) = match catalog.selected_entry(&requested_id) {
        Some(entry) => (entry, Vec::new()),
        None => {
            let fallback = catalog
                .selected_entry("system")
                .or_else(|| catalog.selected_entry("oino-dark"))
                .unwrap_or_else(|| {
                    panic!("built-in theme catalog must contain system or oino-dark")
                });
            (
                fallback,
                vec![ThemeDiagnostic::warning(format!(
                    "theme `{requested_id}` unavailable; using `{}`",
                    fallback.document.id
                ))],
            )
        }
    };

    let mut chain = Vec::new();
    let mut raw = default_raw_theme_tokens();
    let mut palette = default_palette();
    merge_theme_document(
        catalog,
        &entry.document,
        &mut palette,
        &mut raw,
        &mut chain,
        &mut diagnostics,
    );

    match selected_scope {
        EffectiveThemeScope::Project => {
            apply_raw_overrides(&mut raw, &project.overrides);
        }
        EffectiveThemeScope::Global | EffectiveThemeScope::Default => {
            apply_raw_overrides(&mut raw, &global.overrides);
            apply_raw_overrides(&mut raw, &project.overrides);
        }
    }

    let tokens = resolve_raw_tokens(&palette, &raw, &mut diagnostics);
    diagnostics.extend(entry.document.validate());
    let id = entry
        .document
        .normalized_id()
        .unwrap_or_else(|| entry.document.id.clone());
    ResolvedTheme {
        id,
        display_name: if entry.document.display_name.trim().is_empty() {
            entry.document.id.clone()
        } else {
            entry.document.display_name.clone()
        },
        mode: entry.document.mode,
        selected_scope,
        source: entry.source,
        tokens,
        raw_tokens: raw,
        diagnostics,
    }
}

fn merge_theme_document(
    catalog: &ThemeCatalog,
    document: &ThemeDocument,
    palette: &mut BTreeMap<String, String>,
    raw: &mut BTreeMap<String, String>,
    chain: &mut Vec<String>,
    diagnostics: &mut Vec<ThemeDiagnostic>,
) {
    let Some(id) = document.normalized_id() else {
        diagnostics.push(ThemeDiagnostic::error("cannot merge theme with empty id"));
        return;
    };
    if chain.contains(&id) {
        diagnostics.push(ThemeDiagnostic::error(format!(
            "theme inheritance cycle detected at `{id}`"
        )));
        return;
    }
    chain.push(id.clone());
    if let Some(parent_id) = document.inherits.as_deref().and_then(normalize_theme_id) {
        if let Some(parent) = catalog.selected_entry(&parent_id) {
            merge_theme_document(catalog, &parent.document, palette, raw, chain, diagnostics);
        } else {
            diagnostics.push(ThemeDiagnostic::warning(format!(
                "theme `{id}` inherits missing theme `{parent_id}`"
            )));
        }
    }
    palette.extend(document.palette.clone());
    apply_raw_overrides(raw, &document.tokens);
    let _ = chain.pop();
}

fn apply_raw_overrides(raw: &mut BTreeMap<String, String>, overrides: &BTreeMap<String, String>) {
    for (token, value) in overrides {
        raw.insert(normalize_theme_token(token), value.clone());
    }
}

fn resolve_raw_tokens(
    palette: &BTreeMap<String, String>,
    raw: &BTreeMap<String, String>,
    diagnostics: &mut Vec<ThemeDiagnostic>,
) -> BTreeMap<String, Color> {
    let mut out = BTreeMap::new();
    for (token, value) in raw {
        let value = value.trim();
        let color = if let Some(name) = value.strip_prefix("$palette.") {
            match palette.get(name) {
                Some(value) => parse_theme_color(value),
                None => {
                    diagnostics.push(ThemeDiagnostic::warning(format!(
                        "theme token `{token}` references missing palette color `{name}`"
                    )));
                    None
                }
            }
        } else {
            parse_theme_color(value)
        };
        match color {
            Some(color) => {
                out.insert(token.clone(), color);
            }
            None => diagnostics.push(ThemeDiagnostic::warning(format!(
                "theme token `{token}` has invalid color `{value}`"
            ))),
        }
    }
    out
}

#[must_use]
pub fn builtin_theme_documents() -> Vec<ThemeDocument> {
    vec![
        system_theme_document(),
        oino_dark_theme_document(),
        oino_light_theme_document(),
        oino_mono_theme_document(),
        oino_aurora_theme_document(),
    ]
}

fn system_theme_document() -> ThemeDocument {
    ThemeDocument {
        schema_version: 1,
        id: "system".into(),
        display_name: "System".into(),
        description: Some(
            "Follow terminal preference when available; defaults to Oino Dark".into(),
        ),
        mode: ThemeMode::System,
        inherits: Some("oino-dark".into()),
        palette: BTreeMap::new(),
        tokens: BTreeMap::new(),
    }
}

fn oino_dark_theme_document() -> ThemeDocument {
    ThemeDocument {
        schema_version: 1,
        id: "oino-dark".into(),
        display_name: "Oino Dark".into(),
        description: Some("Oino default dark theme with explicit surfaces".into()),
        mode: ThemeMode::Dark,
        inherits: None,
        palette: BTreeMap::from([
            ("bg".into(), "#080f1d".into()),
            ("surface".into(), "#0f172a".into()),
            ("elevated".into(), "#172033".into()),
            ("text".into(), "default".into()),
            ("muted".into(), "dark_gray".into()),
            ("dim".into(), "dark_gray".into()),
            ("accent".into(), "cyan".into()),
            ("success".into(), "green".into()),
            ("warning".into(), "yellow".into()),
            ("error".into(), "red".into()),
            ("selection".into(), "#1d3557".into()),
            ("user".into(), "blue".into()),
            ("assistant".into(), "green".into()),
            ("tool".into(), "yellow".into()),
        ]),
        tokens: default_raw_theme_tokens(),
    }
}

fn oino_light_theme_document() -> ThemeDocument {
    let mut tokens = default_raw_theme_tokens();
    tokens.extend(BTreeMap::from([
        ("app.bg".into(), "$palette.bg".into()),
        ("app.fg".into(), "$palette.text".into()),
        ("panel.bg".into(), "$palette.surface".into()),
        ("panel.border".into(), "$palette.muted".into()),
        ("composer.bg".into(), "$palette.elevated".into()),
        ("list.selected_bg".into(), "$palette.selection".into()),
        ("message.user.border".into(), "$palette.user".into()),
        (
            "message.assistant.border".into(),
            "$palette.assistant".into(),
        ),
        ("markdown.fg".into(), "$palette.text".into()),
    ]));
    ThemeDocument {
        schema_version: 1,
        id: "oino-light".into(),
        display_name: "Oino Light".into(),
        description: Some("Light theme for bright terminals".into()),
        mode: ThemeMode::Light,
        inherits: None,
        palette: BTreeMap::from([
            ("bg".into(), "#f7fafc".into()),
            ("surface".into(), "#edf2f7".into()),
            ("elevated".into(), "#e2e8f0".into()),
            ("text".into(), "#111827".into()),
            ("muted".into(), "#64748b".into()),
            ("dim".into(), "#94a3b8".into()),
            ("accent".into(), "#2563eb".into()),
            ("success".into(), "#15803d".into()),
            ("warning".into(), "#b45309".into()),
            ("error".into(), "#b91c1c".into()),
            ("selection".into(), "#bfdbfe".into()),
            ("user".into(), "#2563eb".into()),
            ("assistant".into(), "#15803d".into()),
            ("tool".into(), "#a16207".into()),
        ]),
        tokens,
    }
}

fn oino_mono_theme_document() -> ThemeDocument {
    let mut tokens = default_raw_theme_tokens();
    tokens.extend(BTreeMap::from([
        ("app.bg".into(), "$palette.bg".into()),
        ("panel.bg".into(), "$palette.surface".into()),
        ("panel.border_focused".into(), "$palette.text".into()),
        ("app.border_focused".into(), "$palette.text".into()),
        ("composer.border_focused".into(), "$palette.text".into()),
        ("message.user.border".into(), "$palette.muted".into()),
        ("message.assistant.border".into(), "$palette.text".into()),
        ("tool.running".into(), "$palette.text".into()),
        ("status.working".into(), "$palette.text".into()),
    ]));
    ThemeDocument {
        schema_version: 1,
        id: "oino-mono".into(),
        display_name: "Oino Mono".into(),
        description: Some("Low-color grayscale theme".into()),
        mode: ThemeMode::Mono,
        inherits: None,
        palette: BTreeMap::from([
            ("bg".into(), "#0a0a0a".into()),
            ("surface".into(), "#121212".into()),
            ("elevated".into(), "#1f1f1f".into()),
            ("text".into(), "#eeeeee".into()),
            ("muted".into(), "#a3a3a3".into()),
            ("dim".into(), "#737373".into()),
            ("accent".into(), "#d4d4d4".into()),
            ("success".into(), "#d4d4d4".into()),
            ("warning".into(), "#e5e5e5".into()),
            ("error".into(), "#f5f5f5".into()),
            ("selection".into(), "#3f3f46".into()),
            ("user".into(), "#d4d4d4".into()),
            ("assistant".into(), "#eeeeee".into()),
            ("tool".into(), "#a3a3a3".into()),
        ]),
        tokens,
    }
}

fn oino_aurora_theme_document() -> ThemeDocument {
    let mut tokens = default_raw_theme_tokens();
    tokens.extend(BTreeMap::from([
        ("app.bg".into(), "$palette.bg".into()),
        ("panel.bg".into(), "$palette.surface".into()),
        ("panel.border".into(), "$palette.border".into()),
        ("panel.border_focused".into(), "$palette.accent".into()),
        ("composer.bg".into(), "$palette.elevated".into()),
        ("composer.border_focused".into(), "$palette.accent".into()),
        ("list.selected_bg".into(), "$palette.selection".into()),
        ("status.working".into(), "$palette.warning".into()),
        ("message.user.border".into(), "$palette.user".into()),
        (
            "message.assistant.border".into(),
            "$palette.assistant".into(),
        ),
        ("tool.running".into(), "$palette.accent".into()),
        (
            "extension_surface.tab_active".into(),
            "$palette.accent".into(),
        ),
    ]));
    ThemeDocument {
        schema_version: 1,
        id: "oino-aurora".into(),
        display_name: "Oino Aurora".into(),
        description: Some("Blue-green dark theme with visible Oino surfaces".into()),
        mode: ThemeMode::Dark,
        inherits: None,
        palette: BTreeMap::from([
            ("bg".into(), "#08111f".into()),
            ("surface".into(), "#0f1b2d".into()),
            ("elevated".into(), "#17263c".into()),
            ("border".into(), "#2b4c6f".into()),
            ("text".into(), "#e6eef8".into()),
            ("muted".into(), "#91a4b8".into()),
            ("dim".into(), "#5e7083".into()),
            ("accent".into(), "#7dd3fc".into()),
            ("success".into(), "#86efac".into()),
            ("warning".into(), "#f6c177".into()),
            ("error".into(), "#f38ba8".into()),
            ("selection".into(), "#213a5a".into()),
            ("user".into(), "#93c5fd".into()),
            ("assistant".into(), "#86efac".into()),
            ("tool".into(), "#f6c177".into()),
        ]),
        tokens,
    }
}

fn default_palette() -> BTreeMap<String, String> {
    oino_dark_theme_document().palette
}

fn default_raw_theme_tokens() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("app.bg".into(), "$palette.bg".into()),
        ("app.fg".into(), "$palette.text".into()),
        ("app.border".into(), "$palette.muted".into()),
        ("app.border_focused".into(), "$palette.accent".into()),
        ("app.title".into(), "$palette.accent".into()),
        ("panel.bg".into(), "$palette.surface".into()),
        ("panel.fg".into(), "$palette.text".into()),
        ("panel.border".into(), "$palette.muted".into()),
        ("panel.border_focused".into(), "$palette.accent".into()),
        ("panel.title".into(), "$palette.accent".into()),
        ("list.fg".into(), "$palette.text".into()),
        ("list.muted".into(), "$palette.muted".into()),
        ("list.selected_fg".into(), "$palette.text".into()),
        ("list.selected_bg".into(), "$palette.selection".into()),
        ("composer.bg".into(), "$palette.elevated".into()),
        ("composer.fg".into(), "$palette.text".into()),
        ("composer.placeholder".into(), "$palette.dim".into()),
        ("composer.border".into(), "$palette.muted".into()),
        ("composer.border_focused".into(), "$palette.accent".into()),
        ("status.bg".into(), "$palette.bg".into()),
        ("status.fg".into(), "$palette.muted".into()),
        ("status.muted".into(), "$palette.muted".into()),
        ("status.working".into(), "$palette.warning".into()),
        ("status.success".into(), "$palette.success".into()),
        ("status.warning".into(), "$palette.warning".into()),
        ("status.error".into(), "$palette.error".into()),
        ("message.user.fg".into(), "$palette.text".into()),
        ("message.user.border".into(), "$palette.user".into()),
        ("message.assistant.fg".into(), "$palette.text".into()),
        (
            "message.assistant.border".into(),
            "$palette.assistant".into(),
        ),
        ("message.error.fg".into(), "$palette.error".into()),
        ("tool.title".into(), "$palette.tool".into()),
        ("tool.fg".into(), "$palette.text".into()),
        ("tool.muted".into(), "$palette.muted".into()),
        ("tool.border".into(), "$palette.tool".into()),
        ("tool.running".into(), "$palette.warning".into()),
        ("tool.success".into(), "$palette.success".into()),
        ("tool.error".into(), "$palette.error".into()),
        ("markdown.fg".into(), "$palette.text".into()),
        ("markdown.heading".into(), "$palette.tool".into()),
        ("markdown.link".into(), "$palette.accent".into()),
        ("markdown.code_border".into(), "$palette.accent".into()),
        ("extension_surface.bg".into(), "$palette.surface".into()),
        ("extension_surface.fg".into(), "$palette.text".into()),
        ("extension_surface.border".into(), "$palette.muted".into()),
        (
            "extension_surface.focused_border".into(),
            "$palette.accent".into(),
        ),
    ])
}

#[must_use]
pub fn normalize_theme_id(value: &str) -> Option<String> {
    let raw = value.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    let mut normalized = String::new();
    let mut previous_dash = false;
    for ch in raw.chars() {
        let next = if ch.is_ascii_alphanumeric() || ch == '.' {
            previous_dash = false;
            ch
        } else if matches!(ch, '-' | '_' | ' ') {
            if previous_dash {
                continue;
            }
            previous_dash = true;
            '-'
        } else {
            continue;
        };
        normalized.push(next);
    }
    let normalized = normalized.trim_matches('-');
    let alias = match normalized {
        "auto" | "default" | "system" => "system",
        "dark" | "oino" | "oino-dark" => "oino-dark",
        "light" | "oino-light" => "oino-light",
        "mono" | "monochrome" | "grayscale" | "greyscale" | "gray" | "grey" | "oino-mono" => {
            "oino-mono"
        }
        other => other,
    };
    (!alias.is_empty()).then(|| alias.to_string())
}

#[must_use]
pub fn normalize_theme_token(token: &str) -> String {
    let mut normalized = String::new();
    for (index, ch) in token.trim().chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | ' ') {
            normalized.push('_');
        } else {
            normalized.push(ch);
        }
    }
    normalize_theme_token_alias(&normalized).to_string()
}

fn normalize_theme_token_alias(token: &str) -> &str {
    match token {
        "accent" => "app.border_focused",
        "success" => "status.success",
        "text" | "fg" => "app.fg",
        "muted" => "status.muted",
        "dim" => "panel.dim",
        "focused_border" | "border_accent" | "panel.focused_border" => "panel.border_focused",
        "app.focused_border" => "app.border_focused",
        "composer.focused_border" => "composer.border_focused",
        "extension_surface.focused_border" => "extension_surface.focused_border",
        "panel_border" | "border" | "border_muted" => "panel.border",
        "user_border" => "message.user.border",
        "user_message_text" => "message.user.fg",
        "assistant_border" => "message.assistant.border",
        "assistant_message_text" => "message.assistant.fg",
        "tool_border" => "tool.border",
        "tool_title" => "tool.title",
        "title" => "panel.title",
        "warning" => "status.warning",
        "error" => "status.error",
        "footer" | "status" | "inline_status" => "status.fg",
        "working" | "working_indicator" => "status.working",
        other => other,
    }
}

#[must_use]
pub fn parse_theme_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("default")
        || value.eq_ignore_ascii_case("reset")
    {
        return Some(Color::Reset);
    }
    if let Some(color) = hex_theme_color(value) {
        return Some(color);
    }
    if let Ok(index) = value.parse::<u8>() {
        return Some(Color::Indexed(index));
    }
    match value.to_ascii_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "dark_gray" | "dark-grey" | "darkgray" => Some(Color::DarkGray),
        "light_red" | "light-red" => Some(Color::LightRed),
        "light_green" | "light-green" => Some(Color::LightGreen),
        "light_yellow" | "light-yellow" => Some(Color::LightYellow),
        "light_blue" | "light-blue" => Some(Color::LightBlue),
        "light_magenta" | "light-magenta" => Some(Color::LightMagenta),
        "light_cyan" | "light-cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

fn hex_theme_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn apply_color_token(theme: &mut Theme, token: &str, color: Color) {
    match token {
        "app.bg" => theme.bg = color,
        "app.fg" | "panel.fg" | "markdown.fg" | "message.assistant.fg" => theme.fg = color,
        "app.border" | "panel.border" => theme.panel_border = color,
        "app.border_focused" | "panel.border_focused" | "composer.border_focused" => {
            theme.focused_border = color;
            theme.accent = color;
        }
        "app.title" | "panel.title" => theme.title = theme.title.fg(color),
        "panel.bg" | "extension_surface.bg" => theme.panel_bg = color,
        "composer.bg" => theme.composer_bg = color,
        "composer.placeholder" | "panel.dim" => {
            theme.dim = color;
            theme.placeholder = theme.placeholder.fg(color);
        }
        "list.selected_bg" => theme.selection_bg = color,
        "list.selected_fg" => theme.selected_fg = color,
        "status.fg" | "status.muted" => {
            theme.muted = color;
            theme.footer = theme.footer.fg(color);
        }
        "status.working" | "tool.running" => theme.working = theme.working.fg(color),
        "status.success" | "tool.success" => theme.success = color,
        "status.warning" => theme.warning = theme.warning.fg(color),
        "status.error" | "message.error.fg" | "tool.error" => theme.error = theme.error.fg(color),
        "message.user.border" => theme.user_border = color,
        "message.assistant.border" => theme.assistant_border = color,
        "tool.border" | "tool.title" | "markdown.heading" => theme.tool_border = color,
        "markdown.link" | "markdown.code_border" | "extension_surface.focused_border" => {
            theme.focused_border = color
        }
        "extension_surface.border" => theme.panel_border = color,
        _ => {}
    }
}

fn is_known_theme_token(token: &str) -> bool {
    matches!(
        token,
        "app.bg"
            | "app.fg"
            | "app.border"
            | "app.border_focused"
            | "app.title"
            | "app.warning"
            | "app.error"
            | "app.tiny_terminal"
            | "panel.bg"
            | "panel.fg"
            | "panel.border"
            | "panel.border_focused"
            | "panel.title"
            | "panel.footer"
            | "panel.dim"
            | "list.fg"
            | "list.muted"
            | "list.separator"
            | "list.cursor"
            | "list.selected_fg"
            | "list.selected_bg"
            | "list.badge"
            | "list.badge_bg"
            | "composer.bg"
            | "composer.fg"
            | "composer.placeholder"
            | "composer.cursor"
            | "composer.border"
            | "composer.border_focused"
            | "composer.reference"
            | "composer.collapsed_paste"
            | "suggestion.bg"
            | "suggestion.fg"
            | "suggestion.match"
            | "suggestion.category"
            | "suggestion.border"
            | "suggestion.selected_fg"
            | "suggestion.selected_bg"
            | "status.bg"
            | "status.fg"
            | "status.muted"
            | "status.working"
            | "status.success"
            | "status.warning"
            | "status.error"
            | "status.extension"
            | "message.user.fg"
            | "message.user.bg"
            | "message.user.border"
            | "message.assistant.fg"
            | "message.assistant.bg"
            | "message.assistant.border"
            | "message.system.fg"
            | "message.system.bg"
            | "message.system.border"
            | "message.error.fg"
            | "message.error.bg"
            | "message.error.border"
            | "message.title"
            | "message.muted"
            | "tool.title"
            | "tool.fg"
            | "tool.muted"
            | "tool.border"
            | "tool.bg"
            | "tool.running"
            | "tool.success"
            | "tool.error"
            | "tool.output"
            | "tool.diff_added"
            | "tool.diff_removed"
            | "tool.diff_context"
            | "thinking.fg"
            | "thinking.muted"
            | "thinking.bg"
            | "thinking.border"
            | "thinking.live"
            | "thinking.collapsed"
            | "resource.title"
            | "resource.fg"
            | "resource.muted"
            | "resource.bg"
            | "resource.border"
            | "resource.badge"
            | "markdown.fg"
            | "markdown.heading"
            | "markdown.heading_secondary"
            | "markdown.link"
            | "markdown.link_url"
            | "markdown.marker"
            | "markdown.muted"
            | "markdown.quote"
            | "markdown.quote_border"
            | "markdown.list_marker"
            | "markdown.table_border"
            | "markdown.code_bg"
            | "markdown.code_border"
            | "markdown.code_line_number"
            | "syntax.comment"
            | "syntax.keyword"
            | "syntax.function"
            | "syntax.variable"
            | "syntax.string"
            | "syntax.number"
            | "syntax.type"
            | "syntax.operator"
            | "syntax.punctuation"
            | "settings.title"
            | "settings.fg"
            | "settings.muted"
            | "settings.active"
            | "settings.changed"
            | "settings.warning"
            | "settings.danger"
            | "extension.package"
            | "extension.runtime"
            | "extension.contribution"
            | "extension.enabled"
            | "extension.disabled"
            | "extension.conflict"
            | "extension.diagnostic"
            | "extension.override"
            | "extension_surface.bg"
            | "extension_surface.fg"
            | "extension_surface.border"
            | "extension_surface.focused_border"
            | "extension_surface.title"
            | "extension_surface.tab_active"
            | "extension_surface.tab_inactive"
            | "extension_surface.conflict"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_document_and_colors() {
        let document = ThemeDocument::from_json_str(
            r##"{
                "schema_version": 1,
                "id": "Team Theme",
                "display_name": "Team Theme",
                "mode": "dark",
                "palette": { "accent": "#7dd3fc" },
                "tokens": { "panel.focusedBorder": "$palette.accent", "status.error": "red" }
            }"##,
        )
        .unwrap_or_else(|err| panic!("theme parse failed: {err}"));
        assert_eq!(document.normalized_id().as_deref(), Some("team-theme"));
        assert!(document.validate().is_empty());
        assert_eq!(
            parse_theme_color("#7dd3fc"),
            Some(Color::Rgb(0x7d, 0xd3, 0xfc))
        );
        assert_eq!(parse_theme_color("242"), Some(Color::Indexed(242)));
    }

    #[test]
    fn resolves_project_theme_over_global_theme() {
        let catalog = ThemeCatalog::builtins();
        let mut global = ThemeSettings::default();
        global.set_active("oino-light");
        let mut project = ThemeSettings::default();
        project.set_active("oino-aurora");
        project
            .overrides
            .insert("panel.borderFocused".into(), "#ff00aa".into());

        let resolved = resolve_effective_theme(&catalog, &global, &project);
        assert_eq!(resolved.id, "oino-aurora");
        assert_eq!(resolved.selected_scope, EffectiveThemeScope::Project);
        assert_eq!(
            resolved.tokens.get("panel.border_focused"),
            Some(&Color::Rgb(0xff, 0x00, 0xaa))
        );
        assert_ne!(resolved.mode, ThemeMode::Light);
    }

    #[test]
    fn resolves_source_precedence_for_duplicate_theme_ids() {
        let mut catalog = ThemeCatalog::builtins();
        let mut global_doc = oino_dark_theme_document();
        global_doc.id = "team".into();
        global_doc.display_name = "Global Team".into();
        let mut project_doc = global_doc.clone();
        project_doc.display_name = "Project Team".into();
        project_doc
            .tokens
            .insert("app.title".into(), "#abcdef".into());
        catalog.register(ThemeCatalogEntry::new(
            ThemeSource {
                kind: ThemeSourceKind::File,
                scope: ThemeSourceScope::Global,
            },
            global_doc,
        ));
        catalog.register(ThemeCatalogEntry::new(
            ThemeSource {
                kind: ThemeSourceKind::File,
                scope: ThemeSourceScope::Project,
            },
            project_doc,
        ));
        let mut global = ThemeSettings::default();
        global.set_active("team");
        let resolved = resolve_effective_theme(&catalog, &global, &ThemeSettings::default());
        assert_eq!(resolved.display_name, "Project Team");
        assert_eq!(resolved.source.scope, ThemeSourceScope::Project);
        assert_eq!(
            resolved.tokens.get("app.title"),
            Some(&Color::Rgb(0xab, 0xcd, 0xef))
        );
    }

    #[test]
    fn missing_theme_falls_back_to_system() {
        let catalog = ThemeCatalog::builtins();
        let mut project = ThemeSettings::default();
        project.set_active("missing-theme");
        let resolved = resolve_effective_theme(&catalog, &ThemeSettings::default(), &project);
        assert_eq!(resolved.id, "system");
        assert!(resolved
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing-theme")));
    }

    #[test]
    fn resolved_theme_converts_to_legacy_tui_theme_boundary() {
        let catalog = ThemeCatalog::builtins();
        let mut project = ThemeSettings::default();
        project.set_active("oino-aurora");
        let resolved = resolve_effective_theme(&catalog, &ThemeSettings::default(), &project);
        let theme = Theme::from_resolved_theme(&resolved);
        assert_eq!(theme.bg, Color::Rgb(0x08, 0x11, 0x1f));
        assert_eq!(theme.panel_bg, Color::Rgb(0x0f, 0x1b, 0x2d));
        assert_eq!(theme.focused_border, Color::Rgb(0x7d, 0xd3, 0xfc));
        assert_eq!(theme.selection_bg, Color::Rgb(0x21, 0x3a, 0x5a));
    }
}
