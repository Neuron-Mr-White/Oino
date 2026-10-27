//! Command center model (design.md §4.25): scopes (All / Actions / Sessions / Files),
//! prefixes (`>` actions, `#` sessions, `@` files), fixed sections for the empty query,
//! grouped results with "Show N more", per-workspace history. Fuzzy matching runs over
//! pre-lowered strings without allocating.

/// What picking an item does (resolved by the workspace).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    /// App action by id (`new-session`, `toggle-sidebar`, …).
    App(String),
    /// Send a slash command as a prompt (`/compact`).
    Slash(String),
    /// Invoke an extension shortcut by id.
    Shortcut(String),
    /// Open a session by id.
    Session(String),
    /// Insert / open a file path.
    File(String),
}

/// Result kind (scope membership).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Commands, shortcuts, app actions.
    Action,
    /// Sessions.
    Session,
    /// Files.
    File,
}

/// Fixed sections of the empty query (design.md §4.25 order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// Files changed in the session.
    RecentChanges,
    /// Sessions (attention-first).
    RecentSessions,
    /// "Go to": Sessions, Board, Extensions, Settings, switch sheet (A2.17).
    Suggested,
    /// "Session": new, model, compact, fork, tree, stop.
    Chat,
    /// "View": sidebar, side pane, terminal, review, files.
    Panels,
    /// Extension slash commands and shortcuts.
    Extensions,
    /// Theme, updates, docs.
    App,
}

impl Section {
    /// Header label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RecentChanges => "Recent changes",
            Self::RecentSessions => "Sessions",
            Self::Suggested => "Go to",
            Self::Chat => "Session",
            Self::Panels => "View",
            Self::Extensions => "Extensions",
            Self::App => "App",
        }
    }
}

/// Search scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Everything.
    #[default]
    All,
    /// Actions only.
    Actions,
    /// Sessions only.
    Sessions,
    /// Files only.
    Files,
}

impl Scope {
    /// Tabs in order.
    pub const ALL: [Self; 4] = [Self::All, Self::Actions, Self::Sessions, Self::Files];

    /// Tab label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Actions => "Actions",
            Self::Sessions => "Sessions",
            Self::Files => "Files",
        }
    }

    fn admits(self, kind: Kind) -> bool {
        matches!(
            (self, kind),
            (Self::All, _)
                | (Self::Actions, Kind::Action)
                | (Self::Sessions, Kind::Session)
                | (Self::Files, Kind::File)
        )
    }
}

/// One palette item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// Main label.
    pub label: String,
    /// Secondary text.
    pub detail: Option<String>,
    /// Key hint (`⌘N`).
    pub keys: Option<String>,
    /// Lucide icon file stem.
    pub icon: &'static str,
    /// Section in the empty query.
    pub section: Section,
    /// Kind.
    pub kind: Kind,
    /// Marked current (active session).
    pub current: bool,
    /// What it does.
    pub action: PaletteAction,
    lower: String,
    lower_detail: String,
}

impl PaletteItem {
    /// A new item (pre-lowers its strings).
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        section: Section,
        kind: Kind,
        icon: &'static str,
        action: PaletteAction,
    ) -> Self {
        let label = label.into();
        Self {
            lower: label.to_lowercase(),
            label,
            detail: None,
            keys: None,
            icon,
            section,
            kind,
            current: false,
            action,
            lower_detail: String::new(),
        }
    }

    /// Sets the detail.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let d = detail.into();
        self.lower_detail = d.to_lowercase();
        self.detail = Some(d);
        self
    }

    /// Sets the key hint.
    #[must_use]
    pub fn keys(mut self, keys: impl Into<String>) -> Self {
        self.keys = Some(keys.into());
        self
    }

    /// Marks the item current.
    #[must_use]
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }
}

/// Fuzzy score of lowercase `needle` in lowercase `hay` (subsequence; consecutive and
/// word-start matches score more; shorter haystacks win ties). No allocation.
#[must_use]
pub fn score(needle: &str, hay: &str) -> Option<i64> {
    if needle.is_empty() {
        return Some(0);
    }
    let mut score = 0i64;
    let mut it = hay.chars().enumerate().peekable();
    let mut prev_char: Option<char> = None;
    let mut last_match: Option<usize> = None;
    let mut len = 0usize;
    for n in needle.chars() {
        loop {
            let (i, c) = it.next()?;
            let start = prev_char.is_none_or(|p| !p.is_alphanumeric());
            prev_char = Some(c);
            len = i + 1;
            if c == n {
                score += 1;
                if last_match.is_some_and(|p| p + 1 == i) {
                    score += 4;
                }
                if start {
                    score += 3;
                }
                last_match = Some(i);
                break;
            }
        }
    }
    let rest = it.count();
    let total = i64::try_from(len + rest).unwrap_or(i64::MAX);
    Some(score * 100 - total)
}

