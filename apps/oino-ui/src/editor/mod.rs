//! Editor support for the Files page (round5 #24): a lazy file-tree model, a
//! hand-rolled stdio JSON-RPC client for `rust-analyzer`, and the bridge that
//! installs it into the gpui-base editor's LSP provider traits.
//!
//! | Module | Contents |
//! |---|---|
//! | [`lsp`] | stdio JSON-RPC client: `initialize`, `didOpen`/`didChange`/`didClose`/`didSave`, `hover`, `formatting`, `completion`, `publishDiagnostics` |
//! | [`bridge`] | [`EditorLsp`]: provider-trait glue, server lifecycle, debounced sync, `rustfmt` fallback |
//! | [`tree`] | [`FileTreeModel`]: lazy directory model → `TreeItem`s |
//! | [`prefs`] | `editor.json` (`lsp` toggle) beside `desktop.json` |

pub mod bridge;
pub mod lsp;
pub mod prefs;
pub mod tree;

pub use bridge::{EditorLsp, LspEvent, LspStatus};
pub use prefs::EditorPrefs;
pub use tree::FileTreeModel;
