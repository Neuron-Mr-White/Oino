//! Line diffs exactly as pi produces them: a port of jsdiff 8.0.4 `diffLines` (Myers with the
//! diagonal pruning of `base.js`) and `createTwoFilesPatch`, plus pi's display diff
//! (`generateDiffString` in `tools/edit-diff.ts`).

use std::collections::HashMap;

/// One change object of `diffLines` (`{value, count, added, removed}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) value: String,
    pub(crate) count: usize,
    pub(crate) added: bool,
    pub(crate) removed: bool,
}

/// jsdiff `tokenize` for lines: each token is a line with its `\n` or `\r\n`.
fn tokenize(value: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let separator = if bytes[index] == b'\n' {
            1
        } else if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            2
        } else {
            0
        };
        if separator == 0 {
            index += 1;
            continue;
        }
        tokens.push(&value[start..index + separator]);
        index += separator;
        start = index;
    }
    if start < value.len() {
        tokens.push(&value[start..]);
    }
    tokens
}

#[derive(Clone, Copy)]
struct Component {
    count: usize,
    added: bool,
    removed: bool,
    previous: Option<usize>,
}

#[derive(Clone, Copy)]
struct Path {
    old_pos: i64,
    last: Option<usize>,
}

struct Myers<'a> {
    old: Vec<&'a str>,
    new: Vec<&'a str>,
    components: Vec<Component>,
}

fn signed(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn index(value: i64) -> usize {
    usize::try_from(value).unwrap_or(0)
}

impl Myers<'_> {
    fn push(&mut self, component: Component) -> usize {
        self.components.push(component);
        self.components.len() - 1
    }

    fn add_to_path(&mut self, path: Path, added: bool, removed: bool, old_inc: i64) -> Path {
        let component = match path.last.map(|last| self.components[last]) {
            Some(last) if last.added == added && last.removed == removed => Component {
                count: last.count + 1,
                added,
                removed,
                previous: last.previous,
            },
            _ => Component {
                count: 1,
                added,
                removed,
                previous: path.last,
            },
        };
        Path {
            old_pos: path.old_pos + old_inc,
            last: Some(self.push(component)),
        }
    }

    fn extract_common(&mut self, path: &mut Path, diagonal: i64) -> i64 {
        let (new_len, old_len) = (signed(self.new.len()), signed(self.old.len()));
        let mut old_pos = path.old_pos;
        let mut new_pos = old_pos - diagonal;
        let mut common = 0;
        while new_pos + 1 < new_len
            && old_pos + 1 < old_len
            && self.old[index(old_pos + 1)] == self.new[index(new_pos + 1)]
        {
            new_pos += 1;
            old_pos += 1;
            common += 1;
        }
        if common > 0 {
            path.last = Some(self.push(Component {
                count: common,
                added: false,
                removed: false,
                previous: path.last,
            }));
        }
        path.old_pos = old_pos;
        new_pos
    }

    fn build_values(&self, last: Option<usize>) -> Vec<Change> {
        let mut chain = Vec::new();
        let mut current = last;
        while let Some(at) = current {
            chain.push(self.components[at]);
            current = self.components[at].previous;
        }
        chain.reverse();
        let (mut new_pos, mut old_pos) = (0, 0);
        chain
            .into_iter()
            .map(|component| {
                let value = if component.removed {
                    let value = self.old[old_pos..old_pos + component.count].concat();
                    old_pos += component.count;
                    value
                } else {
                    let value = self.new[new_pos..new_pos + component.count].concat();
                    new_pos += component.count;
                    if !component.added {
                        old_pos += component.count;
                    }
                    value
                };
                Change {
                    value,
                    count: component.count,
                    added: component.added,
                    removed: component.removed,
                }
            })
            .collect()
    }

    fn run(mut self) -> Vec<Change> {
        let (new_len, old_len) = (signed(self.new.len()), signed(self.old.len()));
        let max_edit_length = new_len + old_len;
        let mut best: HashMap<i64, Path> = HashMap::new();
        let mut seed = Path {
            old_pos: -1,
            last: None,
        };
        let new_pos = self.extract_common(&mut seed, 0);
        if seed.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
            return self.build_values(seed.last);
        }
        best.insert(0, seed);
        let (mut min_diagonal, mut max_diagonal) = (i64::MIN, i64::MAX);
        let mut edit_length = 1;
        while edit_length <= max_edit_length {
            let mut diagonal = min_diagonal.max(-edit_length);
            while diagonal <= max_diagonal.min(edit_length) {
                let remove_path = best.remove(&(diagonal - 1));
                let add_path = best.get(&(diagonal + 1)).copied();
                let can_add = add_path.is_some_and(|path| {
                    let add_new_pos = path.old_pos - diagonal;
                    0 <= add_new_pos && add_new_pos < new_len
                });
                let can_remove = remove_path.is_some_and(|path| path.old_pos + 1 < old_len);
                if !can_add && !can_remove {
                    best.remove(&diagonal);
                    diagonal += 2;
                    continue;
                }
                let use_add = !can_remove
                    || (can_add
                        && remove_path
                            .zip(add_path)
                            .is_some_and(|(remove, add)| remove.old_pos < add.old_pos));
                let chosen = if use_add { add_path } else { remove_path };
                let Some(chosen) = chosen else {
                    diagonal += 2;
                    continue;
                };
                let mut base = if use_add {
                    self.add_to_path(chosen, true, false, 0)
                } else {
                    self.add_to_path(chosen, false, true, 1)
                };
                let new_pos = self.extract_common(&mut base, diagonal);
                if base.old_pos + 1 >= old_len && new_pos + 1 >= new_len {
                    return self.build_values(base.last);
                }
                best.insert(diagonal, base);
                if base.old_pos + 1 >= old_len {
                    max_diagonal = max_diagonal.min(diagonal - 1);
                }
                if new_pos + 1 >= new_len {
                    min_diagonal = min_diagonal.max(diagonal + 1);
                }
                diagonal += 2;
            }
            edit_length += 1;
        }
        Vec::new()
    }
}