/// Indices (char positions) of `needle`'s greedy match in `label` (for highlighting).
#[must_use]
pub fn matched_positions(needle: &str, label: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut chars = label.chars().flat_map(char::to_lowercase).enumerate();
    for n in needle.chars().flat_map(char::to_lowercase) {
        for (i, c) in chars.by_ref() {
            if c == n {
                out.push(i);
                break;
            }
        }
    }
    out
}

/// Splits a query into `(scope from prefix, needle)`.
#[must_use]
pub fn parse_query(query: &str, scope: Scope) -> (Scope, String) {
    let (scope, rest) = match query.chars().next() {
        Some('>') => (Scope::Actions, &query[1..]),
        Some('#') => (Scope::Sessions, &query[1..]),
        Some('@') => (Scope::Files, &query[1..]),
        _ => (scope, query),
    };
    (scope, rest.trim().to_lowercase())
}

/// A rendered palette row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// Section / group header.
    Header(&'static str),
    /// Item index into the item list.
    Item(usize),
    /// "Show N more" for a kind in the All scope.
    More(Kind, usize),
}

/// Rows per group in the All scope.
pub const GROUP_ROWS: usize = 3;

fn group_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Action => "Actions",
        Kind::Session => "Sessions",
        Kind::File => "Files",
    }
}

/// The rows for `query` in `scope` over `items`.
#[must_use]
pub fn rows(items: &[PaletteItem], query: &str, scope: Scope) -> Vec<Row> {
    let (scope, needle) = parse_query(query, scope);
    let mut out = Vec::new();
    if needle.is_empty() {
        if scope == Scope::All {
            let mut sections: Vec<Section> = items.iter().map(|i| i.section).collect();
            sections.sort();
            sections.dedup();
            for section in sections {
                out.push(Row::Header(section.label()));
                out.extend(
                    items
                        .iter()
                        .enumerate()
                        .filter(|(_, i)| i.section == section)
                        .take(GROUP_ROWS)
                        .map(|(ix, _)| Row::Item(ix)),
                );
            }
        } else {
            out.extend(
                items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| scope.admits(i.kind))
                    .map(|(ix, _)| Row::Item(ix)),
            );
        }
        return out;
    }
    for kind in [Kind::Action, Kind::Session, Kind::File] {
        if !scope.admits(kind) {
            continue;
        }
        let mut scored: Vec<(i64, usize)> = items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind == kind)
            .filter_map(|(ix, i)| {
                let best = score(&needle, &i.lower)
                    .max(score(&needle, &i.lower_detail).map(|s| s - 50))?;
                Some((best, ix))
            })
            .collect();
        if scored.is_empty() {
            continue;
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        out.push(Row::Header(group_label(kind)));
        let limit = if scope == Scope::All {
            GROUP_ROWS
        } else {
            usize::MAX
        };
        let more = scored.len().saturating_sub(limit);
        out.extend(scored.into_iter().take(limit).map(|(_, ix)| Row::Item(ix)));
        if more > 0 {
            out.push(Row::More(kind, more));
        }
    }
    out
}

/// Index of the first selectable row at or after `from` (wrapping), stepping `dir`.
#[must_use]
pub fn step(rows: &[Row], from: usize, dir: isize) -> Option<usize> {
    let n = rows.len();
    if n == 0 {
        return None;
    }
    let mut i = from;
    for _ in 0..n {
        i = i.checked_add_signed(dir).map_or(n - 1, |v| v % n);
        if !matches!(rows[i], Row::Header(_)) {
            return Some(i);
        }
    }
    None
}

/// First selectable row.
#[must_use]
pub fn first(rows: &[Row]) -> Option<usize> {
    rows.iter().position(|r| !matches!(r, Row::Header(_)))
}

#[cfg(test)]
mod tests {

