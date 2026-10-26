//! Selectors that replace the editor area (pi `showModelSelector`, `showSessionSelector`,
//! `showTreeSelector`, `showUserMessageSelector`, `showSettingsSelector`, thinking and
//! scoped-model selectors).

use oino_agent::QueueMode;
use oino_ai::{JsonValue, Model, ThinkingLevel};
use oino_client::protocol::{OinoCommand, PiCommand};
use oino_core::session::NavigateTreeOptions;
use oino_view::composer::Composer;
use oino_view::fuzzy;
use oino_view::keys::{Key, KeyId};
use oino_view::sessions::{Scope, SessionListModel};
use oino_view::transcript::NoticeLevel;
use oino_view::tree::{FilterMode, TreeModel};

use super::submit::level_name;
use super::{App, Effect, ModelsFor, Tag};

/// Model selector state.
#[derive(Clone, Debug, Default)]
pub(crate) struct ModelPicker {
    pub(crate) query: String,
    pub(crate) filtered: Vec<usize>,
    pub(crate) selected: usize,
}

/// Scoped models selector state (`enabledModels`).
#[derive(Clone, Debug, Default)]
pub(crate) struct ScopedPicker {
    pub(crate) query: String,
    pub(crate) filtered: Vec<usize>,
    pub(crate) selected: usize,
    /// Enabled `provider/id` patterns in cycle order.
    pub(crate) enabled: Vec<String>,
    pub(crate) dirty: bool,
}

/// Session picker state.
#[derive(Clone, Debug)]
pub(crate) struct SessionPicker {
    pub(crate) model: SessionListModel,
    pub(crate) status: Option<String>,
    pub(crate) rename: Option<Composer>,
}

/// What the tree overlay is asking.
#[derive(Clone, Debug)]
pub(crate) enum TreePrompt {
    /// Editing a label.
    Label(String, Composer),
    /// Branch summary choice for navigating to an entry.
    Summary {
        /// Target entry.
        target: String,
        /// Selected option (0 no summary, 1 summarize, 2 custom prompt).
        selected: usize,
    },
    /// Custom summary instructions.
    Instructions(String, Composer),
}

/// Tree navigator state.
#[derive(Clone, Debug)]
pub(crate) struct TreeOverlay {
    pub(crate) model: TreeModel,
    pub(crate) prompt: Option<TreePrompt>,
}

/// Fork picker state.
#[derive(Clone, Debug, Default)]
pub(crate) struct ForkPicker {
    pub(crate) items: Vec<(String, String)>,
    pub(crate) query: String,
    pub(crate) filtered: Vec<usize>,
    pub(crate) selected: usize,
}

/// Value kind of a settings row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingKind {
    /// On/off.
    Bool(bool),
    /// One of the options.
    Choice(Vec<&'static str>, String),
}

/// One `/settings` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingItem {
    pub(crate) label: &'static str,
    pub(crate) description: &'static str,
    pub(crate) path: &'static [&'static str],
    pub(crate) kind: SettingKind,
}

/// Settings screen state.
#[derive(Clone, Debug, Default)]
pub(crate) struct SettingsOverlay {
    pub(crate) items: Vec<SettingItem>,
    /// Extension namespaces listed after the core rows (the settings hub).
    pub(crate) namespaces: Vec<oino_view::settings_hub::HubNamespace>,
    pub(crate) selected: usize,
    pub(crate) status: Option<String>,
}

/// An open selector.
#[derive(Clone, Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "at most one selector exists at a time"
)]
pub(crate) enum Overlay {
    /// `/model`.
    Models(ModelPicker),
    /// `/scoped-models`.
    Scoped(ScopedPicker),
    /// `/thinking`.
    Thinking {
        /// Selected index.
        selected: usize,
    },
    /// `/resume`.
    Sessions(SessionPicker),
    /// `/tree`.
    Tree(TreeOverlay),
    /// `/fork`.
    Fork(ForkPicker),
    /// `/settings`.
    Settings(SettingsOverlay),
    /// One extension namespace of the settings hub.
    ExtSettings(super::ExtSettingsPage),
    /// `/login` / `/logout` provider picker.
    Providers(super::ProviderPicker),
}

pub(crate) fn model_label(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}

