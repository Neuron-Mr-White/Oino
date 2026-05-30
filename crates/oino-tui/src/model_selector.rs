#![forbid(unsafe_code)]

//! Reusable model selector panel with search, filtering, provider grouping, and cursor management.
//!
//! Used by:
//! - Main Settings → Model Selection
//! - Notify Settings → Summary Model picker
//! - Compaction Settings → LLM Model picker
//!
//! Each usage creates its own [`ModelSelector`] instance with a [`ModelSelectorContext`]
//! that identifies the purpose and customises the panel title.

use crate::fuzzy::{ascii_subsequence_match_parts, fuzzy_indices, FuzzyMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::settings::ModelOption;

/// Identifies which part of the UI owns this model selector, so rendering can
/// vary the title and so the caller can dispatch the final action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSelectorContext {
    /// Main model selection in Settings → Models.
    Main,
    /// Picking a summary model inside Settings → Notify.
    NotifySummary,
    /// Picking a compaction LLM model inside Settings → Compaction.
    CompactionModel,
}

impl ModelSelectorContext {
    /// Panel title shown when the selector is active (before model count is appended).
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Main => " Model Selection ",
            Self::NotifySummary => " Summary Model ",
            Self::CompactionModel => " Compaction Model ",
        }
    }

    /// The field value changed — the caller should persist `selected_model`.
    #[must_use]
    pub const fn changed_action_label(self) -> &'static str {
        match self {
            Self::Main => "model",
            Self::NotifySummary => "summary model",
            Self::CompactionModel => "compaction model",
        }
    }
}

/// Result of a single key press inside the model selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSelectorAction {
    /// No visible change; re-render is optional.
    None,
    /// The user confirmed a model. `id` is the canonical model identifier.
    Select { id: String },
    /// The user cancelled (pressed Esc at the top level).
    Cancel,
}

/// Standalone model selector state that can be embedded in any settings page.
///
/// Owns the model list, search state, filtered indices, and cursor. The host
/// page is responsible for:
/// 1. Calling [`ModelSelector::set_models`] when the catalog updates.
/// 2. Calling [`ModelSelector::handle_key`] for key events while active.
/// 3. Calling [`ModelSelector::cancel`] when the host decides to close the selector.
/// 4. Rendering via [`crate::render::render_model_panel`] (or equivalent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelector {
    /// Context identifies the caller for title/action dispatch.
    pub context: ModelSelectorContext,
    /// Full ordered model catalog.
    pub models: Vec<ModelOption>,
    /// The model considered "selected" before the selector was opened.
    pub initial_model: String,
    /// Currently highlighted model (index into `models`).
    pub cursor: usize,
    /// Search query typed by the user.
    pub search: String,
    /// Whether the search input is focused.
    pub search_active: bool,
    /// Indices into `models` matching the current search.
    pub filtered_indices: Vec<usize>,
    /// Status line text (e.g., "loaded", "refreshing…").
    pub status: String,
    /// Whether the catalog is currently being refreshed.
    pub refreshing: bool,
}

impl ModelSelector {
    /// Create a new selector bound to `context` with an initial model id.
    #[must_use]
    pub fn new(context: ModelSelectorContext, initial_model: impl Into<String>) -> Self {
        Self {
            context,
            models: Vec::new(),
            initial_model: initial_model.into(),
            cursor: 0,
            search: String::new(),
            search_active: false,
            filtered_indices: Vec::new(),
            status: "Model catalog not loaded yet".into(),
            refreshing: false,
        }
    }

    /// Open the selector: reset search, recompute filter, position cursor on
    /// the current selection.
    pub fn open(&mut self) {
        self.search_active = false;
        self.search.clear();
        self.reposition_cursor_to_initial();
        self.refresh_filter();
    }

    /// Reset state when closing/cancelling.
    pub fn cancel(&mut self) {
        self.search_active = false;
        self.search.clear();
        self.refresh_filter();
        self.reposition_cursor_to_initial();
    }

    /// Update the model catalog (e.g., after a provider refresh). Preserves
    /// the cursor position when possible.
    pub fn set_models(&mut self, mut models: Vec<ModelOption>, status: impl Into<String>) {
        let browsing_id = self.models.get(self.cursor).map(|m| m.id.clone());
        sort_model_options_for_display(&mut models);
        self.models = models;
        self.status = status.into();
        let target = browsing_id.as_deref().unwrap_or(&self.initial_model);
        self.cursor = self
            .models
            .iter()
            .position(|m| m.id == target)
            .or_else(|| self.models.iter().position(|m| m.id == self.initial_model))
            .unwrap_or_else(|| self.cursor.min(self.models.len().saturating_sub(1)));
        self.refresh_filter();
    }

