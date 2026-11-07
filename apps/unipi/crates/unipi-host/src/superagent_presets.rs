//! Persisted Superagent presets and per-project watcher preferences.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    pub autonomy: String,
    pub mode: String,
    #[serde(default)]
    pub max_parallel: Option<usize>,
    pub review: String,
    #[serde(default)]
    pub profiles: Option<Vec<String>>,
    #[serde(default)]
    pub priority: bool,
}
impl Preset {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.label.trim().is_empty() {
            return Err("preset id and label are required".into());
        }
        if !matches!(self.autonomy.as_str(), "low" | "medium" | "high") {
            return Err("invalid autonomy".into());
        }
        if !matches!(
            self.mode.as_str(),
            "none" | "goal" | "ralph" | "swarm" | "graph"
        ) {
            return Err("invalid mode".into());
        }
        if !matches!(self.review.as_str(), "none" | "light" | "full") {
            return Err("invalid review".into());
        }
        if self.max_parallel == Some(0) {
            return Err("maxParallel must be positive".into());
        }
        if self.model.as_ref().is_some_and(|model| {
            !model
                .split_once('/')
                .is_some_and(|(p, id)| !p.is_empty() && !id.is_empty())
        }) {
            return Err("model must be provider/id".into());
        }
        if let Some(thinking) = &self.thinking {
            thinking
                .parse::<oino_ai::ThinkingLevel>()
                .map_err(|_| "invalid thinking level")?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Overrides {
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub autonomy: Option<String>,
    pub mode: Option<String>,
    pub max_parallel: Option<usize>,
    pub review: Option<String>,
    pub profiles: Option<Vec<String>>,
    pub priority: Option<bool>,
}
impl Overrides {
    pub fn apply(self, preset: &mut Preset) {
        if let Some(v) = self.model {
            preset.model = Some(v);
        }
        if let Some(v) = self.thinking {
            preset.thinking = Some(v);
        }
        if let Some(v) = self.autonomy {
            preset.autonomy = v;
        }
        if let Some(v) = self.mode {
            preset.mode = v;
        }
        if let Some(v) = self.max_parallel {
            preset.max_parallel = Some(v);
        }
        if let Some(v) = self.review {
            preset.review = v;
        }
        if let Some(v) = self.profiles {
            preset.profiles = Some(v);
        }
        if let Some(v) = self.priority {
            preset.priority = v;
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub default_preset_id: String,
    pub presets: Vec<Preset>,
    #[serde(default)]
    pub projects: HashMap<String, bool>,
}
impl Default for Store {
    fn default() -> Self {
        let entries = [
            (
                "quick-fix",
                "Quick fix",
                "A focused fix",
                "high",
                "none",
                None,
                "none",
                false,
            ),
            (
                "bug-batch",
                "Bug batch",
                "Batch independent bug fixes",
                "high",
                "swarm",
                Some(4),
                "light",
                false,
            ),
            (
                "feature",
                "Feature",
                "Implement a feature",
                "high",
                "graph",
                Some(3),
                "full",
                false,
            ),
            (
                "urgent",
                "Urgent",
                "Prioritize urgent work",
                "high",
                "none",
                None,
                "none",
                true,
            ),
            (
                "research",
                "Research",
                "Investigate a question",
                "medium",
                "none",
                None,
                "none",
                false,
            ),
        ];
        Self {
            default_preset_id: "quick-fix".into(),
            projects: HashMap::new(),
            presets: entries
                .into_iter()
                .map(
                    |(id, label, description, autonomy, mode, max_parallel, review, priority)| {
                        Preset {
                            id: id.into(),
                            label: label.into(),
                            description: description.into(),
                            model: None,
                            thinking: None,
                            autonomy: autonomy.into(),
                            mode: mode.into(),
                            max_parallel,
                            review: review.into(),
                            profiles: None,
                            priority,
                        }
                    },
                )
                .collect(),
        }
    }
}
impl Store {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::data_dir::write_json(
            path,
            &serde_json::to_value(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }
    pub fn resolve(&self, id: Option<&str>, overrides: Overrides) -> Result<Preset, String> {
        let id = id.unwrap_or(&self.default_preset_id);
        let mut preset = self
            .presets
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| format!("Unknown preset: {id}"))?;
        overrides.apply(&mut preset);
        preset.validate()?;
        Ok(preset)
    }
}
