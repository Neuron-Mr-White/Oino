//! pi `AgentSession.cycleModel` selection rules and thinking level on model switches.

use indexmap::IndexMap;
use oino_ai::{Model, ThinkingLevel};

use super::defaults::DEFAULT_THINKING_LEVEL;
use super::initial::model_thinking_level_key;
use super::scope::ScopedModel;

/// Cycle direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CycleDirection {
    /// Next model.
    #[default]
    Forward,
    /// Previous model.
    Backward,
}

fn same_model(a: &Model, b: &Model) -> bool {
    a.id == b.id && a.provider == b.provider
}

fn step(len: usize, current: Option<usize>, direction: CycleDirection) -> usize {
    let current = current.unwrap_or(0);
    match direction {
        CycleDirection::Forward => (current + 1) % len,
        CycleDirection::Backward => (current + len - 1) % len,
    }
}

/// pi `_cycleScopedModel`: the next scoped model among those still available; `None` when at
/// most one remains.
#[must_use]
pub fn next_scoped_model(
    scoped: &[ScopedModel],
    available: &[Model],
    current: Option<&Model>,
    direction: CycleDirection,
) -> Option<ScopedModel> {
    let candidates: Vec<&ScopedModel> = scoped
        .iter()
        .filter(|s| available.iter().any(|m| same_model(m, &s.model)))
        .collect();
    if candidates.len() <= 1 {
        return None;
    }
    let index = current.and_then(|c| candidates.iter().position(|s| same_model(&s.model, c)));
    Some(candidates[step(candidates.len(), index, direction)].clone())
}

/// pi `_cycleAvailableModel`.
#[must_use]
pub fn next_available_model(
    available: &[Model],
    current: Option<&Model>,
    direction: CycleDirection,
) -> Option<Model> {
    if available.len() <= 1 {
        return None;
    }
    let index = current.and_then(|c| available.iter().position(|m| same_model(m, c)));
    Some(available[step(available.len(), index, direction)].clone())
}

/// Inputs of [`thinking_level_for_model_switch`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ThinkingSwitch<'a> {
    /// Level given explicitly (scoped `model:high`).
    pub explicit: Option<ThinkingLevel>,
    /// Settings `modelThinkingLevels`.
    pub model_thinking_levels: Option<&'a IndexMap<String, ThinkingLevel>>,
    /// Settings `defaultThinkingLevel`.
    pub default_thinking_level: Option<ThinkingLevel>,
    /// The session's current level.
    pub current: Option<ThinkingLevel>,
}

/// pi `_getThinkingLevelForModelSwitch`: explicit > per-model setting > default setting >
/// current > `medium` (the caller clamps to the model).
#[must_use]
pub fn thinking_level_for_model_switch(
    target: Option<&Model>,
    inputs: &ThinkingSwitch<'_>,
) -> ThinkingLevel {
    if let Some(explicit) = inputs.explicit {
        return explicit;
    }
    if let Some(level) = target.and_then(|model| {
        inputs
            .model_thinking_levels?
            .get(&model_thinking_level_key(&model.provider, &model.id))
            .copied()
    }) {
        return level;
    }
    inputs
        .default_thinking_level
        .or(inputs.current)
        .unwrap_or(DEFAULT_THINKING_LEVEL)
}
