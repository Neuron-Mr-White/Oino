//! Backend-driven frontend features (modes.md §4): merged completions
//! (`_oino/get_completions`), editor intents (`_oino/editor_event`), the extension settings
//! hub (`_oino/get_settings_schema` & co.) and provider sign-in (`_oino/login` / `logout`).

use std::time::{Duration, Instant};

use oino_ai::JsonValue;
use oino_client::protocol::{
    AuthProviderInfo, CompletionGroup, EditorEvent, EditorIntent, OinoCommand, SettingsScope,
};
use oino_core::extensions::settings::SettingsLayer;
use oino_view::composer::Composer;
use oino_view::keys::{Key, KeyId};
use oino_view::settings_hub::{self, FieldKind, FieldScope, HubField, HubNamespace};
use oino_view::transcript::NoticeLevel;

use super::overlay::{Overlay, line_edit};
use super::{App, Completion, EditPurpose, Effect, Tag};

/// `changed` intents are debounced (ui-protocol §7.2).
const EDITOR_DEBOUNCE: Duration = Duration::from_millis(150);

/// Owner id of the core `oino.*` settings namespaces (`oino_core::extensions::settings`).
const CORE_SETTINGS_OWNER: &str = "oino";

/// A fresh object key for a `dict` row (`new`, `new2`, …).
fn next_key(rows: &[(String, JsonValue)]) -> String {
    (1..=usize::MAX)
        .map(|index| {
            if index == 1 {
                "new".to_owned()
            } else {
                format!("new{index}")
            }
        })
        .find(|candidate| !rows.iter().any(|(key, _)| key == candidate))
        .unwrap_or_else(|| "new".to_owned())
}

/// One extension namespace page of the settings hub.
#[derive(Clone, Debug)]
pub(crate) struct ExtSettingsPage {
    pub(crate) namespace: HubNamespace,
    pub(crate) fields: Vec<HubField>,
    /// Effective values (secrets masked).
    pub(crate) values: JsonValue,
    /// Write target (Tab toggles).
    pub(crate) scope: SettingsScope,
    pub(crate) selected: usize,
    pub(crate) editing: Option<Composer>,
    pub(crate) status: Option<String>,
    /// Stored values `resolve` rejected (ui-protocol §7.1).
    pub(crate) diagnostics: Vec<String>,
    /// Open `list` / `dict` sub-page (rows of one field).
    pub(crate) sub: Option<ExtSubPage>,
}

impl ExtSettingsPage {
    /// Indices of the rows rendered for the current values (`visibleIf`, §7.4).
    pub(crate) fn visible(&self) -> Vec<usize> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.is_visible(&self.values))
            .map(|(index, _)| index)
            .collect()
    }

    /// The selected field, if the selection points at a rendered row.
    pub(crate) fn selected_field(&self) -> Option<&HubField> {
        let field = self.fields.get(self.selected)?;
        field.is_visible(&self.values).then_some(field)
    }

    /// Moves the selection by `delta` over the rendered rows (wrapping).
    pub(crate) fn move_selection(&mut self, delta: isize) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let position = visible
            .iter()
            .position(|index| *index == self.selected)
            .unwrap_or(0);
        let len = isize::try_from(visible.len()).unwrap_or(isize::MAX);
        let next = (isize::try_from(position).unwrap_or(0) + delta).rem_euclid(len);
        self.selected = visible[usize::try_from(next).unwrap_or(0)];
    }
}

/// A `list` / `dict` field's rows, edited one row at a time (§7 TUI rendering).
#[derive(Clone, Debug)]
pub(crate) struct ExtSubPage {
    /// Label of the parent field.
    pub(crate) title: String,
    /// Parent field (`FieldKind::List` / `FieldKind::Dict`).
    pub(crate) field: HubField,
    /// Rows in stored order (array index or dict key, value).
    pub(crate) rows: Vec<(String, JsonValue)>,
    pub(crate) selected: usize,
    pub(crate) editing: Option<Composer>,
    pub(crate) status: Option<String>,
}