/// jsdiff `diffLines(old, new)`.
#[must_use]
pub(crate) fn diff_lines(old: &str, new: &str) -> Vec<Change> {
    Myers {
        old: tokenize(old),
        new: tokenize(new),
        components: Vec::new(),
    }
    .run()
}

/// jsdiff `splitLines`: lines keeping their trailing `\n`.
fn split_lines_keep(text: &str) -> Vec<String> {
    let has_trailing = text.ends_with('\n');
    let mut lines: Vec<String> = text.split('\n').map(|line| format!("{line}\n")).collect();
    if has_trailing {
        lines.pop();
    } else if let Some(last) = lines.last_mut() {
        last.pop();
    }
    lines
}

struct Hunk {
    old_start: usize,
    old_lines: usize,
    new_start: usize,
    new_lines: usize,
    lines: Vec<String>,
}

/// jsdiff `structuredPatch` hunks.
fn hunks(old: &str, new: &str, context: usize) -> Vec<Hunk> {
    let mut parts: Vec<(Change, Vec<String>)> = diff_lines(old, new)
        .into_iter()
        .map(|change| {
            let lines = split_lines_keep(&change.value);
            (change, lines)
        })
        .collect();
    parts.push((
        Change {
            value: String::new(),
            count: 0,
            added: false,
            removed: false,
        },
        Vec::new(),
    ));
    let prefixed = |lines: &[String]| {
        lines
            .iter()
            .map(|line| format!(" {line}"))
            .collect::<Vec<_>>()
    };
    let mut hunks = Vec::new();
    let (mut old_range_start, mut new_range_start) = (0_usize, 0_usize);
    let mut current: Vec<String> = Vec::new();
    let (mut old_line, mut new_line) = (1_usize, 1_usize);
    for at in 0..parts.len() {
        let (change, lines) = &parts[at];
        if change.added || change.removed {
            if old_range_start == 0 {
                old_range_start = old_line;
                new_range_start = new_line;
                if at > 0 {
                    let previous = &parts[at - 1].1;
                    current = if context > 0 {
                        prefixed(&previous[previous.len().saturating_sub(context)..])
                    } else {
                        Vec::new()
                    };
                    old_range_start -= current.len();
                    new_range_start -= current.len();
                }
            }
            let sign = if change.added { '+' } else { '-' };
            current.extend(lines.iter().map(|line| format!("{sign}{line}")));
            if change.added {
                new_line += lines.len();
            } else {
                old_line += lines.len();
            }
        } else {
            if old_range_start != 0 {
                if lines.len() <= context * 2 && at + 2 < parts.len() {
                    current.extend(prefixed(lines));
                } else {
                    let size = lines.len().min(context);
                    current.extend(prefixed(&lines[..size]));
                    hunks.push(Hunk {
                        old_start: old_range_start,
                        old_lines: old_line - old_range_start + size,
                        new_start: new_range_start,
                        new_lines: new_line - new_range_start + size,
                        lines: std::mem::take(&mut current),
                    });
                    old_range_start = 0;
                    new_range_start = 0;
                }
            }
            old_line += lines.len();
            new_line += lines.len();
        }
    }
    for hunk in &mut hunks {
        let mut lines = Vec::with_capacity(hunk.lines.len());
        for line in hunk.lines.drain(..) {
            if let Some(stripped) = line.strip_suffix('\n') {
                lines.push(stripped.to_owned());
            } else {
                lines.push(line);
                lines.push("\\ No newline at end of file".to_owned());
            }
        }
        hunk.lines = lines;
    }
    hunks
}

