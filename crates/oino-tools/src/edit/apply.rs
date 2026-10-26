//! Matching and applying edits (pi `tools/edit-diff.ts`).

use unicode_normalization::UnicodeNormalization;

use crate::js;

/// One replacement (pi `Edit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Text to find.
    pub old_text: String,
    /// Replacement.
    pub new_text: String,
}

/// Line ending of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineEnding {
    Lf,
    Crlf,
}

/// pi `detectLineEnding`: CRLF when the first `\r\n` comes before the first bare `\n`.
pub(crate) fn detect_line_ending(content: &str) -> LineEnding {
    match (content.find("\r\n"), content.find('\n')) {
        (Some(crlf), Some(lf)) if crlf < lf => LineEnding::Crlf,
        _ => LineEnding::Lf,
    }
}

/// pi `normalizeToLF`.
pub(crate) fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// pi `restoreLineEndings`.
pub(crate) fn restore_line_endings(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Crlf => text.replace('\n', "\r\n"),
        LineEnding::Lf => text.to_owned(),
    }
}

/// pi `normalizeForFuzzyMatch`: NFKC, trailing whitespace per line, smart quotes, Unicode
/// dashes and special spaces folded to ASCII.
pub(crate) fn normalize_for_fuzzy_match(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    let trimmed = nfkc
        .split('\n')
        .map(js::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    trimmed
        .chars()
        .map(|ch| match ch {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

/// A match of one edit in the replacement base (byte offsets).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Replacement {
    edit_index: usize,
    start: usize,
    len: usize,
    new_text: String,
}

/// pi `fuzzyFindText`.
struct FuzzyMatch {
    index: Option<usize>,
    len: usize,
    used_fuzzy: bool,
}

fn fuzzy_find(content: &str, old_text: &str) -> FuzzyMatch {
    if let Some(index) = content.find(old_text) {
        return FuzzyMatch {
            index: Some(index),
            len: old_text.len(),
            used_fuzzy: false,
        };
    }
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    match fuzzy_content.find(&fuzzy_old) {
        Some(index) => FuzzyMatch {
            index: Some(index),
            len: fuzzy_old.len(),
            used_fuzzy: true,
        },
        None => FuzzyMatch {
            index: None,
            len: 0,
            used_fuzzy: false,
        },
    }
}

/// pi `countOccurrences`: `content.split(oldText).length - 1` in fuzzy space.
fn count_occurrences(content: &str, old_text: &str) -> i64 {
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    if fuzzy_old.is_empty() {
        // JavaScript splits into UTF-16 units.
        return i64::try_from(js::utf16_len(&fuzzy_content)).unwrap_or(i64::MAX) - 1;
    }
    i64::try_from(fuzzy_content.matches(fuzzy_old.as_str()).count()).unwrap_or(i64::MAX)
}

/// Why edits could not be applied; the texts are pi's.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// An `oldText` is empty.
    #[error("{}", if *.total == 1 { format!("oldText must not be empty in {}.", .path) } else { format!("edits[{}].oldText must not be empty in {}.", .index, .path) })]
    EmptyOldText {
        /// File path as given.
        path: String,
        /// Edit index.
        index: usize,
        /// Number of edits.
        total: usize,
    },
    /// An `oldText` was not found.
    #[error("{}", if *.total == 1 { format!("Could not find the exact text in {}. The old text must match exactly including all whitespace and newlines.", .path) } else { format!("Could not find edits[{}] in {}. The oldText must match exactly including all whitespace and newlines.", .index, .path) })]
    NotFound {
        /// File path as given.
        path: String,
        /// Edit index.
        index: usize,
        /// Number of edits.
        total: usize,
    },
    /// An `oldText` occurs more than once.
    #[error("{}", if *.total == 1 { format!("Found {} occurrences of the text in {}. The text must be unique. Please provide more context to make it unique.", .occurrences, .path) } else { format!("Found {} occurrences of edits[{}] in {}. Each oldText must be unique. Please provide more context to make it unique.", .occurrences, .index, .path) })]
    Duplicate {
        /// File path as given.
        path: String,
        /// Edit index.
        index: usize,
        /// Number of edits.
        total: usize,
        /// Occurrences found.
        occurrences: i64,
    },
    /// Two edits overlap.
    #[error(
        "edits[{first}] and edits[{second}] overlap in {path}. Merge them into one edit or target disjoint regions."
    )]
    Overlap {
        /// File path as given.
        path: String,
        /// Earlier edit.
        first: usize,
        /// Later edit.
        second: usize,
    },
    /// The result equals the input.
    #[error("{}", if *.total == 1 { format!("No changes made to {}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected.", .path) } else { format!("No changes made to {}. The replacements produced identical content.", .path) })]
    NoChange {
        /// File path as given.
        path: String,
        /// Number of edits.
        total: usize,
    },
    /// Fuzzy normalisation changed the line structure.
    #[error("Cannot preserve unchanged lines because the base content has a different line count.")]
    LineCountMismatch,
    /// A replacement fell outside the base content.
    #[error("Replacement range is outside the base content.")]
    OutOfRange,
}