fn wrap_index(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len_i = isize::try_from(len).unwrap_or(isize::MAX);
    let next = (isize::try_from(current).unwrap_or(0) + delta).rem_euclid(len_i);
    usize::try_from(next).unwrap_or(0)
}

fn edit_query(query: &mut String, key: &KeyId) -> bool {
    if key.key == Key::Backspace && !key.mods.ctrl && !key.mods.alt {
        query.pop();
        return true;
    }
    if let Some(c) = key.printable() {
        query.push(c);
        return true;
    }
    false
}

#[allow(
    clippy::too_many_lines,
    reason = "declarative table of pi settings rows"
)]
fn settings_items(settings: &oino_ai::JsonObject) -> Vec<SettingItem> {
    let get = |path: &[&str]| -> Option<&JsonValue> {
        let mut value = settings.get(path[0])?;
        for key in &path[1..] {
            value = value.get(key)?;
        }
        Some(value)
    };
    let boolean = |path: &[&str], default: bool| {
        SettingKind::Bool(get(path).and_then(JsonValue::as_bool).unwrap_or(default))
    };
    let choice = |path: &[&str], options: Vec<&'static str>, default: &str| {
        let value = get(path)
            .and_then(JsonValue::as_str)
            .unwrap_or(default)
            .to_owned();
        SettingKind::Choice(options, value)
    };
    vec![
        SettingItem {
            label: "Auto-compact",
            description: "Automatically compact context when it gets too large",
            path: &["compaction", "enabled"],
            kind: boolean(&["compaction", "enabled"], true),
        },
        SettingItem {
            label: "Auto-retry",
            description: "Retry transient provider errors automatically",
            path: &["retry", "enabled"],
            kind: boolean(&["retry", "enabled"], true),
        },
        SettingItem {
            label: "Steering mode",
            description: "Enter while streaming queues steering messages",
            path: &["steeringMode"],
            kind: choice(
                &["steeringMode"],
                vec!["one-at-a-time", "all"],
                "one-at-a-time",
            ),
        },
        SettingItem {
            label: "Follow-up mode",
            description: "How queued follow-up messages are delivered",
            path: &["followUpMode"],
            kind: choice(
                &["followUpMode"],
                vec!["one-at-a-time", "all"],
                "one-at-a-time",
            ),
        },
        SettingItem {
            label: "Thinking level",
            description: "Default reasoning depth for thinking-capable models",
            path: &["defaultThinkingLevel"],
            kind: choice(
                &["defaultThinkingLevel"],
                vec!["off", "minimal", "low", "medium", "high", "xhigh", "max"],
                "medium",
            ),
        },
        SettingItem {
            label: "Hide thinking",
            description: "Collapse thinking blocks in the transcript",
            path: &["hideThinkingBlock"],
            kind: boolean(&["hideThinkingBlock"], false),
        },
        SettingItem {
            label: "Theme",
            description: "Colour theme",
            path: &["theme"],
            kind: choice(&["theme"], vec!["dark", "light"], "dark"),
        },
        SettingItem {
            label: "Show images",
            description: "Render images inline when the terminal supports it",
            path: &["terminal", "showImages"],
            kind: boolean(&["terminal", "showImages"], true),
        },
        SettingItem {
            label: "Auto-resize images",
            description: "Resize large images before sending",
            path: &["images", "autoResize"],
            kind: boolean(&["images", "autoResize"], true),
        },
        SettingItem {
            label: "Block images",
            description: "Never send images to the model",
            path: &["images", "blockImages"],
            kind: boolean(&["images", "blockImages"], false),
        },
        SettingItem {
            label: "Skill commands",
            description: "Expose skills as /skill:name commands",
            path: &["enableSkillCommands"],
            kind: boolean(&["enableSkillCommands"], true),
        },
        SettingItem {
            label: "Double-escape action",
            description: "What pressing Escape twice on an empty editor opens",
            path: &["doubleEscapeAction"],
            kind: choice(
                &["doubleEscapeAction"],
                vec!["tree", "fork", "none"],
                "tree",
            ),
        },
        SettingItem {
            label: "Tree filter",
            description: "Default filter of the /tree navigator",
            path: &["treeFilterMode"],
            kind: choice(
                &["treeFilterMode"],
                vec!["default", "no-tools", "user-only", "labeled-only", "all"],
                "default",
            ),
        },
        SettingItem {
            label: "Quiet startup",
            description: "Hide the startup header",
            path: &["quietStartup"],
            kind: boolean(&["quietStartup"], false),
        },
        SettingItem {
            label: "TUI mode",
            description: "regular keeps the transcript in the terminal scrollback (next start)",
            path: &["tuiMode"],
            kind: choice(&["tuiMode"], vec!["fullscreen", "regular"], "fullscreen"),
        },
        SettingItem {
            label: "Cache-miss notices",
            description: "Show prompt cache misses and billed summaries",
            path: &["showCacheMissNotices"],
            kind: boolean(&["showCacheMissNotices"], true),
        },
    ]
}

