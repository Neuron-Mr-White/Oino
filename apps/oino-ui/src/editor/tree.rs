//! The lazy directory model behind the Files page's tree (round5 #24).
//!
//! `gpui_base::TreeItem` reports "folder" by having children, so a collapsed
//! directory carries a placeholder child that real children replace the first
//! time the user expands it. The model keeps loaded directories and the
//! expanded set; the view rebuilds `TreeItem`s from it on every expand /
//! collapse / refresh (`TreeState::set_items` keeps expansion because the items
//! carry `.expanded(true)` back).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gpui_kit::component::tree::TreeItem;

/// Directories never walked into (VCS internals and build output — the two
/// names that make a project tree unusable when expanded eagerly).
const SKIP_DIRS: [&str; 4] = [".git", "target", "node_modules", ".oino"];
/// A directory's entry cap; beyond it the tree still works but lists less.
const MAX_ENTRIES: usize = 2000;
/// The placeholder id suffix inside a collapsed or empty directory.
const STUB: &str = "\u{0}stub";

/// A lazily expanded directory tree rooted at the project dir.
#[derive(Debug)]
pub struct FileTreeModel {
    /// The tree root (the project dir).
    pub root: PathBuf,
    /// Directories the user expanded (order-insensitive; `BTreeSet` for tests).
    expanded: BTreeSet<PathBuf>,
    /// Loaded directory listings: dir → entries (dirs first, then files, both
    /// alphabetical). A dir absent here is not loaded yet.
    loaded: BTreeMap<PathBuf, Vec<PathBuf>>,
}

/// Lists `dir` filtered (dotfiles and [`SKIP_DIRS`] hidden), sorted dirs first.
fn read_dir(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<(bool, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| (e.file_type().is_ok_and(|t| t.is_dir()), e.path()))
        .filter(|(is_dir, path)| {
            *is_dir
                || path
                    .file_name()
                    .is_some_and(|n| !n.to_string_lossy().is_empty())
        })
        .collect();
    out.retain(|(_, path)| {
        path.file_name()
            .is_none_or(|n| !SKIP_DIRS.contains(&n.to_string_lossy().as_ref()))
            || !path.is_dir()
    });
    out.sort_by(|(a_dir, a), (b_dir, b)| b_dir.cmp(a_dir).then_with(|| a.cmp(b)));
    out.truncate(MAX_ENTRIES);
    out.into_iter().map(|(_, path)| path).collect()
}

impl FileTreeModel {
    /// A model rooted at `root` with only the top level loaded.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        let mut model = Self {
            root: root.to_path_buf(),
            expanded: BTreeSet::new(),
            loaded: BTreeMap::new(),
        };
        model.load(root);
        model
    }

    fn load(&mut self, dir: &Path) {
        if !self.loaded.contains_key(dir) {
            let entries = read_dir(dir);
            self.loaded.insert(dir.to_path_buf(), entries);
        }
    }

    /// Whether `path` is an expandable directory in this tree.
    #[must_use]
    pub fn is_dir(path: &Path) -> bool {
        path.is_dir()
    }

    /// Expands `dir`, loading its first level if needed.
    pub fn expand(&mut self, dir: &Path) {
        self.expanded.insert(dir.to_path_buf());
        self.load(dir);
    }

    /// Collapses `dir` (children stay loaded for an instant re-open).
    pub fn collapse(&mut self, dir: &Path) {
        self.expanded.remove(dir);
    }

    /// Drops all listings and reloads the root plus still-expanded dirs.
    pub fn refresh(&mut self) {
        let expanded: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        self.loaded.clear();
        self.load(&self.root.clone());
        for dir in expanded {
            self.load(&dir);
        }
    }

    /// Builds the `TreeItem`s for the visible entries.
    #[must_use]
    pub fn items(&self) -> Vec<TreeItem> {
        self.children_items(&self.root)
    }

    fn children_items(&self, dir: &Path) -> Vec<TreeItem> {
        let Some(children) = self.loaded.get(dir) else {
            return Vec::new();
        };
        let mut items = Vec::with_capacity(children.len());
        for path in children {
            let label = path.file_name().map_or_else(
                || path.to_string_lossy().into_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
            let mut item = TreeItem::new(path.to_string_lossy().into_owned(), label);
            if Self::is_dir(path) {
                let expanded = self.expanded.contains(path);
                item = item.expanded(expanded);
                if expanded {
                    let kids = self.children_items(path);
                    item = item.children(if kids.is_empty() {
                        // An empty directory still reads as a folder.
                        vec![
                            TreeItem::new(format!("{STUB}{}", path.display()), "(empty)")
                                .disabled(true),
                        ]
                    } else {
                        kids
                    });
                } else {
                    // One stub child so the entry shows its disclosure arrow;
                    // it is never rendered while collapsed.
                    item = item.child(TreeItem::new(format!("{STUB}{}", path.display()), ""));
                }
            }
            items.push(item);
        }
        items
    }

    /// The path a tree item id refers to, or `None` for stub rows.
    #[must_use]
    pub fn path_of(id: &str) -> Option<PathBuf> {
        (!id.contains(STUB)).then(|| PathBuf::from(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oino-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/nested")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("src/nested/deep.rs"), "// deep\n").unwrap();
        std::fs::write(dir.join("README.md"), "# t\n").unwrap();
        dir
    }

    #[test]
    fn lazy_levels_and_skip_dirs() {
        let dir = fixture("lazy");
        let mut model = FileTreeModel::new(&dir);
        let items = model.items();
        // Root level: src (folder stub), README.md — no .hidden, no target.
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_ref()).collect();
        assert_eq!(labels, vec!["src", "README.md"]);
        assert!(items[0].is_folder());
        assert!(!items[1].is_folder());

        // Expanding src loads one level.
        model.expand(&dir.join("src"));
        let items = model.items();
        let src_children: Vec<&str> = items[0].children.iter().map(|i| i.label.as_ref()).collect();
        assert_eq!(src_children, vec!["nested", "main.rs"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collapse_keeps_children_loaded() {
        let dir = fixture("collapse");
        let mut model = FileTreeModel::new(&dir);
        model.expand(&dir.join("src"));
        model.collapse(&dir.join("src"));
        let items = model.items();
        assert!(!items[0].is_expanded());
        assert!(items[0].is_folder());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
