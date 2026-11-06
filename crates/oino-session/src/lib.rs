//! `oino-session`: session format v3 for Oino.
//!
//! Session files are JSONL: a `"type":"session"` header followed by tree entries linked by
//! `id`/`parentId`. This crate owns the line types, LLM conversion of coding-agent messages,
//! the pure context-building rules, migration (v1/v2 and pre-v1 Oino files) and — behind
//! the default `manager` feature — the synchronous [`SessionManager`] (D20). Without
//! `manager` the crate is pure and builds for `wasm32-wasip2`.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`entry`] | [`SessionHeader`], [`SessionEntry`], [`EntryKind`], [`EntryId`], [`IsoTimestamp`], [`FileLine`] |
//! | [`messages`] | [`convert_to_llm`], summary prefixes, `bashExecutionToText` |
//! | [`context`] | [`EntryIndex`], [`SessionProjection`], [`SessionContext`] |
//! | [`migrate`] | [`migrate::detect`], [`migrate::parse_session_text`], legacy Oino mapping |
//! | [`manager`] (feature) | [`SessionManager`], listing, [`ReadonlySession`] |
//! | [`time`] | ISO formatting/parsing |
//!
//! Files live at `<sessions root>/--<cwd>--/<ts>_<sessionId>.jsonl` and are append-only; a
//! file is created only once the first user or assistant message exists.

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-256.png",
    html_favicon_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/oino.ico"
)]
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
    BranchSummaryEntry, CURRENT_SESSION_VERSION, CWD_CHANGE_CUSTOM_TYPE, CompactionAnchor,
    CompactionEntry, CompactionState, ContextReplacement, EditableContent, EntryId, EntryKind,
    FileLine, IsoTimestamp, RetainedImage, SessionEntry, SessionHeader, SummaryKind,
};
pub use error::SessionError;
#[cfg(feature = "manager")]
pub use error::SessionLockError;
#[cfg(feature = "manager")]
pub use manager::{
    BranchSummaryDraft, CompactionDraft, CreateOptions, INTERRUPTED_TOOL_TEXT, LeaseInfo,
    MigratedSession, ProgressSink, RESUME_REPAIR_TYPE, ReadonlySession, RepairReport, SessionInfo,
    SessionLease, SessionManager, SessionTreeFlatNode, SessionTreeNode, TOOL_DISPATCH_TYPE,
    TakeoverRequest, clear_takeover_request, lock_file_path, lock_metadata_path,
    read_takeover_request, takeover_path, write_takeover_request,
};
pub use messages::{
    BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX, COMPACTION_SUMMARY_PREFIX,
    COMPACTION_SUMMARY_SUFFIX, convert_to_llm,
};
pub use migrate::{FileFormat, MigrationWarning};
