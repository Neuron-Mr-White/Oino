//! minimatch subset used by pi model scoping (`nocase`, `*` does not cross `/`).

use globset::GlobBuilder;

/// Whether `text` matches `pattern` case-insensitively. Invalid patterns match literally.
#[must_use]
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    match GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
    {
        Ok(glob) => glob.compile_matcher().is_match(text),
        Err(_) => pattern.eq_ignore_ascii_case(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("anthropic/*", "Anthropic/claude-4"));
        assert!(!glob_match("*", "anthropic/claude"));
        assert!(glob_match("*sonnet*", "claude-sonnet-4"));
        assert!(glob_match("gpt-?", "gpt-5"));
        assert!(glob_match("{a,b}/x", "b/x"));
        assert!(glob_match("[", "["));
    }
}