/// `/login` / `/logout` provider picker.
#[derive(Clone, Debug)]
pub(crate) struct ProviderPicker {
    pub(crate) providers: Vec<AuthProviderInfo>,
    pub(crate) selected: usize,
    pub(crate) logout: bool,
}

/// What a settings-schema reply opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaPurpose {
    /// Refresh the namespace rows of `/settings`.
    Hub,
    /// Open one namespace page (`openSettings{namespace}`).
    Namespace(String),
}

impl App {
    /// Asks the backend for completions at the cursor (merged when the reply arrives).
    pub(crate) fn request_completions(&mut self) {
        let text = self.composer.text();
        if text.trim().is_empty() {
            return;
        }
        let cursor = self.composer.cursor_offset();
        self.send_concurrent(
            Tag::Completions {
                text: text.clone(),
                cursor,
            },
            OinoCommand::GetCompletions { text, cursor },
        );
    }

    /// Merges backend groups (command arguments, `complete-input` contributors, core `@`
    /// paths when no local popup is open) into the popup, if the composer did not change.
    pub(crate) fn on_completions(&mut self, text: &str, cursor: usize, data: Option<&JsonValue>) {
        if self.composer.text() != text || self.composer.cursor_offset() != cursor {
            return;
        }
        let groups: Vec<CompletionGroup> = data
            .and_then(|d| d.get("groups"))
            .and_then(|g| serde_json::from_value(g.clone()).ok())
            .unwrap_or_default();
        let local = self.completion.is_some();
        let remote: Vec<CompletionGroup> = groups
            .into_iter()
            .filter(|g| g.source != "commands" && !(local && g.source == "files"))
            .collect();
        let Some(from) = remote.first().and_then(|g| g.from) else {
            return;
        };
        let items: Vec<(String, String)> = remote
            .iter()
            .filter(|g| g.from == Some(from))
            .flat_map(|g| g.items.iter())
            .map(|i| {
                let label = i
                    .description
                    .clone()
                    .or_else(|| i.tag.clone())
                    .unwrap_or_default();
                (i.value.clone(), label)
            })
            .collect();
        if items.is_empty() {
            return;
        }
        match &mut self.completion {
            Some(completion) if completion.replace_from.is_none() && !completion.commands => {
                for item in items {
                    if !completion.items.contains(&item) {
                        completion.items.push(item);
                    }
                }
            }
            Some(_) => {}
            None => {
                self.completion = Some(Completion {
                    items,
                    selected: 0,
                    commands: false,
                    replace_from: Some(from),
                });
            }
        }
    }

    /// Notes a composer change for the debounced `changed` / `cleared` intent.
    pub(crate) fn note_editor_change(&mut self) {
        if self.composer.text() != self.editor_reported {
            self.editor_changed_at.get_or_insert_with(Instant::now);
        }
    }

    /// Sends the debounced `changed` / `cleared` intent once the text settled.
    pub(crate) fn flush_editor_event(&mut self) {
        let Some(since) = self.editor_changed_at else {
            return;
        };
        if since.elapsed() < EDITOR_DEBOUNCE {
            return;
        }
        self.editor_changed_at = None;
        let text = self.composer.text();
        if text == self.editor_reported {
            return;
        }
        let intent = if text.is_empty() {
            EditorIntent::Cleared
        } else {
            EditorIntent::Changed
        };
        self.editor_reported.clone_from(&text);
        self.editor_intent(intent, text, None);
    }

    /// Sends one editor intent; consumable intents route their reply to
    /// [`Tag::EditorConsumable`].
    pub(crate) fn editor_intent(
        &mut self,
        intent: EditorIntent,
        text: String,
        token_id: Option<String>,
    ) {
        self.editor_events += 1;
        let event = EditorEvent {
            event_id: format!("e{}", self.editor_events),
            intent,
            text,
            token_id,
        };
        let tag = if intent.consumable() {
            Tag::EditorConsumable(intent)
        } else {
            Tag::Ignore
        };
        self.send_concurrent(tag, OinoCommand::EditorEvent(event));
    }

