//! Summary prompts (`compaction`, `branch-summarization`,
//! `utils.js`).

/// Branch navigation retains the legacy structured checkpoint instruction.
pub const BRANCH_SYSTEM_PROMPT: &str = "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.";

/// Recovered adaptive §5.1 body. The identity prefix was not recovered and is omitted.
pub const SUMMARIZATION_SYSTEM_PROMPT: &str = r"Summarize this conversation or agent trajectory so the assistant can continue effectively after compaction.

{continuation}

Choose the sections and level of detail that fit this session. Organize around what matters for continuation, not a chronological account of tool calls.

Favor a thorough, complete summary. Use your judgment to prioritize the material and allocate detail where it helps most. Do not omit useful context merely to keep the summary short.

Always preserve relevant user context: the primary request and intent, the latest request, consequential requirements, clarifications, preferences, approvals and their limits, corrections, and unanswered questions. Distinguish user decisions from assistant proposals or assumptions.

Preserve other context according to its value for continuing the work. This may include current line of investigation, important discoveries and reasoning, decisions, outstanding delegated work, unresolved findings, and immediate next actions. Keep identifiers and locations needed to act on that context.

Project files, recorded evidence, and other relevant artifacts may already be durable sources of truth. Prefer referencing these sources while preserving important context that exists only in the conversation.

When updating a previous summary, integrate new information, replace superseded state, and remove details that no longer help continuation. Do not turn proposals into commitments, reports into verified results, partial checks into verified correctness, or historical evidence into current proof.

Return the summary inside <summary> tags.";

/// Conditional adaptive paragraph, first summary.
pub const SUMMARIZATION_PROMPT: &str = "You are to read the full conversation and produce a summary based on guidelines provided below.";
/// Conditional adaptive paragraph, subsequent summaries.
pub const UPDATE_SUMMARIZATION_PROMPT: &str = "You've previously produced a summary of the session up to a certain point. There have been new messages since then. You must update the summary to cover these messages, adhering to the guidelines provided below.";
/// §5.4 soft length cap instruction.
pub const SOFT_CAP_INSTRUCTION: &str = "IMPORTANT: Please keep your new <summary> around a similar length by trimming or condensing the least important information while preserving key facts.";

/// The turn-prefix summarization prompt (prefix of a split turn).
pub const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = "The messages above are earlier context from an ongoing conversation. Later messages are stored separately and do not need to be reconstructed.

Create a concise checkpoint of the user's request and the progress shown above. This checkpoint will be placed before the later messages so the conversation can continue with the necessary context.

## Original Request
[What did the user ask for?]

## Progress So Far
- [Key decisions and work completed in these messages]

## Context Needed to Continue
- [Information from these messages needed to understand the later work]

Only summarize information explicitly present above. Do not infer or recreate later messages.";

/// The branch-summary preamble (prepended to branch summaries).
pub const BRANCH_SUMMARY_PREAMBLE: &str = "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n";

/// The branch summary prompt constant.
pub const BRANCH_SUMMARY_PROMPT: &str = r#"Create a structured summary of this conversation branch for context when returning later.

Use this EXACT format:

## Goal
[What was the user trying to accomplish in this branch?]

## Constraints & Preferences
- [Any constraints, preferences, or requirements mentioned]
- [Or "(none)" if none were mentioned]

## Progress
### Done
- [x] [Completed tasks/changes]

### In Progress
- [ ] [Work that was started but not finished]

### Blocked
- [Issues preventing progress, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [What should happen next to continue this work]

Keep each section concise. Preserve exact file paths, function names, and error messages."#;
