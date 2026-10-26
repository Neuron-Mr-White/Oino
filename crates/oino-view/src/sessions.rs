//! Session resume picker model (pi `session-selector.ts`).
//!
//! Scope (current folder / all), sort (threaded / recent / fuzzy), name filter (all /
//! named), path display, search and delete confirmation. Threaded order nests forks under
//! their parent session (`parentSessionPath`).

use std::collections::HashMap;

/// One saved session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionItem {
    /// File path.
    pub path: String,
    /// Session id.
    pub id: String,
    /// Working directory.
    pub cwd: String,
    /// Display name.
    pub name: Option<String>,
    /// Parent session file.
    pub parent_path: Option<String>,
    /// Last activity (ms since epoch).
    pub modified_ms: i64,
    /// Message entries.
    pub message_count: usize,
    /// First user message.
    pub first_message: String,
    /// Search corpus.
    pub all_text: String,
}

/// Which sessions are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Sessions of the current cwd.
    #[default]
    Current,
    /// Every session.
    All,
}

/// Sort mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortMode {
    /// Forks nested under parents, newest first.
    #[default]
    Threaded,
    /// Newest first, flat.
    Recent,
    /// Fuzzy relevance (when searching).
    Fuzzy,
}

impl SortMode {
    /// Header label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Threaded => "Threaded",
            Self::Recent => "Recent",
            Self::Fuzzy => "Fuzzy",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Threaded => Self::Recent,
            Self::Recent => Self::Fuzzy,
            Self::Fuzzy => Self::Threaded,
        }
    }
}

/// A visible row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRow {
    /// Index into the item list.
    pub index: usize,
    /// Tree prefix (`│  ├─ `).
    pub prefix: String,
}

/// The picker model.
#[derive(Clone, Debug, Default)]
pub struct SessionListModel {
    items: Vec<SessionItem>,
    rows: Vec<SessionRow>,
    /// Scope.
    pub scope: Scope,
    /// Sort mode.
    pub sort: SortMode,
    /// Named-only filter.
    pub named_only: bool,
    /// Show file paths.
    pub show_path: bool,
    /// Search query.
    pub query: String,
    /// Path awaiting delete confirmation.
    pub confirm_delete: Option<String>,
    /// Currently open session file.
    pub current_path: Option<String>,
    /// Items are loading.
    pub loading: bool,
    selected: usize,
}

impl SessionListModel {
    /// A loading model.
    #[must_use]
    pub fn new(scope: Scope, current_path: Option<String>) -> Self {
        Self {
            scope,
            current_path,
            loading: true,
            ..Self::default()
        }
    }

    /// Replaces the items (newest first expected).
    pub fn set_items(&mut self, items: Vec<SessionItem>) {
        self.items = items;
        self.loading = false;
        self.refresh();
    }

    /// Item by index.
    #[must_use]
    pub fn item(&self, index: usize) -> Option<&SessionItem> {
        self.items.get(index)
    }

    /// Visible rows.
    #[must_use]
    pub fn rows(&self) -> &[SessionRow] {
        &self.rows
    }

    /// Selected row index.
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Selected session.
    #[must_use]
    pub fn selected_item(&self) -> Option<&SessionItem> {
        self.rows
            .get(self.selected)
            .and_then(|r| self.items.get(r.index))
    }

    /// Moves the selection (wrapping).
    pub fn move_by(&mut self, delta: isize) {
        let len = self.rows.len();
        if len == 0 {
            return;
        }
        let len_i = isize::try_from(len).unwrap_or(isize::MAX);
        let next = (isize::try_from(self.selected).unwrap_or(0) + delta).rem_euclid(len_i);
        self.selected = usize::try_from(next).unwrap_or(0);
    }

