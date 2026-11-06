//! Semantic search is opt-in and read once when constructing session tools.
impl crate::Settings {
    /// Semantic search defaults off, including old sessions/settings.
    #[must_use]
    pub fn semantic_search_enabled(&self) -> bool {
        self.value(&["search", "semanticEnabled"])
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_off_and_snapshot() {
        let mut manager = crate::SettingsManager::in_memory(serde_json::Map::new());
        assert!(!manager.settings().semantic_search_enabled());
        manager
            .set(
                crate::SettingsScope::Global,
                &["search", "semanticEnabled"],
                Some(serde_json::json!(true)),
            )
            .unwrap();
        assert!(manager.settings().semantic_search_enabled());
    }
}