    /// Default behaviour of a consumable intent nobody consumed.
    pub(crate) fn on_editor_consumed(&mut self, intent: EditorIntent, data: Option<&JsonValue>) {
        let consumed = data
            .and_then(|d| d.get("consumed"))
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if consumed {
            return;
        }
        if intent == EditorIntent::TokenBackspace {
            self.composer.backspace();
            self.note_editor_change();
        }
    }

    // ── settings hub ─────────────────────────────────────────────────────

    /// Fetches the extension settings schemas.
    pub(crate) fn request_settings_schema(&mut self, purpose: SchemaPurpose) {
        self.send(Tag::SettingsSchema(purpose), OinoCommand::GetSettingsSchema);
    }

    pub(crate) fn on_settings_schema(&mut self, purpose: SchemaPurpose, data: Option<&JsonValue>) {
        self.settings_schema = data
            .and_then(|d| d.get("namespaces"))
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let namespaces = settings_hub::namespaces(&self.settings_schema);
        match purpose {
            SchemaPurpose::Hub => {
                if let Some(Overlay::Settings(settings)) = &mut self.overlay {
                    // Core `oino.*` namespaces are the settings groups the rows above this
                    // list already edit (no duplicates, DIRECTION §4.2.2); the app renders them
                    // as its Agent group (ui-protocol §7).
                    settings.namespaces = namespaces
                        .into_iter()
                        .filter(|namespace| namespace.extension_id != CORE_SETTINGS_OWNER)
                        .collect();
                }
            }
            SchemaPurpose::Namespace(query) => {
                let found = namespaces
                    .into_iter()
                    .find(|n| n.namespace == query || n.extension_id == query);
                match found {
                    Some(namespace) => self.open_namespace(namespace),
                    None => self.notice(
                        NoticeLevel::Warning,
                        format!("No extension settings for \"{query}\""),
                    ),
                }
            }
        }
    }

    /// Opens a namespace page and loads its values.
    pub(crate) fn open_namespace(&mut self, namespace: HubNamespace) {
        let fields = settings_hub::fields(&self.settings_schema, &namespace.namespace);
        self.send(
            Tag::ExtValues(namespace.namespace.clone()),
            OinoCommand::GetExtensionSettings {
                namespace: namespace.namespace.clone(),
                scope: SettingsLayer::Effective,
            },
        );
        self.overlay = Some(Overlay::ExtSettings(ExtSettingsPage {
            namespace,
            fields,
            values: JsonValue::Null,
            scope: SettingsScope::Global,
            selected: 0,
            editing: None,
            status: None,
            diagnostics: Vec::new(),
            sub: None,
        }));
    }

