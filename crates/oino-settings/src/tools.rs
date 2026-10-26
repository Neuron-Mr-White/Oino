//! `defaultTools` selection rules (pi `settings-manager.ts` tool helpers).

use serde_json::Value;

/// Tools enabled at startup when `defaultTools` does not change them (pi `DEFAULT_TOOL_NAMES`).
pub const DEFAULT_TOOL_NAMES: [&str; 4] = ["read", "bash", "edit", "write"];

/// Whether `entry` is a `+name` or `-name` modifier (pi `isToolModifier`).
#[must_use]
pub fn is_tool_modifier(entry: &str) -> bool {
    entry.starts_with('+') || entry.starts_with('-')
}

/// pi `getToolListError`: a tool list is either plain names/patterns or only exact `+name` /
/// `-name` entries. Returns the problem, or `None` when the list is valid.
#[must_use]
pub fn tool_list_error<S: AsRef<str>>(entries: &[S]) -> Option<String> {
    let modifiers: Vec<&str> = entries
        .iter()
        .map(AsRef::as_ref)
        .filter(|entry| is_tool_modifier(entry))
        .collect();
    if modifiers.is_empty() {
        return None;
    }
    if modifiers.len() < entries.len() {
        return Some("tool names cannot be mixed with +name or -name entries".to_owned());
    }
    modifiers
        .iter()
        .find(|entry| entry.contains('*'))
        .map(|pattern| {
            format!("+name and -name entries take exact tool names, not patterns: {pattern}")
        })
}

/// pi `applyToolModifiers`: apply `+name` (add when absent) and `-name` (remove) in order.
#[must_use]
pub fn apply_tool_modifiers<S: AsRef<str>>(base: &[String], entries: &[S]) -> Vec<String> {
    let mut tools = base.to_vec();
    for entry in entries.iter().map(AsRef::as_ref) {
        if !is_tool_modifier(entry) {
            continue;
        }
        let name = &entry[1..];
        let index = tools.iter().position(|tool| tool == name);
        if entry.starts_with('+') {
            if index.is_none() && !name.is_empty() {
                tools.push(name.to_owned());
            }
        } else if let Some(index) = index {
            tools.remove(index);
        }
    }
    tools
}

/// pi `resolveDefaultTools`: plain names replace [`DEFAULT_TOOL_NAMES`], then modifiers apply.
#[must_use]
pub fn resolve_default_tools(entries: &[String]) -> Vec<String> {
    let plain: Vec<String> = entries
        .iter()
        .filter(|entry| !is_tool_modifier(entry))
        .cloned()
        .collect();
    let base = if !plain.is_empty() || entries.is_empty() {
        plain
    } else {
        DEFAULT_TOOL_NAMES
            .iter()
            .map(|&name| name.to_owned())
            .collect()
    };
    apply_tool_modifiers(&base, entries)
}

/// pi `mergeDefaultTools`: `None` when neither layer sets the key; a modifier-only override list
/// is appended to an array base, anything else replaces it.
pub(crate) fn merge_default_tools(
    base: Option<&Value>,
    overrides: Option<&Value>,
) -> Option<Value> {
    let Some(overrides) = overrides else {
        return base.cloned();
    };
    match (base, overrides) {
        (Some(Value::Array(base)), Value::Array(over))
            if over
                .iter()
                .all(|entry| entry.as_str().is_some_and(is_tool_modifier)) =>
        {
            Some(Value::Array(base.iter().chain(over).cloned().collect()))
        }
        _ => Some(overrides.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn resolve_matches_pi_docs() {
        assert_eq!(
            resolve_default_tools(&strings(&["-bash", "+powershell", "+grep"])),
            strings(&["read", "edit", "write", "powershell", "grep"])
        );
        assert_eq!(
            resolve_default_tools(&strings(&["read", "+grep", "-read"])),
            strings(&["grep"])
        );
        assert_eq!(resolve_default_tools(&[]), Vec::<String>::new());
    }

    #[test]
    fn tool_list_errors() {
        assert_eq!(tool_list_error(&["read", "bash"]), None);
        assert_eq!(
            tool_list_error(&["read", "+bash"]).as_deref(),
            Some("tool names cannot be mixed with +name or -name entries")
        );
        assert_eq!(
            tool_list_error(&["+a*"]).as_deref(),
            Some("+name and -name entries take exact tool names, not patterns: +a*")
        );
    }
}