/// pi `applyReplacements` (from the end, offsets relative to `offset`).
fn apply_replacements(content: &str, replacements: &[Replacement], offset: usize) -> String {
    let mut result = content.to_owned();
    for replacement in replacements.iter().rev() {
        let start = replacement.start - offset;
        result.replace_range(start..start + replacement.len, &replacement.new_text);
    }
    result
}

/// pi `splitLinesWithEndings` as byte spans.
fn line_spans(content: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            spans.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < content.len() {
        spans.push((start, content.len()));
    }
    spans
}

fn replacement_line_range(
    lines: &[(usize, usize)],
    replacement: &Replacement,
) -> Result<(usize, usize), EditError> {
    let start = replacement.start;
    let end = replacement.start + replacement.len;
    let start_line = lines
        .iter()
        .position(|(line_start, line_end)| start >= *line_start && start < *line_end)
        .ok_or(EditError::OutOfRange)?;
    let mut end_line = start_line;
    while end_line < lines.len() && lines[end_line].1 < end {
        end_line += 1;
    }
    if end_line >= lines.len() {
        return Err(EditError::OutOfRange);
    }
    Ok((start_line, end_line + 1))
}

/// pi `applyReplacementsPreservingUnchangedLines`: replacements matched in `base` rewrite only
/// the lines they touch; other lines are copied from `original`.
fn apply_preserving_unchanged_lines(
    original: &str,
    base: &str,
    replacements: &[Replacement],
) -> Result<String, EditError> {
    let original_lines: Vec<&str> = line_spans(original)
        .into_iter()
        .map(|(start, end)| &original[start..end])
        .collect();
    let base_lines = line_spans(base);
    if original_lines.len() != base_lines.len() {
        return Err(EditError::LineCountMismatch);
    }
    let mut sorted = replacements.to_vec();
    sorted.sort_by_key(|replacement| replacement.start);
    let mut groups: Vec<(usize, usize, Vec<Replacement>)> = Vec::new();
    for replacement in sorted {
        let (start_line, end_line) = replacement_line_range(&base_lines, &replacement)?;
        if let Some(current) = groups.last_mut()
            && start_line < current.1
        {
            current.1 = current.1.max(end_line);
            current.2.push(replacement);
            continue;
        }
        groups.push((start_line, end_line, vec![replacement]));
    }
    let mut result = String::new();
    let mut original_index = 0;
    for (start_line, end_line, group) in &groups {
        result.push_str(&original_lines[original_index..*start_line].concat());
        let group_start = base_lines[*start_line].0;
        let group_end = base_lines[end_line - 1].1;
        result.push_str(&apply_replacements(
            &base[group_start..group_end],
            group,
            group_start,
        ));
        original_index = *end_line;
    }
    result.push_str(&original_lines[original_index..].concat());
    Ok(result)
}

/// pi `AppliedEditsResult`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppliedEdits {
    pub(crate) base_content: String,
    pub(crate) new_content: String,
}

