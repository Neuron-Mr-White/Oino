//! Compaction and branch summarisation (pi `core/compaction/*`), plus the assistant error
//! classification core reuses for overflow recovery and auto-retry.
//!
//! Pure logic over `oino-session` entries and projections; the only side effect is the
//! summary request, sent through an [`oino_agent::StreamFn`] by a [`Summarizer`]. Session
//! I/O, events and extension hooks stay in core.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`settings`] | `CompactionSettings` | [`CompactionSettings`], [`DEFAULT_COMPACTION_SETTINGS`] |
//! | [`tokens`] | `calculateContextTokens`, `estimateTokens`, `estimateContextTokens`, `estimateProjectedContextTokens`, `shouldCompact`, `getLastAssistantUsage` | [`ContextUsageEstimate`], [`estimate_tokens`], [`should_compact`] |
//! | [`cut_point`] | `findCutPoint`, `findTurnStartIndex` | [`CutPointResult`], [`find_cut_point`] |
//! | [`prepare`] | `prepareCompaction`, `CompactionPreparation`, `CompactionResult` | [`prepare_compaction`], [`CompactionPreparation`], [`CompactionResult`] |
//! | [`summarize`] | `completeSummarization`, `generateSummaryWithUsage`, `getSummarizationFailure`, `compact` | [`Summarizer`], [`generate_summary`], [`compact()`], [`summarize_default`] |
//! | [`branch`] | `branch-summarization.js` | [`collect_entries_for_branch_summary`], [`prepare_branch_entries`], [`generate_branch_summary`] |
//! | [`utils`] | `utils.js` | [`FileOperations`], [`FileLists`], [`format_file_operations`], [`serialize_conversation`] |
//! | [`prompts`] | prompt constants | [`SUMMARIZATION_SYSTEM_PROMPT`] and the summary prompts |
//! | [`retry`] | pi-ai `retryAssistantCall`, `RetryPolicy`, `RetryCallbacks` | [`RetryPolicy`], [`retry_delay_ms`], [`SummarizationRetryObserver`] |
//! | [`errors`] | pi-ai `isContextOverflow`, `isRetryableAssistantError`, `isRecoverableLength` | [`is_context_overflow`], [`is_retryable_assistant_error`] |
//!
//! String lengths, truncation and sorting follow JavaScript (UTF-16 code units) so token
//! estimates, cut points and prompts match pi byte for byte.

pub mod branch;
pub mod cut_point;
mod error;
pub mod errors;
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
pub use prepare::{CompactionDetails, CompactionPreparation, CompactionResult, prepare_compaction};
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