    pub(crate) fn on_ext_values(&mut self, namespace: &str, data: Option<&JsonValue>) {
        if let Some(Overlay::ExtSettings(page)) = &mut self.overlay
            && page.namespace.namespace == namespace
        {
            page.values = data
                .and_then(|d| d.get("values"))
                .cloned()
                .unwrap_or(JsonValue::Null);
            page.diagnostics = data
                .and_then(|d| d.get("diagnostics"))
                .and_then(JsonValue::as_array)
                .map(|diagnostics| {
                    diagnostics
                        .iter()
                        .map(|d| {
                            let key = d.get("key").and_then(JsonValue::as_str).unwrap_or("?");
                            let message = d
                                .get("message")
                                .and_then(JsonValue::as_str)
                                .unwrap_or_default();
                            format!("{key}: {message}")
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(sub) = &mut page.sub
                && let Some(value) = settings_hub::value_at(&page.values, &sub.field.key)
            {
                sub.rows = sub.field.rows(Some(value));
                sub.selected = sub.selected.min(sub.rows.len().saturating_sub(1));
            }
        }
    }

    fn write_ext_setting(
        &mut self,
        page: &mut ExtSettingsPage,
        field: &HubField,
        value: Option<JsonValue>,
    ) {
        let scope = match field.scope {
            FieldScope::Global => SettingsScope::Global,
            FieldScope::Project => SettingsScope::Project,
            FieldScope::Both => page.scope,
        };
        let namespace = page.namespace.namespace.clone();
        let key = field.key.clone();
        let command = match value {
            Some(value) => OinoCommand::SetExtensionSetting {
                namespace: namespace.clone(),
                key,
                value: Some(value),
                scope,
            },
            None => OinoCommand::UnsetExtensionSetting {
                namespace: namespace.clone(),
                key,
                scope,
            },
        };
        page.status = None;
        self.send(Tag::ExtSet(namespace.clone()), command);
        self.send(
            Tag::ExtValues(namespace.clone()),
            OinoCommand::GetExtensionSettings {
                namespace,
                scope: SettingsLayer::Effective,
            },
        );
    }

    /// Open the row sub-page of a `list` / `dict` field.
    fn open_sub_page(page: &mut ExtSettingsPage, field: &HubField) {
        let value = settings_hub::value_at(&page.values, &field.key).cloned();
        page.sub = Some(ExtSubPage {
            title: field.label.clone(),
            field: field.clone(),
            rows: field.rows(value.as_ref()),
            selected: 0,
            editing: None,
            status: None,
        });
    }

    /// Writes the rows of an open sub-page back as the field's array / object.
    fn write_sub_rows(&mut self, page: &mut ExtSettingsPage) {
        let Some(sub) = &mut page.sub else {
            return;
        };
        let field = sub.field.clone();
        let value = field.container(&sub.rows);
        self.write_ext_setting(page, &field, Some(value));
    }

    /// Edits `json` values in `$EDITOR` (`editExternally`, §7).
    fn edit_json_externally(&mut self, page: &ExtSettingsPage, field: &HubField) {
        let text = settings_hub::value_at(&page.values, &field.key)
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_default())
            .unwrap_or_default();
        self.effects.push(Effect::ExternalEditor {
            text,
            extension: "json".into(),
            purpose: EditPurpose::Setting {
                namespace: page.namespace.namespace.clone(),
                key: field.key.clone(),
            },
        });
    }

    /// Keys of an open `list` / `dict` sub-page; `false` closes the whole overlay.
    fn ext_sub_key(&mut self, page: &mut ExtSettingsPage, key: &KeyId) -> bool {
        let up = self.keys.matches("tui.select.up", key);
        let down = self.keys.matches("tui.select.down", key);
        let confirm = self.keys.matches("tui.select.confirm", key);
        let cancel = self.keys.matches("tui.select.cancel", key);
        let Some(sub) = &mut page.sub else {
            return true;
        };
        if let Some(input) = &mut sub.editing {
            if cancel {
                sub.editing = None;
            } else if confirm {
                let text = input.text();
                sub.editing = None;
                let selected = sub.selected;
                if let Some((_, current)) = sub.rows.get(selected) {
                    let row = sub.field.row(current);
                    match row.parse(&text) {
                        Ok(value) => {
                            let value = value.unwrap_or(JsonValue::Null);
                            if let Some((_, slot)) = sub.rows.get_mut(selected) {
                                *slot = value;
                            }
                            self.write_sub_rows(page);
                        }
                        Err(error) => {
                            if let Some(sub) = &mut page.sub {
                                sub.status = Some(error);
                            }
                        }
                    }
                }
            } else {
                line_edit(input, key);
            }
            return true;
        }
        if cancel {
            page.sub = None;
            return true;
        }
        let len = sub.rows.len();
        if up && len > 0 {
            sub.selected = (sub.selected + len - 1) % len;
        } else if down && len > 0 {
            sub.selected = (sub.selected + 1) % len;
        } else if key.key == Key::Char('a') && !key.mods.ctrl && !key.mods.alt {
            let value = sub.field.new_row();
            let name = match &sub.field.kind {
                FieldKind::Dict { .. } => next_key(&sub.rows),
                _ => sub.rows.len().to_string(),
            };
            sub.rows.push((name, value));
            sub.selected = sub.rows.len() - 1;
            self.write_sub_rows(page);
        } else if (key.key == Key::Delete || (key.key == Key::Char('d') && key.mods.ctrl))
            && let Some((key, _)) = sub.rows.get(sub.selected).cloned()
        {
            let _ = key;
            sub.rows.remove(sub.selected);
            sub.selected = sub.selected.min(sub.rows.len().saturating_sub(1));
            self.write_sub_rows(page);
        } else if confirm || key.key == Key::Space || key.key == Key::Right || key.key == Key::Left
        {
            let Some((_, current)) = sub.rows.get(sub.selected).cloned() else {
                return true;
            };
            let row = sub.field.row(&current);
            if row.is_text() || row.is_json() {
                let mut input = Composer::new();
                if !matches!(row.kind, FieldKind::Secret) {
                    input.set_text(&match &current {
                        JsonValue::String(text) => text.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    });
                }
                sub.editing = Some(input);
            } else if let Some(next) = row.cycle(Some(&current), key.key == Key::Left) {
                if let Some((_, slot)) = sub.rows.get_mut(sub.selected) {
                    *slot = next;
                }
                self.write_sub_rows(page);
            } else if row.is_container()
                && let Some(sub) = &mut page.sub
            {
                sub.status = Some(format!("{} rows are edited one level down", row.label));
            }
        }
        true
    }

    /// Writes a `json` settings field after the external editor returned (§7).
    pub(crate) fn on_setting_edited(&mut self, namespace: &str, key: &str, text: Option<String>) {
        let Some(Overlay::ExtSettings(mut page)) = self.overlay.take() else {
            return;
        };
        if page.namespace.namespace != namespace {
            self.overlay = Some(Overlay::ExtSettings(page));
            return;
        }
        match text {
            Some(text) => match serde_json::from_str::<JsonValue>(&text) {
                Ok(value) => {
                    if let Some(field) = page.fields.iter().find(|field| field.key == key).cloned()
                    {
                        self.write_ext_setting(&mut page, &field, Some(value));
                    }
                }
                Err(error) => page.status = Some(format!("{key}: {error}")),
            },
            None => page.status = Some("external editor cancelled".into()),
        }
        self.overlay = Some(Overlay::ExtSettings(page));
    }

    /// Keys of a namespace page; returns whether the overlay stays open.
    pub(crate) fn ext_settings_key(&mut self, page: &mut ExtSettingsPage, key: &KeyId) -> bool {
        if page.sub.is_some() {
            return self.ext_sub_key(page, key);
        }
        let up = self.keys.matches("tui.select.up", key);
        let down = self.keys.matches("tui.select.down", key);
        let confirm = self.keys.matches("tui.select.confirm", key);
        let cancel = self.keys.matches("tui.select.cancel", key);
        if let Some(input) = &mut page.editing {
            if cancel {
                page.editing = None;
            } else if confirm {
                let text = input.text();
                page.editing = None;
                if let Some(field) = page.selected_field().cloned() {
                    match field.parse(&text) {
                        Ok(value) => self.write_ext_setting(page, &field, value),
                        Err(error) => page.status = Some(error),
                    }
                }
            } else {
                line_edit(input, key);
            }
            return true;
        }
        if cancel {
            self.open_settings(None);
            return false;
        }
        if up {
            page.move_selection(-1);
            return true;
        }
        if down {
            page.move_selection(1);
            return true;
        }
        if key.key == Key::Tab {
            page.scope = match page.scope {
                SettingsScope::Global => SettingsScope::Project,
                SettingsScope::Project => SettingsScope::Global,
            };
            return true;
        }
        let Some(field) = page.selected_field().cloned() else {
            return true;
        };
        let current = settings_hub::value_at(&page.values, &field.key).cloned();
        if key.key == Key::Delete || (key.key == Key::Char('d') && key.mods.ctrl) {
            self.write_ext_setting(page, &field, None);
            return true;
        }
        let cycle =
            confirm || key.key == Key::Space || key.key == Key::Right || key.key == Key::Left;
        if !cycle {
            return true;
        }
        if !field.is_enabled(&page.values) {
            page.status = Some(format!("{} is disabled here", field.label));
            return true;
        }
        if let FieldKind::Action { command, .. } = &field.kind {
            if confirm {
                let command = format!("/{}", command.trim_start_matches('/'));
                self.submit_text(&command, Vec::new(), false);
                return false;
            }
            return true;
        }
        if field.is_container() {
            if confirm {
                Self::open_sub_page(page, &field);
            }
            return true;
        }
        if field.is_json() {
            if confirm {
                self.edit_json_externally(page, &field);
            }
            return true;
        }
        if field.is_text() && confirm {
            let mut input = Composer::new();
            if !matches!(field.kind, FieldKind::Secret)
                && let Some(value) = &current
            {
                input.set_text(
                    &value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned),
                );
            }
            page.editing = Some(input);
            return true;
        }
        if let Some(next) = field.cycle(current.as_ref(), key.key == Key::Left) {
            self.write_ext_setting(page, &field, Some(next));
        } else if !field.is_text() {
            page.status = Some(format!("{} is read-only here", field.label));
        }
        true
    }

    // ── sign-in ──────────────────────────────────────────────────────────

    /// `/login [provider]` and `/logout [provider]`.
    pub(crate) fn auth_command(&mut self, logout: bool, provider: Option<String>) {
        match provider {
            Some(provider) if logout => {
                self.send(
                    Tag::Logout(provider.clone()),
                    OinoCommand::Logout { provider },
                );
            }
            Some(provider) => self.login(provider),
            None => self.send(Tag::AuthProviders { logout }, OinoCommand::AuthProviders),
        }
    }

    fn login(&mut self, provider: String) {
        self.notice(NoticeLevel::Info, format!("Signing in to {provider}…"));
        // Concurrent: the flow raises dialogs while the command is pending.
        self.send_concurrent(
            Tag::Login(provider.clone()),
            OinoCommand::Login { provider },
        );
    }

    pub(crate) fn on_auth_providers(&mut self, logout: bool, data: Option<&JsonValue>) {
        let providers: Vec<AuthProviderInfo> = data
            .and_then(|d| d.get("providers"))
            .and_then(|p| serde_json::from_value(p.clone()).ok())
            .unwrap_or_default();
        let providers: Vec<AuthProviderInfo> = providers
            .into_iter()
            .filter(|p| if logout { p.stored.is_some() } else { p.oauth })
            .collect();
        if providers.is_empty() {
            let text = if logout {
                "No stored credentials to remove"
            } else {
                "No provider offers sign-in; set an API key with the provider's environment variable or auth.json"
            };
            self.notice(NoticeLevel::Info, text);
            return;
        }
        self.overlay = Some(Overlay::Providers(ProviderPicker {
            providers,
            selected: 0,
            logout,
        }));
    }

    /// Keys of the provider picker; returns whether it stays open.
    pub(crate) fn provider_key(&mut self, picker: &mut ProviderPicker, key: &KeyId) -> bool {
        let len = picker.providers.len();
        if self.keys.matches("tui.select.cancel", key) {
            return false;
        }
        if self.keys.matches("tui.select.up", key) && len > 0 {
            picker.selected = (picker.selected + len - 1) % len;
        } else if self.keys.matches("tui.select.down", key) && len > 0 {
            picker.selected = (picker.selected + 1) % len;
        } else if self.keys.matches("tui.select.confirm", key) {
            if let Some(provider) = picker.providers.get(picker.selected) {
                let id = provider.id.clone();
                if picker.logout {
                    self.send(
                        Tag::Logout(id.clone()),
                        OinoCommand::Logout { provider: id },
                    );
                } else {
                    self.login(id);
                }
            }
            return false;
        }
        true
    }
}