impl App {
    pub(crate) fn on_models(&mut self, purpose: ModelsFor) {
        match purpose {
            ModelsFor::Footer => {}
            ModelsFor::Selector(query) => {
                let mut picker = ModelPicker {
                    query: query.unwrap_or_default(),
                    ..ModelPicker::default()
                };
                self.filter_models(&mut picker);
                if let Some(current) = &self.session.model
                    && let Some(pos) = picker.filtered.iter().position(|&i| {
                        self.models[i].id == current.id
                            && self.models[i].provider == current.provider
                    })
                {
                    picker.selected = pos;
                }
                self.overlay = Some(Overlay::Models(picker));
            }
            ModelsFor::Direct(query) => {
                let exact = self
                    .models
                    .iter()
                    .find(|m| model_label(m) == query || m.id == query);
                if let Some(model) = exact {
                    let (provider, model_id) = (model.provider.clone(), model.id.clone());
                    self.send(Tag::SetModel, PiCommand::SetModel { provider, model_id });
                } else {
                    self.on_models(ModelsFor::Selector(Some(query)));
                }
            }
            ModelsFor::Scoped => {
                let enabled = self
                    .settings
                    .get("enabledModels")
                    .and_then(JsonValue::as_array)
                    .map(|v| {
                        v.iter()
                            .filter_map(|p| p.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut picker = ScopedPicker {
                    enabled,
                    ..ScopedPicker::default()
                };
                picker.filtered = (0..self.models.len()).collect();
                self.overlay = Some(Overlay::Scoped(picker));
            }
            ModelsFor::CycleBackward => {
                if self.models.len() < 2 {
                    self.notice(NoticeLevel::Info, "Only one model available");
                    return;
                }
                let current = self.session.model.as_ref().and_then(|c| {
                    self.models
                        .iter()
                        .position(|m| m.id == c.id && m.provider == c.provider)
                });
                let index = current.map_or(0, |i| wrap_index(i, -1, self.models.len()));
                let model = &self.models[index];
                let (provider, model_id) = (model.provider.clone(), model.id.clone());
                self.send(Tag::SetModel, PiCommand::SetModel { provider, model_id });
            }
        }
    }

    fn filter_models(&self, picker: &mut ModelPicker) {
        picker.filtered = fuzzy::filter(&self.models, &picker.query, |m| {
            format!("{} {}", model_label(m), m.name)
        });
        picker.selected = picker.selected.min(picker.filtered.len().saturating_sub(1));
    }

    pub(crate) fn open_thinking_selector(&mut self) {
        if self.thinking_levels.is_empty() {
            self.notice(NoticeLevel::Info, "Current model does not support thinking");
            return;
        }
        let selected = self
            .thinking_levels
            .iter()
            .position(|l| *l == self.session.thinking)
            .unwrap_or(0);
        self.overlay = Some(Overlay::Thinking { selected });
    }

    pub(crate) fn open_sessions(&mut self) {
        self.overlay = Some(Overlay::Sessions(SessionPicker {
            model: SessionListModel::new(Scope::Current, self.session.session_file.clone()),
            status: None,
            rename: None,
        }));
        self.effects.push(Effect::ListSessions(Scope::Current));
    }

    pub(crate) fn open_tree(&mut self, data: Option<&JsonValue>) {
        let Some(data) = data else {
            return;
        };
        let home = self.host.home().map(|h| h.display().to_string());
        let mut model = TreeModel::from_json(data, home);
        if let Some(mode) = self
            .settings
            .get("treeFilterMode")
            .and_then(JsonValue::as_str)
        {
            model.set_filter(FilterMode::parse(mode));
        }
        if model.is_empty() && model.filter == FilterMode::Default {
            self.notice(NoticeLevel::Info, "No entries in session");
            return;
        }
        self.overlay = Some(Overlay::Tree(TreeOverlay {
            model,
            prompt: None,
        }));
    }

    pub(crate) fn open_fork_picker(&mut self, items: Vec<(String, String)>) {
        if items.is_empty() {
            self.notice(NoticeLevel::Info, "No messages to fork from");
            return;
        }
        let selected = items.len() - 1;
        self.overlay = Some(Overlay::Fork(ForkPicker {
            filtered: (0..items.len()).collect(),
            items,
            query: String::new(),
            selected,
        }));
    }

    /// `/settings`: core rows plus the extension namespaces (the hub); a `query` naming a
    /// namespace or extension opens that page.
    pub(crate) fn open_settings(&mut self, query: Option<String>) {
        if let Some(query) = query.filter(|q| !q.is_empty()) {
            self.request_settings_schema(super::SchemaPurpose::Namespace(query));
            return;
        }
        self.settings = self.host.settings();
        let overlay = SettingsOverlay {
            items: settings_items(&self.settings),
            namespaces: oino_view::settings_hub::namespaces(&self.settings_schema),
            ..SettingsOverlay::default()
        };
        self.overlay = Some(Overlay::Settings(overlay));
        self.request_settings_schema(super::SchemaPurpose::Hub);
    }

    /// Key handling while a selector is open.
    #[allow(
        clippy::too_many_lines,
        reason = "one block per selector, mirroring pi component handleInput"
    )]
    pub(crate) fn overlay_key(&mut self, key: KeyId) {
        let Some(mut overlay) = self.overlay.take() else {
            return;
        };
        let up = self.keys.matches("tui.select.up", &key);
        let down = self.keys.matches("tui.select.down", &key);
        let confirm = self.keys.matches("tui.select.confirm", &key);
        let cancel = self.keys.matches("tui.select.cancel", &key);
        let keep = match &mut overlay {
            Overlay::Models(picker) => {
                if cancel {
                    false
                } else if up || down {
                    picker.selected = wrap_index(
                        picker.selected,
                        if up { -1 } else { 1 },
                        picker.filtered.len(),
                    );
                    true
                } else if confirm {
                    if let Some(&i) = picker.filtered.get(picker.selected) {
                        let model = &self.models[i];
                        let (provider, model_id) = (model.provider.clone(), model.id.clone());
                        self.send(Tag::SetModel, PiCommand::SetModel { provider, model_id });
                    }
                    false
                } else {
                    if edit_query(&mut picker.query, &key) {
                        picker.selected = 0;
                        self.filter_models(picker);
                    }
                    true
                }
            }
            Overlay::Scoped(picker) => self.scoped_key(picker, &key, up, down, confirm, cancel),
            Overlay::Thinking { selected } => {
                if cancel {
                    false
                } else if up || down {
                    *selected = wrap_index(
                        *selected,
                        if up { -1 } else { 1 },
                        self.thinking_levels.len(),
                    );
                    true
                } else if confirm {
                    if let Some(&level) = self.thinking_levels.get(*selected) {
                        self.send(Tag::Ignore, PiCommand::SetThinkingLevel { level });
                        self.session.thinking = level;
                    }
                    false
                } else {
                    true
                }
            }
            Overlay::Sessions(picker) => self.sessions_key(picker, &key, up, down, confirm, cancel),
            Overlay::Tree(tree) => self.tree_key(tree, &key, up, down, confirm, cancel),
            Overlay::Fork(picker) => {
                if cancel {
                    false
                } else if up || down {
                    picker.selected = wrap_index(
                        picker.selected,
                        if up { -1 } else { 1 },
                        picker.filtered.len(),
                    );
                    true
                } else if confirm {
                    if let Some(&i) = picker.filtered.get(picker.selected) {
                        let entry_id = picker.items[i].0.clone();
                        self.send(Tag::Fork, PiCommand::Fork { entry_id });
                    }
                    false
                } else {
                    if edit_query(&mut picker.query, &key) {
                        picker.filtered =
                            fuzzy::filter(&picker.items, &picker.query, |(_, t)| t.clone());
                        picker.selected = picker.filtered.len().saturating_sub(1);
                    }
                    true
                }
            }
            Overlay::ExtSettings(page) => self.ext_settings_key(page, &key),
            Overlay::Providers(picker) => self.provider_key(picker, &key),
            Overlay::Settings(settings) => {
                if cancel {
                    false
                } else if up || down {
                    settings.selected = wrap_index(
                        settings.selected,
                        if up { -1 } else { 1 },
                        settings.items.len() + settings.namespaces.len(),
                    );
                    true
                } else if let Some(namespace) = settings
                    .selected
                    .checked_sub(settings.items.len())
                    .and_then(|i| settings.namespaces.get(i))
                    .filter(|_| confirm || key.key == Key::Right)
                {
                    self.open_namespace(namespace.clone());
                    false
                } else if confirm
                    || key.key == Key::Space
                    || key.key == Key::Right
                    || key.key == Key::Left
                {
                    let back = key.key == Key::Left;
                    let index = settings.selected;
                    if let Some(item) = settings.items.get_mut(index) {
                        let value = match &mut item.kind {
                            SettingKind::Bool(b) => {
                                *b = !*b;
                                JsonValue::Bool(*b)
                            }
                            SettingKind::Choice(options, current) => {
                                let pos = options.iter().position(|o| o == current).unwrap_or(0);
                                let next =
                                    wrap_index(pos, if back { -1 } else { 1 }, options.len());
                                options[next].clone_into(current);
                                JsonValue::String(current.clone())
                            }
                        };
                        let item = item.clone();
                        settings.status = self.apply_setting(&item, value).err();
                    }
                    true
                } else {
                    true
                }
            }
        };
        if keep && self.overlay.is_none() {
            self.overlay = Some(overlay);
        }
    }

    fn apply_setting(&mut self, item: &SettingItem, value: JsonValue) -> Result<(), String> {
        match (item.path, &value) {
            (["compaction", "enabled"], JsonValue::Bool(enabled)) => {
                self.send(
                    Tag::Ignore,
                    PiCommand::SetAutoCompaction { enabled: *enabled },
                );
                self.session.auto_compaction = *enabled;
            }
            (["retry", "enabled"], JsonValue::Bool(enabled)) => {
                self.send(Tag::Ignore, PiCommand::SetAutoRetry { enabled: *enabled });
            }
            (["steeringMode" | "followUpMode"], JsonValue::String(mode)) => {
                let mode = if mode == "all" {
                    QueueMode::All
                } else {
                    QueueMode::OneAtATime
                };
                if item.path[0] == "steeringMode" {
                    self.send(Tag::Ignore, PiCommand::SetSteeringMode { mode });
                } else {
                    self.send(Tag::Ignore, PiCommand::SetFollowUpMode { mode });
                }
            }
            (["hideThinkingBlock"], JsonValue::Bool(hide)) => {
                self.hide_thinking = *hide;
                self.cache.clear();
            }
            (["theme"], JsonValue::String(name)) => {
                self.theme = crate::theme::Theme::named(name);
                self.cache.clear();
            }
            _ => {}
        }
        self.host.set_setting(item.path, Some(value))?;
        self.settings = self.host.settings();
        Ok(())
    }

    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "pre-resolved select actions shared by every selector"
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "one branch per pi scoped-models binding"
    )]
    fn scoped_key(
        &mut self,
        picker: &mut ScopedPicker,
        key: &KeyId,
        up: bool,
        down: bool,
        confirm: bool,
        cancel: bool,
    ) -> bool {
        let label_at = |app: &Self, picker: &ScopedPicker, pos: usize| {
            picker
                .filtered
                .get(pos)
                .map(|&i| model_label(&app.models[i]))
        };
        if cancel {
            if picker.dirty {
                self.notice(
                    NoticeLevel::Info,
                    "Scoped models discarded (save with ctrl+s)",
                );
            }
            return false;
        }
        if up || down {
            picker.selected = wrap_index(
                picker.selected,
                if up { -1 } else { 1 },
                picker.filtered.len(),
            );
            return true;
        }
        if confirm {
            if let Some(label) = label_at(self, picker, picker.selected) {
                if let Some(pos) = picker.enabled.iter().position(|e| *e == label) {
                    picker.enabled.remove(pos);
                } else {
                    picker.enabled.push(label);
                }
                picker.dirty = true;
            }
            return true;
        }
        if self.keys.matches("app.models.save", key) {
            let value = if picker.enabled.is_empty() {
                None
            } else {
                Some(JsonValue::Array(
                    picker
                        .enabled
                        .iter()
                        .cloned()
                        .map(JsonValue::String)
                        .collect(),
                ))
            };
            match self.host.set_setting(&["enabledModels"], value) {
                Ok(()) => {
                    self.settings = self.host.settings();
                    self.notice(NoticeLevel::Info, "Saved scoped models to settings");
                }
                Err(error) => self.notice(NoticeLevel::Error, error),
            }
            return false;
        }
        if self.keys.matches("app.models.enableAll", key) {
            for pos in 0..picker.filtered.len() {
                if let Some(label) = label_at(self, picker, pos)
                    && !picker.enabled.contains(&label)
                {
                    picker.enabled.push(label);
                }
            }
            picker.dirty = true;
            return true;
        }
        if self.keys.matches("app.models.clearAll", key) {
            let visible: Vec<String> = (0..picker.filtered.len())
                .filter_map(|pos| label_at(self, picker, pos))
                .collect();
            picker.enabled.retain(|e| !visible.contains(e));
            picker.dirty = true;
            return true;
        }
        if self.keys.matches("app.models.toggleProvider", key) {
            if let Some(&i) = picker.filtered.get(picker.selected) {
                let provider = self.models[i].provider.clone();
                let labels: Vec<String> = self
                    .models
                    .iter()
                    .filter(|m| m.provider == provider)
                    .map(model_label)
                    .collect();
                let all_on = labels.iter().all(|l| picker.enabled.contains(l));
                if all_on {
                    picker.enabled.retain(|e| !labels.contains(e));
                } else {
                    for label in labels {
                        if !picker.enabled.contains(&label) {
                            picker.enabled.push(label);
                        }
                    }
                }
                picker.dirty = true;
            }
            return true;
        }
        let reorder_up = self.keys.matches("app.models.reorderUp", key);
        if reorder_up || self.keys.matches("app.models.reorderDown", key) {
            if let Some(label) = label_at(self, picker, picker.selected)
                && let Some(pos) = picker.enabled.iter().position(|e| *e == label)
            {
                let target = if reorder_up {
                    pos.checked_sub(1)
                } else {
                    Some(pos + 1)
                };
                if let Some(target) = target.filter(|t| *t < picker.enabled.len()) {
                    picker.enabled.swap(pos, target);
                    picker.dirty = true;
                }
            }
            return true;
        }
        if edit_query(&mut picker.query, key) {
            picker.filtered = fuzzy::filter(&self.models, &picker.query, model_label);
            picker.selected = 0;
        }
        true
    }

    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "pre-resolved select actions shared by every selector"
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "one branch per pi session-selector binding"
    )]
    fn sessions_key(
        &mut self,
        picker: &mut SessionPicker,
        key: &KeyId,
        up: bool,
        down: bool,
        confirm: bool,
        cancel: bool,
    ) -> bool {
        if let Some(input) = &mut picker.rename {
            if cancel {
                picker.rename = None;
            } else if confirm {
                let name = input.text().trim().to_owned();
                picker.rename = None;
                if let Some(item) = picker.model.selected_item().cloned()
                    && !name.is_empty()
                {
                    self.send(
                        Tag::SetName(name.clone()),
                        PiCommand::SetSessionName { name: name.clone() },
                    );
                    picker.model.rename(&item.path, Some(name));
                }
            } else {
                line_edit(input, key);
            }
            return true;
        }
        if let Some(path) = picker.model.confirm_delete.clone() {
            if confirm {
                picker.model.confirm_delete = None;
                self.effects.push(Effect::DeleteSession(path));
            } else if cancel {
                picker.model.confirm_delete = None;
            }
            return true;
        }
        if cancel {
            if picker.model.query.is_empty() {
                return false;
            }
            picker.model.set_query("");
            return true;
        }
        if up || down {
            picker.model.move_by(if up { -1 } else { 1 });
            return true;
        }
        if confirm {
            if let Some(item) = picker.model.selected_item() {
                let path = item.path.clone();
                if Some(&path) != self.session.session_file.as_ref() {
                    self.send(
                        Tag::Replace("Resume"),
                        PiCommand::SwitchSession { session_path: path },
                    );
                }
            }
            return false;
        }
        if self.keys.matches("tui.input.tab", key) {
            picker.model.scope = match picker.model.scope {
                Scope::Current => Scope::All,
                Scope::All => Scope::Current,
            };
            picker.model.loading = true;
            self.effects.push(Effect::ListSessions(picker.model.scope));
            return true;
        }
        if self.keys.matches("app.session.togglePath", key) {
            picker.model.show_path = !picker.model.show_path;
            return true;
        }
        if self.keys.matches("app.session.toggleSort", key) {
            picker.model.toggle_sort();
            return true;
        }
        if self.keys.matches("app.session.toggleNamedFilter", key) {
            picker.model.toggle_named();
            return true;
        }
        if self.keys.matches("app.session.rename", key) {
            let is_current = picker
                .model
                .selected_item()
                .is_some_and(|i| Some(&i.path) == self.session.session_file.as_ref());
            if is_current {
                let mut input = Composer::new();
                input.set_text(self.session.session_name.as_deref().unwrap_or(""));
                picker.rename = Some(input);
            } else {
                picker.status = Some("Only the current session can be renamed here".into());
            }
            return true;
        }
        let delete = self.keys.matches("app.session.delete", key)
            || (self.keys.matches("app.session.deleteNoninvasive", key)
                && picker.model.query.is_empty());
        if delete {
            if let Some(item) = picker.model.selected_item() {
                if Some(&item.path) == self.session.session_file.as_ref() {
                    picker.status = Some("Cannot delete the current session".into());
                } else {
                    picker.model.confirm_delete = Some(item.path.clone());
                }
            }
            return true;
        }
        let mut query = picker.model.query.clone();
        if edit_query(&mut query, key) {
            picker.model.set_query(query);
        }
        true
    }

    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "pre-resolved select actions shared by every selector"
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "pi tree-selector handleInput: one branch per binding"
    )]
    fn tree_key(
        &mut self,
        tree: &mut TreeOverlay,
        key: &KeyId,
        up: bool,
        down: bool,
        confirm: bool,
        cancel: bool,
    ) -> bool {
        match &mut tree.prompt {
            Some(TreePrompt::Label(entry, input)) => {
                if cancel {
                    tree.prompt = None;
                } else if confirm {
                    let text = input.text().trim().to_owned();
                    let label = (!text.is_empty()).then_some(text);
                    let entry_id = entry.clone();
                    self.send(
                        Tag::SetLabel {
                            entry_id: entry_id.clone(),
                            label: label.clone(),
                        },
                        OinoCommand::SetLabel { entry_id, label },
                    );
                    tree.prompt = None;
                } else {
                    line_edit(input, key);
                }
                return true;
            }
            Some(TreePrompt::Summary { target, selected }) => {
                if cancel {
                    tree.prompt = None;
                } else if up || down {
                    *selected = wrap_index(*selected, if up { -1 } else { 1 }, 3);
                } else if confirm {
                    let target = target.clone();
                    match *selected {
                        0 => {
                            self.navigate(target, false, None);
                            return false;
                        }
                        1 => {
                            self.navigate(target, true, None);
                            return false;
                        }
                        _ => tree.prompt = Some(TreePrompt::Instructions(target, Composer::new())),
                    }
                }
                return true;
            }
            Some(TreePrompt::Instructions(target, input)) => {
                if cancel {
                    tree.prompt = None;
                } else if confirm {
                    let instructions = input.text().trim().to_owned();
                    let target = target.clone();
                    self.navigate(
                        target,
                        true,
                        (!instructions.is_empty()).then_some(instructions),
                    );
                    return false;
                } else {
                    line_edit(input, key);
                }
                return true;
            }
            None => {}
        }
        let model = &mut tree.model;
        if up || down {
            model.move_by(if up { -1 } else { 1 });
        } else if self.keys.matches("app.tree.foldOrUp", key) {
            model.fold_or_up();
        } else if self.keys.matches("app.tree.unfoldOrDown", key) {
            model.unfold_or_down();
        } else if self.keys.matches("tui.select.pageUp", key) || key.key == Key::Left {
            model.page(-10);
        } else if self.keys.matches("tui.select.pageDown", key) || key.key == Key::Right {
            model.page(10);
        } else if confirm {
            let Some(entry) = model.selected_entry() else {
                return true;
            };
            let id = entry.id.as_str().to_owned();
            if current_leaf(model).as_deref() == Some(id.as_str()) {
                self.notice(NoticeLevel::Info, "Already at this point");
                return false;
            }
            tree.prompt = Some(TreePrompt::Summary {
                target: id,
                selected: 0,
            });
        } else if self.keys.matches("app.message.copy", key) {
            match model.selected_copy_text() {
                Some(text) => {
                    self.effects.push(Effect::ClipboardWrite(text));
                    self.notice(NoticeLevel::Info, "Copied to clipboard");
                }
                None => self.notice(NoticeLevel::Info, "Nothing to copy"),
            }
        } else if cancel {
            if model.query.is_empty() {
                return false;
            }
            model.set_query("");
        } else if self.keys.matches("app.tree.filter.default", key) {
            model.set_filter(FilterMode::Default);
        } else if self.keys.matches("app.tree.filter.noTools", key) {
            model.toggle_filter(FilterMode::NoTools);
        } else if self.keys.matches("app.tree.filter.userOnly", key) {
            model.toggle_filter(FilterMode::UserOnly);
        } else if self.keys.matches("app.tree.filter.labeledOnly", key) {
            model.toggle_filter(FilterMode::LabeledOnly);
        } else if self.keys.matches("app.tree.filter.all", key) {
            model.toggle_filter(FilterMode::All);
        } else if self.keys.matches("app.tree.filter.cycleBackward", key) {
            model.cycle_filter(false);
        } else if self.keys.matches("app.tree.filter.cycleForward", key) {
            model.cycle_filter(true);
        } else if self.keys.matches("tui.editor.deleteCharBackward", key) {
            let mut query = model.query.clone();
            query.pop();
            model.set_query(query);
        } else if self.keys.matches("app.tree.editLabel", key) {
            if let Some(entry) = model.selected_entry() {
                let id = entry.id.as_str().to_owned();
                let mut input = Composer::new();
                input.set_text(model.selected_label().unwrap_or(""));
                tree.prompt = Some(TreePrompt::Label(id, input));
            }
        } else if self.keys.matches("app.tree.toggleLabelTimestamp", key) {
            model.show_label_timestamps = !model.show_label_timestamps;
        } else if let Some(c) = key.printable() {
            let mut query = model.query.clone();
            query.push(c);
            model.set_query(query);
        }
        true
    }

    fn navigate(
        &mut self,
        target_id: String,
        summarize: bool,
        custom_instructions: Option<String>,
    ) {
        self.send_concurrent(
            Tag::Navigate,
            OinoCommand::NavigateTree {
                target_id,
                options: NavigateTreeOptions {
                    summarize,
                    custom_instructions,
                    replace_instructions: false,
                    label: None,
                },
            },
        );
    }

    /// Thinking level label for selectors.
    pub(crate) fn thinking_label(level: ThinkingLevel) -> &'static str {
        level_name(level)
    }
}