    pub fn set_refreshing(&mut self, refreshing: bool) {
        self.refreshing = refreshing;
    }

    /// Handle a key event. Returns the action the host should perform.
    pub fn handle_key(&mut self, key: KeyEvent) -> ModelSelectorAction {
        if self.search_active {
            return self.handle_search_key(key);
        }
        match key.code {
            KeyCode::Esc => {
                self.cancel();
                ModelSelectorAction::Cancel
            }
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.search_active = true;
                self.search.clear();
                self.refresh_filter();
                ModelSelectorAction::None
            }
            KeyCode::Up => {
                self.move_cursor(-1);
                ModelSelectorAction::None
            }
            KeyCode::Down => {
                self.move_cursor(1);
                ModelSelectorAction::None
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.move_cursor(-1);
                ModelSelectorAction::None
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_cursor(1);
                ModelSelectorAction::None
            }
            KeyCode::Enter => self.apply_selection(),
            KeyCode::Backspace | KeyCode::Left => {
                self.cancel();
                ModelSelectorAction::Cancel
            }
            _ => ModelSelectorAction::None,
        }
    }

    // ── Query helpers ──────────────────────────────────────────────────

    /// Read-only access to the filtered indices (for rendering).
    #[must_use]
    pub fn filtered_indices(&self) -> &[usize] {
        &self.filtered_indices
    }

    /// Position of `cursor` inside `filtered_indices`.
    #[must_use]
    pub fn cursor_filtered_position(&self) -> usize {
        self.filtered_indices
            .iter()
            .position(|idx| *idx == self.cursor)
            .unwrap_or(0)
    }

    /// The model currently under the cursor.
    #[must_use]
    pub fn cursor_model(&self) -> Option<&ModelOption> {
        self.models.get(self.cursor)
    }

    /// The currently selected model's display label.
    #[must_use]
    pub fn selected_model_label(&self) -> &str {
        self.models
            .iter()
            .find(|m| m.id == self.initial_model)
            .map_or(self.initial_model.as_str(), |m| m.display_name.as_str())
    }

    // ── Private helpers ────────────────────────────────────────────────

    fn apply_selection(&mut self) -> ModelSelectorAction {
        if self.search_active {
            self.search_active = false;
        }
        let Some(model) = self.models.get(self.cursor) else {
            return ModelSelectorAction::None;
        };
        if self.initial_model == model.id {
            // User confirmed the same model — treat as cancel.
            return ModelSelectorAction::Cancel;
        }
        let id = model.id.clone();
        self.initial_model = id.clone();
        ModelSelectorAction::Select { id }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> ModelSelectorAction {
        match key.code {
            KeyCode::Esc => {
                self.search_active = false;
                self.search.clear();
                self.refresh_filter();
                self.reposition_cursor_to_initial();
                ModelSelectorAction::None
            }
            KeyCode::Enter => {
                self.search_active = false;
                ModelSelectorAction::None
            }
            KeyCode::Backspace => {
                self.search.pop();
                self.refresh_filter();
                ModelSelectorAction::None
            }
            KeyCode::Up => {
                self.move_cursor_filtered(-1);
                ModelSelectorAction::None
            }
            KeyCode::Down => {
                self.move_cursor_filtered(1);
                ModelSelectorAction::None
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.search.push(ch);
                self.refresh_filter();
                ModelSelectorAction::None
            }
            _ => ModelSelectorAction::None,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        // In non-search mode, move through all models
        if !self.search_active {
            let len = self.models.len();
            self.cursor = move_index(self.cursor, len, delta);
            return;
        }
        self.move_cursor_filtered(delta);
    }

    fn move_cursor_filtered(&mut self, delta: isize) {
        let indices = &self.filtered_indices;
        if indices.is_empty() {
            return;
        }
        let current = indices
            .iter()
            .position(|idx| *idx == self.cursor)
            .unwrap_or(0);
        let next = move_index(current, indices.len(), delta);
        self.cursor = indices[next];
    }

    fn refresh_filter(&mut self) {
        let query = self.search.trim();
        self.filtered_indices = if query.is_empty() {
            (0..self.models.len()).collect()
        } else {
            let candidates = model_filter_candidate_indices(&self.models, query);
            fuzzy_indices(&candidates, query, FuzzyMode::Text, None, |idx| {
                let m = &self.models[*idx];
                format!("{} {} {}", m.provider, m.id, m.display_name)
            })
            .into_iter()
            .map(|ci| candidates[ci])
            .collect()
        };
        // Keep cursor inside the filter
        if let Some(&first) = self.filtered_indices.first() {
            if !self.filtered_indices.contains(&self.cursor) {
                self.cursor = first;
            }
        }
    }

    fn reposition_cursor_to_initial(&mut self) {
        if let Some(idx) = self.models.iter().position(|m| m.id == self.initial_model) {
            self.cursor = idx;
        }
    }
}

// ── Free functions (extracted from settings.rs, now shared) ─────────

pub(super) fn sort_model_options_for_display(models: &mut [ModelOption]) {
    models.sort_by(|left, right| {
        left.availability
            .display_rank()
            .cmp(&right.availability.display_rank())
    });
}

pub(super) fn model_filter_candidate_indices(models: &[ModelOption], query: &str) -> Vec<usize> {
    if !query.is_ascii() {
        return (0..models.len()).collect();
    }
    if let Some(provider_prefix) = query.strip_suffix(':') {
        let lower = provider_prefix.to_lowercase();
        return models
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.provider.to_lowercase().starts_with(&lower).then_some(i))
            .collect();
    }
    models
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            ascii_subsequence_match_parts(
                [
                    m.provider.as_str(),
                    " ",
                    m.provider_label.as_str(),
                    " ",
                    m.id.as_str(),
                    " ",
                    m.display_name.as_str(),
                ],
                query,
            )
            .then_some(i)
        })
        .collect()
}

fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len.saturating_sub(1);
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs()).min(last)
    } else {
        current.saturating_add(delta as usize).min(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn selector_opens_cancels_and_selects() {
        let mut sel = ModelSelector::new(ModelSelectorContext::Main, "model-a");
        sel.set_models(
            vec![ModelOption::new("model-a"), ModelOption::new("model-b")],
            "loaded",
        );
        sel.open();
        assert!(sel.search.is_empty());

        // Move down and select model-b
        sel.handle_key(key(KeyCode::Down));
        let action = sel.handle_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            ModelSelectorAction::Select {
                id: "model-b".into()
            }
        );
        assert_eq!(sel.initial_model, "model-b");
    }

    #[test]
    fn selector_cancel_restores_initial_cursor() {
        let mut sel = ModelSelector::new(ModelSelectorContext::Main, "model-a");
        sel.set_models(
            vec![ModelOption::new("model-a"), ModelOption::new("model-b")],
            "loaded",
        );
        sel.open();
        sel.handle_key(key(KeyCode::Down));
        assert_eq!(sel.cursor, 1);
        let action = sel.handle_key(key(KeyCode::Esc));
        assert_eq!(action, ModelSelectorAction::Cancel);
        assert_eq!(sel.cursor, 0); // repositioned to initial
    }

    #[test]
    fn selector_search_filters_and_esc_clears() {
        let mut sel = ModelSelector::new(ModelSelectorContext::Main, "openai/gpt");
        sel.set_models(
            vec![
                ModelOption::new("anthropic/claude"),
                ModelOption::new("openai/gpt"),
                ModelOption::new("google/gemini"),
            ],
            "loaded",
        );
        sel.open();
        sel.handle_key(key(KeyCode::Char('/')));
        assert!(sel.search_active);
        sel.handle_key(key(KeyCode::Char('g')));
        assert_eq!(sel.search, "g");
        assert_eq!(sel.filtered_indices.len(), 2);
        assert!(sel.filtered_indices.contains(&1));
        assert!(sel.filtered_indices.contains(&2));

        sel.handle_key(key(KeyCode::Esc));
        assert!(!sel.search_active);
        assert_eq!(sel.search, "");
        assert_eq!(sel.cursor, 1);
    }

    #[test]
    fn set_models_preserves_cursor_on_refresh() {
        let mut sel = ModelSelector::new(ModelSelectorContext::Main, "model-a");
        sel.set_models(
            vec![ModelOption::new("model-a"), ModelOption::new("model-b")],
            "loaded",
        );
        sel.open();
        sel.cursor = 1;

        sel.set_models(
            vec![
                ModelOption::new("model-a"),
                ModelOption::new("model-b"),
                ModelOption::new("model-c"),
            ],
            "refreshed",
        );
        assert_eq!(sel.cursor, 1);
    }

    #[test]
    fn selector_selecting_same_model_returns_cancel() {
        let mut sel = ModelSelector::new(ModelSelectorContext::Main, "model-a");
        sel.set_models(vec![ModelOption::new("model-a")], "loaded");
        sel.open();
        let action = sel.handle_key(key(KeyCode::Enter));
        assert_eq!(action, ModelSelectorAction::Cancel);
    }

    #[test]
    fn context_titles_are_distinct() {
        assert!(ModelSelectorContext::Main.title().contains("Selection"));
        assert!(ModelSelectorContext::NotifySummary
            .title()
            .contains("Summary"));
        assert!(ModelSelectorContext::CompactionModel
            .title()
            .contains("Compaction"));
    }
}