    /// design.md §2.11 palette row: filtering 10 000 items stays within one frame (≤ 4 ms in
    /// release; the debug-build bound is looser). `cargo test --release -p oino-ui
    /// palette_10k -- --nocapture` prints the number.
    #[test]
    fn palette_10k() {
        let items: Vec<PaletteItem> = (0..10_000)
            .map(|i| {
                PaletteItem::new(
                    format!("session {i} fix the parser trailing commas"),
                    Section::RecentSessions,
                    Kind::Session,
                    "dot",
                    PaletteAction::Session(i.to_string()),
                )
            })
            .collect();
        let start = std::time::Instant::now();
        let r = rows(&items, "pars com", Scope::All);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        #[allow(clippy::print_stderr, reason = "benchmark output")]
        {
            eprintln!("palette 10k filter: {ms:.2} ms ({} rows)", r.len());
        }
        assert!(!r.is_empty());
        assert!(
            ms < if cfg!(debug_assertions) { 250.0 } else { 16.0 },
            "{ms} ms"
        );
    }

    use super::*;

    fn items() -> Vec<PaletteItem> {
        vec![
            PaletteItem::new(
                "New session",
                Section::Suggested,
                Kind::Action,
                "square-pen",
                PaletteAction::App("new".into()),
            )
            .keys("⌘N"),
            PaletteItem::new(
                "Compact",
                Section::Chat,
                Kind::Action,
                "archive",
                PaletteAction::Slash("compact".into()),
            ),
            PaletteItem::new(
                "Fix parser",
                Section::RecentSessions,
                Kind::Session,
                "dot",
                PaletteAction::Session("s1".into()),
            )
            .detail("json-lite"),
            PaletteItem::new(
                "Fix lexer",
                Section::RecentSessions,
                Kind::Session,
                "dot",
                PaletteAction::Session("s2".into()),
            ),
            PaletteItem::new(
                "Fix docs",
                Section::RecentSessions,
                Kind::Session,
                "dot",
                PaletteAction::Session("s3".into()),
            ),
            PaletteItem::new(
                "Fix tests",
                Section::RecentSessions,
                Kind::Session,
                "dot",
                PaletteAction::Session("s4".into()),
            ),
            PaletteItem::new(
                "src/parser.rs",
                Section::RecentChanges,
                Kind::File,
                "file",
                PaletteAction::File("src/parser.rs".into()),
            ),
        ]
    }

    #[test]
    fn scoring_prefers_word_starts_and_runs() {
        assert!(score("ns", "new session") > score("ns", "transcript names"));
        assert!(score("new", "new session") > score("new", "renew"));
        assert_eq!(score("zz", "new session"), None);
        assert_eq!(score("", "x"), Some(0));
        assert_eq!(matched_positions("ns", "New session"), vec![0, 4]);
    }

    #[test]
    fn empty_query_shows_sections_in_order() {
        let items = items();
        let r = rows(&items, "", Scope::All);
        assert_eq!(r[0], Row::Header("Recent changes"));
        assert_eq!(r[1], Row::Item(6));
        assert_eq!(r[2], Row::Header("Sessions"));
        assert_eq!(
            r.iter()
                .filter(|x| matches!(x, Row::Item(i) if items[*i].kind == Kind::Session))
                .count(),
            3
        );
        let sessions = rows(&items, "", Scope::Sessions);
        assert_eq!(sessions.len(), 4);
    }

    #[test]
    fn queries_group_by_kind_with_more() {
        let items = items();
        let r = rows(&items, "fix", Scope::All);
        assert_eq!(r[0], Row::Header("Sessions"));
        assert_eq!(r.last(), Some(&Row::More(Kind::Session, 1)));
        let scoped = rows(&items, "#fix", Scope::All);
        assert_eq!(
            scoped.iter().filter(|x| matches!(x, Row::Item(_))).count(),
            4
        );
        let actions = rows(&items, ">comp", Scope::All);
        assert_eq!(actions, vec![Row::Header("Actions"), Row::Item(1)]);
        let files = rows(&items, "@parser", Scope::All);
        assert_eq!(files[1], Row::Item(6));
        assert_eq!(
            rows(&items, "json", Scope::Sessions)[1],
            Row::Item(2),
            "detail matches"
        );
        assert_eq!(first(&r), Some(1));
        assert_eq!(step(&r, 1, 1), Some(2));
        assert_eq!(step(&r, 1, -1), Some(r.len() - 1));
    }
}