fn current_leaf(model: &TreeModel) -> Option<String> {
    model
        .rows(0, model.len())
        .into_iter()
        .find(|r| r.is_leaf)
        .map(|r| r.entry_id)
}

/// Single-line editing keys shared by inline inputs.
pub(crate) fn line_edit(input: &mut Composer, key: &KeyId) {
    match key.key {
        Key::Backspace if key.mods.ctrl || key.mods.alt => input.delete_word_backward(),
        Key::Backspace => input.backspace(),
        Key::Delete => input.delete(),
        Key::Left if key.mods.ctrl || key.mods.alt => input.word_left(),
        Key::Left => input.left(),
        Key::Right if key.mods.ctrl || key.mods.alt => input.word_right(),
        Key::Right => input.right(),
        Key::Home => input.home(),
        Key::End => input.end(),
        Key::Char('a') if key.mods.ctrl => input.home(),
        Key::Char('e') if key.mods.ctrl => input.end(),
        Key::Char('u') if key.mods.ctrl => input.delete_to_line_start(),
        Key::Char('k') if key.mods.ctrl => input.delete_to_line_end(),
        Key::Char('w') if key.mods.ctrl => input.delete_word_backward(),
        _ => {
            if let Some(c) = key.printable() {
                input.insert_char(c);
            }
        }
    }
}
