//! Compaction and branch summarisation, plus the assistant error
//! classification core reuses for overflow recovery and auto-retry.
//!
//! Pure logic over `oino-session` entries and projections; the only side effect is the
//! summary request, sent through an [`oino_agent::StreamFn`] by a [`Summarizer`]. Session
//! I/O, events and extension hooks stay in core.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`settings`] | [`CompactionSettings`], [`DEFAULT_COMPACTION_SETTINGS`] |
//! | [`tokens`] | [`ContextUsageEstimate`], [`estimate_tokens`], [`should_compact`] |
//! | [`cut_point`] | [`CutPointResult`], [`find_cut_point`] |
//! | [`prepare`] | [`prepare_compaction`], [`CompactionPreparation`], [`CompactionResult`] |
//! | [`summarize`] | [`Summarizer`], [`generate_summary`], [`compact()`], [`summarize_default`] |
//! | [`branch`] | [`collect_entries_for_branch_summary`], [`prepare_branch_entries`], [`generate_branch_summary`] |
//! | [`utils`] | [`FileOperations`], [`FileLists`], [`format_file_operations`], [`serialize_conversation`] |
//! | [`hook_payload`] | [`bound_branch_entries`], [`bound_preparation`], [`HOOK_PAYLOAD_LIMITS`] |
//! | [`prompts`] | [`SUMMARIZATION_SYSTEM_PROMPT`] and the summary prompts |
//! | [`retry`] | [`RetryPolicy`], [`retry_delay_ms`], [`SummarizationRetryObserver`] |
//! | [`errors`] | [`is_context_overflow`], [`is_retryable_assistant_error`] |
//!
//! String lengths, truncation and sorting follow JavaScript (UTF-16 code units) so token
//! estimates, cut points and prompts are locked to golden fixtures.

pub mod branch;
pub mod cut_point;
mod error;
pub mod errors;
pub mod hook_payload;
mod js;
mod pattern;
pub mod prepare;
pub mod prompts;
pub mod retry;
pub mod settings;
pub mod summarize;
pub mod tokens;
pub mod utils;
mod uuid;

pub use branch::{
    BranchPreparation, BranchSummaryDetails, BranchSummaryOptions, BranchSummaryResult,
    CollectEntriesResult, DEFAULT_BRANCH_SUMMARY_RESERVE_TOKENS,
    collect_entries_for_branch_summary, generate_branch_summary, prepare_branch_entries,
};
pub use cut_point::{CutPointResult, find_cut_point, find_turn_start_index};
pub use error::CompactionError;
pub use errors::{is_context_overflow, is_recoverable_length, is_retryable_assistant_error};
pub use hook_payload::{
    BoundedBranch, HOOK_PAYLOAD_LIMITS, HookPayloadLimits, bound_branch_entries, bound_preparation,
};
pub use prepare::{
    CompactionDetails, CompactionPreparation, CompactionResult, prepare_compaction,
    prepare_single_turn_compaction,
};
pub use prompts::SUMMARIZATION_SYSTEM_PROMPT;
pub use retry::{
    DEFAULT_MAX_AGENT_RETRY_DELAY_MS, RetryPolicy, SummarizationRetryObserver,
    retry_assistant_call, retry_delay_ms,
};
pub use settings::{CompactionSettings, DEFAULT_COMPACTION_SETTINGS};
pub use summarize::{
    Headers, Summarizer, SummaryOutput, compact, generate_summary, summarization_context,
    summarization_failure, summarize_default,
};
pub use tokens::{
    ContextUsageEstimate, calculate_context_tokens, estimate_context_tokens,
    estimate_projected_context_tokens, estimate_tokens, last_assistant_usage, should_compact,
};
pub use utils::{FileLists, FileOperations, format_file_operations, serialize_conversation};