/// pi `applyEditsToNormalizedContent`.
pub(crate) fn apply_edits_to_normalized_content(
    normalized: &str,
    edits: &[Edit],
    path: &str,
) -> Result<AppliedEdits, EditError> {
    let total = edits.len();
    let edits: Vec<Edit> = edits
        .iter()
        .map(|edit| Edit {
            old_text: normalize_to_lf(&edit.old_text),
            new_text: normalize_to_lf(&edit.new_text),
        })
        .collect();
    if let Some(index) = edits.iter().position(|edit| edit.old_text.is_empty()) {
        return Err(EditError::EmptyOldText {
            path: path.to_owned(),
            index,
            total,
        });
    }
    let used_fuzzy = edits
        .iter()
        .any(|edit| fuzzy_find(normalized, &edit.old_text).used_fuzzy);
    let base = if used_fuzzy {
        normalize_for_fuzzy_match(normalized)
    } else {
        normalized.to_owned()
    };
    let mut matched = Vec::with_capacity(total);
    for (index, edit) in edits.iter().enumerate() {
        let found = fuzzy_find(&base, &edit.old_text);
        let Some(start) = found.index else {
            return Err(EditError::NotFound {
                path: path.to_owned(),
                index,
                total,
            });
        };
        let occurrences = count_occurrences(&base, &edit.old_text);
        if occurrences > 1 {
            return Err(EditError::Duplicate {
                path: path.to_owned(),
                index,
                total,
                occurrences,
            });
        }
        matched.push(Replacement {
            edit_index: index,
            start,
            len: found.len,
            new_text: edit.new_text.clone(),
        });
    }
    matched.sort_by_key(|replacement| replacement.start);
    for pair in matched.windows(2) {
        if pair[0].start + pair[0].len > pair[1].start {
            return Err(EditError::Overlap {
                path: path.to_owned(),
                first: pair[0].edit_index,
                second: pair[1].edit_index,
            });
        }
    }
    let new_content = if used_fuzzy {
        apply_preserving_unchanged_lines(normalized, &base, &matched)?
    } else {
        apply_replacements(&base, &matched, 0)
    };
    if new_content == normalized {
        return Err(EditError::NoChange {
            path: path.to_owned(),
            total,
        });
    }
    Ok(AppliedEdits {
        base_content: normalized.to_owned(),
        new_content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(old: &str, new: &str) -> Edit {
        Edit {
            old_text: old.into(),
            new_text: new.into(),
        }
    }

    #[test]
    fn exact_replacement() {
        let result =
            apply_edits_to_normalized_content("a\nb\nc\n", &[edit("b", "x")], "f").unwrap();
        assert_eq!(result.new_content, "a\nx\nc\n");
    }

    #[test]
    fn fuzzy_keeps_untouched_lines() {
        let content = "keep \u{201C}q\u{201D}  \nchange me  \n";
        let result = apply_edits_to_normalized_content(content, &[edit("change me", "done")], "f");
        // Exact match exists ("change me"), so no fuzzy mode.
        assert_eq!(
            result.unwrap().new_content,
            "keep \u{201C}q\u{201D}  \ndone  \n"
        );
        let result =
            apply_edits_to_normalized_content(content, &[edit("change me\n", "done\n")], "f")
                .unwrap();
        assert_eq!(result.new_content, "keep \u{201C}q\u{201D}  \ndone\n");
    }

    #[test]
    fn errors_match_pi() {
        let error = apply_edits_to_normalized_content("aa", &[edit("a", "b")], "f").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Found 2 occurrences of the text in f. The text must be unique. Please provide more context to make it unique."
        );
        let error =
            apply_edits_to_normalized_content("ab", &[edit("a", "x"), edit("ab", "y")], "f")
                .unwrap_err();
        assert_eq!(
            error.to_string(),
            "edits[0] and edits[1] overlap in f. Merge them into one edit or target disjoint regions."
        );
    }

    #[test]
    fn line_endings() {
        assert_eq!(detect_line_ending("a\r\nb\n"), LineEnding::Crlf);
        assert_eq!(detect_line_ending("a\nb\r\n"), LineEnding::Lf);
        assert_eq!(normalize_to_lf("a\r\nb\rc"), "a\nb\nc");
    }
}