    /// Sets the query (re-filters).
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.refresh();
    }

    /// `app.session.toggleSort`.
    pub fn toggle_sort(&mut self) {
        self.sort = self.sort.next();
        self.refresh();
    }

    /// `app.session.toggleNamedFilter`.
    pub fn toggle_named(&mut self) {
        self.named_only = !self.named_only;
        self.refresh();
    }

    /// Removes a deleted session.
    pub fn remove(&mut self, path: &str) {
        self.items.retain(|i| i.path != path);
        self.refresh();
    }

    /// Updates a renamed session.
    pub fn rename(&mut self, path: &str, name: Option<String>) {
        if let Some(item) = self.items.iter_mut().find(|i| i.path == path) {
            item.name = name;
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        let candidates: Vec<usize> = (0..self.items.len())
            .filter(|&i| !self.named_only || self.items[i].name.is_some())
            .collect();
        let query = self.query.trim();
        self.rows = if self.sort == SortMode::Threaded && query.is_empty() {
            self.threaded(&candidates)
        } else if query.is_empty() {
            let mut order = candidates;
            order.sort_by_key(|&i| std::cmp::Reverse(self.items[i].modified_ms));
            order
                .into_iter()
                .map(|index| SessionRow {
                    index,
                    prefix: String::new(),
                })
                .collect()
        } else {
            let texts: Vec<String> = candidates
                .iter()
                .map(|&i| {
                    let item = &self.items[i];
                    format!(
                        "{} {} {} {}",
                        item.name.as_deref().unwrap_or(""),
                        item.first_message,
                        item.all_text,
                        item.cwd
                    )
                })
                .collect();
            let mut order = crate::fuzzy::filter(&texts, query, Clone::clone);
            if self.sort != SortMode::Fuzzy {
                order.sort_by_key(|&o| std::cmp::Reverse(self.items[candidates[o]].modified_ms));
            }
            order
                .into_iter()
                .map(|o| SessionRow {
                    index: candidates[o],
                    prefix: String::new(),
                })
                .collect()
        };
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }

    fn threaded(&self, candidates: &[usize]) -> Vec<SessionRow> {
        let by_path: HashMap<&str, usize> = candidates
            .iter()
            .map(|&i| (self.items[i].path.as_str(), i))
            .collect();
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut roots = Vec::new();
        for &i in candidates {
            match self.items[i]
                .parent_path
                .as_deref()
                .and_then(|p| by_path.get(p))
            {
                Some(&parent) if parent != i => children.entry(parent).or_default().push(i),
                _ => roots.push(i),
            }
        }
        let newest =
            |v: &mut Vec<usize>| v.sort_by_key(|&i| std::cmp::Reverse(self.items[i].modified_ms));
        newest(&mut roots);
        for list in children.values_mut() {
            newest(list);
        }
        let mut rows = Vec::new();
        let mut stack: Vec<(usize, usize, Vec<bool>, bool)> = roots
            .iter()
            .rev()
            .map(|&r| (r, 0, Vec::new(), true))
            .collect();
        let mut seen = std::collections::HashSet::new();
        while let Some((index, depth, ancestors, is_last)) = stack.pop() {
            if !seen.insert(index) {
                continue;
            }
            let prefix = if depth == 0 {
                String::new()
            } else {
                let mut p: String = ancestors
                    .iter()
                    .map(|&continues| if continues { "│  " } else { "   " })
                    .collect();
                p.push_str(if is_last { "└─ " } else { "├─ " });
                p
            };
            rows.push(SessionRow { index, prefix });
            if let Some(kids) = children.get(&index) {
                let mut next_ancestors = ancestors.clone();
                if depth > 0 {
                    next_ancestors.push(!is_last);
                }
                for (k, &child) in kids.iter().enumerate().rev() {
                    stack.push((
                        child,
                        depth + 1,
                        next_ancestors.clone(),
                        k == kids.len() - 1,
                    ));
                }
            }
        }
        rows
    }

    /// Header title (pi).
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self.scope {
            Scope::Current => "Resume Session (Current Folder)",
            Scope::All => "Resume Session (All)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, parent: Option<&str>, modified: i64, name: Option<&str>) -> SessionItem {
        SessionItem {
            path: path.into(),
            id: path.into(),
            parent_path: parent.map(str::to_owned),
            modified_ms: modified,
            name: name.map(str::to_owned),
            first_message: format!("msg {path}"),
            ..SessionItem::default()
        }
    }

    #[test]
    fn threaded_named_and_search() {
        let mut model = SessionListModel::new(Scope::Current, None);
        model.set_items(vec![
            item("a", None, 10, None),
            item("b", Some("a"), 30, Some("fork")),
            item("c", None, 20, None),
        ]);
        let order: Vec<(&str, &str)> = model
            .rows()
            .iter()
            .map(|r| {
                (
                    model.item(r.index).unwrap().path.as_str(),
                    r.prefix.as_str(),
                )
            })
            .collect();
        assert_eq!(order, vec![("c", ""), ("a", ""), ("b", "└─ ")]);
        model.toggle_named();
        assert_eq!(model.rows().len(), 1);
        model.toggle_named();
        model.toggle_sort();
        assert_eq!(model.sort, SortMode::Recent);
        assert_eq!(model.selected_item().unwrap().path, "b");
        model.set_query("msg c");
        assert_eq!(model.rows().len(), 1);
    }
}