/// pi `generateUnifiedPatch`: jsdiff `createTwoFilesPatch(path, path, old, new, undefined,
/// undefined, {context: 4, headerOptions: FILE_HEADERS_ONLY})`.
#[must_use]
pub(crate) fn unified_patch(path: &str, old: &str, new: &str) -> String {
    let mut out = vec![format!("--- {path}"), format!("+++ {path}")];
    for hunk in hunks(old, new, 4) {
        let old_start = if hunk.old_lines == 0 {
            hunk.old_start - 1
        } else {
            hunk.old_start
        };
        let new_start = if hunk.new_lines == 0 {
            hunk.new_start - 1
        } else {
            hunk.new_start
        };
        out.push(format!(
            "@@ -{old_start},{} +{new_start},{} @@",
            hunk.old_lines, hunk.new_lines
        ));
        out.extend(hunk.lines);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// pi `generateDiffString` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DisplayDiff {
    pub(crate) diff: String,
    pub(crate) first_changed_line: Option<usize>,
}

/// Accumulates pi's display diff lines and the old/new line counters.
struct DiffWriter {
    output: Vec<String>,
    old_line: usize,
    new_line: usize,
    width: usize,
}

impl DiffWriter {
    fn line(&mut self, prefix: char, number: usize, line: &str) {
        let width = self.width;
        self.output.push(format!("{prefix}{number:>width$} {line}"));
    }

    fn context(&mut self, lines: &[&str]) {
        for line in lines {
            self.line(' ', self.old_line, line);
            self.old_line += 1;
            self.new_line += 1;
        }
    }

    /// `...` marker for `count` hidden context lines (none when `count` is 0).
    fn skip(&mut self, count: usize) {
        if count > 0 {
            self.output.push(format!(" {} ...", " ".repeat(self.width)));
            self.old_line += count;
            self.new_line += count;
        }
    }

    /// An unchanged block between (optional) changes, trimmed to `CONTEXT` lines per side.
    fn unchanged(&mut self, raw: &[&str], after_change: bool, before_change: bool) {
        const CONTEXT: usize = 4;
        match (after_change, before_change) {
            (true, true) if raw.len() <= CONTEXT * 2 => self.context(raw),
            (true, true) => {
                self.context(&raw[..CONTEXT]);
                self.output.push(format!(" {} ...", " ".repeat(self.width)));
                let skipped = raw.len() - 2 * CONTEXT;
                self.old_line += skipped;
                self.new_line += skipped;
                self.context(&raw[raw.len() - CONTEXT..]);
            }
            (true, false) => {
                let shown = raw.len().min(CONTEXT);
                self.context(&raw[..shown]);
                self.skip(raw.len() - shown);
            }
            (false, true) => {
                let skipped = raw.len().saturating_sub(CONTEXT);
                self.skip(skipped);
                self.context(&raw[skipped..]);
            }
            (false, false) => {
                self.old_line += raw.len();
                self.new_line += raw.len();
            }
        }
    }
}

/// pi `generateDiffString(old, new, 4)`: numbered `+`/`-`/context lines with `...` gaps.
#[must_use]
pub(crate) fn display_diff(old: &str, new: &str) -> DisplayDiff {
    let parts = diff_lines(old, new);
    let width = old
        .split('\n')
        .count()
        .max(new.split('\n').count())
        .to_string()
        .len();
    let mut writer = DiffWriter {
        output: Vec::new(),
        old_line: 1,
        new_line: 1,
        width,
    };
    let mut last_was_change = false;
    let mut first_changed_line = None;
    for (at, part) in parts.iter().enumerate() {
        let mut raw: Vec<&str> = part.value.split('\n').collect();
        if raw.last() == Some(&"") {
            raw.pop();
        }
        if part.added || part.removed {
            first_changed_line.get_or_insert(writer.new_line);
            for line in raw {
                if part.added {
                    writer.line('+', writer.new_line, line);
                    writer.new_line += 1;
                } else {
                    writer.line('-', writer.old_line, line);
                    writer.old_line += 1;
                }
            }
            last_was_change = true;
            continue;
        }
        let next_is_change = parts
            .get(at + 1)
            .is_some_and(|next| next.added || next.removed);
        writer.unchanged(&raw, last_was_change, next_is_change);
        last_was_change = false;
    }
    DisplayDiff {
        diff: writer.output.join("\n"),
        first_changed_line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_lines_with_separators() {
        assert_eq!(tokenize("a\nb\r\nc"), vec!["a\n", "b\r\n", "c"]);
        assert_eq!(tokenize(""), Vec::<&str>::new());
    }

    #[test]
    fn diff_lines_basic() {
        let changes = diff_lines("a\nb\nc\n", "a\nx\nc\n");
        let summary: Vec<(String, bool, bool)> = changes
            .into_iter()
            .map(|c| (c.value, c.added, c.removed))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("a\n".into(), false, false),
                ("b\n".into(), false, true),
                ("x\n".into(), true, false),
                ("c\n".into(), false, false),
            ]
        );
    }

    #[test]
    fn unified_patch_marks_missing_newline() {
        let patch = unified_patch("f", "a\nb", "a\nc");
        assert_eq!(
            patch,
            "--- f\n+++ f\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn display_diff_numbers_lines() {
        let diff = display_diff("a\nb\nc", "a\nx\nc");
        assert_eq!(diff.diff, " 1 a\n-2 b\n+2 x\n 3 c");
        assert_eq!(diff.first_changed_line, Some(2));
    }
}
