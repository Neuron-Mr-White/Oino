//! `oino-session`: pi session format v3 for Oino.
//!
//! Session files are JSONL: a `"type":"session"` header followed by tree entries linked by
//! `id`/`parentId`. This crate owns the line types, LLM conversion of coding-agent messages,
//! the pure context-building rules, migration (pi v1/v2 and pre-v1 Oino files) and — behind
//! the default `manager` feature — the synchronous [`SessionManager`] (D20). Without
//! `manager` the crate is pure and builds for `wasm32-wasip2`.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`entry`] | `SessionHeader`, `SessionEntry` | [`SessionHeader`], [`SessionEntry`], [`EntryKind`], [`EntryId`], [`IsoTimestamp`], [`FileLine`] |
//! | [`messages`] | `core/messages.js` | [`convert_to_llm`], summary prefixes, `bashExecutionToText` |
//! | [`context`] | `buildContextEntries` / `buildSessionProjection` / `buildSessionContext` | [`EntryIndex`], [`SessionProjection`], [`SessionContext`] |
//! | [`migrate`] | `migrateSessionEntries` | [`migrate::detect`], [`migrate::parse_session_text`], legacy Oino mapping |
//! | [`manager`] (feature) | `SessionManager` | [`SessionManager`], listing, [`ReadonlySession`] |
//! | [`time`] | `Date#toISOString` | ISO formatting/parsing |
//!
//! Files live at `<sessions root>/--<cwd>--/<ts>_<sessionId>.jsonl` and are append-only; a
//! file is created only once the first user or assistant message exists.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod context;
pub mod entry;
mod error;
#[cfg(feature = "manager")]
pub mod manager;
pub mod messages;
pub mod migrate;
pub mod time;

pub use context::{
    BranchSelection, EntryIndex, ModelRef, ProjectedEntry, SessionContext, SessionProjection,
    branch_selection, build_context_entries, build_session_context, build_session_projection,
    entry_to_context_messages, latest_compaction, path_to,
};
pub use entry::{
    BranchSummaryEntry, CURRENT_SESSION_VERSION, CompactionEntry, ContextReplacement,
    EditableContent, EntryId, EntryKind, FileLine, IsoTimestamp, SessionEntry, SessionHeader,
};
pub use error::SessionError;
#[cfg(feature = "manager")]
pub use manager::{
    BranchSummaryDraft, CompactionDraft, CreateOptions, MigratedSession, ProgressSink,
    ReadonlySession, SessionInfo, SessionManager, SessionTreeNode,
};
pub use messages::{
    BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX, COMPACTION_SUMMARY_PREFIX,
    COMPACTION_SUMMARY_SUFFIX, convert_to_llm,
};
pub use migrate::{FileFormat, MigrationWarning};
