/**
 * Chat event corpus ([chat-events]): one realistic entry per event kind that
 * can reach a chat, shaped like real session lines (synthetic text, no user
 * data). Used by the eventKinds / EventCard goldens and by the QA seed
 * session (scripts/qa/chat-events-seed.mjs writes it as a session file).
 */
import type { SessionEntry } from '../../../lib/session/types';

const ts = (n: number) => `2026-11-03T10:${String(10 + n).padStart(2, '0')}:00.000Z`;

const recall = [
  '## 🧠 Memory System Active',
  '',
  'You have 5 memories stored for project "oino".',
  '**BEFORE starting work**, call `memory_search` with relevant keywords to check for existing context.',
  '',
  'Available memories:',
  '- oino_handoff_20261102_finalize_graph',
  '- oino_phone_chat_package_b_findings',
  '- oino_chat_scroll_bottom_lock_unread_inner_follow_findings',
  '- oino_remote_build_agent37_vm_deb_apk_recipe',
  '- oino_design_spec_zcode_unipi_parity',
  '',
  '**AFTER completing the task**, if you learned something non-obvious,',
  'call `memory_store` to save it for future sessions.',
].join('\n');

/** `[kind id, entry]` — the kind id is what `describeEvent` must answer (`null` = hidden). */
export const CHAT_EVENT_CORPUS: Array<[string | null, SessionEntry]> = [
  ['memory-recall', { type: 'custom_message', id: 'ev01', timestamp: ts(1), customType: 'unipi-memory-recall-reminder', content: recall, display: true, details: { oinoProvenance: { source: 'Memory', title: 'Memory recall', synopsis: 'Recalled at session start' } } }],
  ['memory-recall', { type: 'custom_message', id: 'ev02', timestamp: ts(2), customType: 'unipi-memory-recall-reminder', content: '## 🧠 Memory System Active\n\nYou have 0 memories stored for project "openmontage".\n\n**AFTER completing the task**, call `memory_store`.', display: false }],
  ['memory-nudge', { type: 'custom_message', id: 'ev03', timestamp: ts(3), customType: 'unipi-memory-retro-reminder', content: '**🧠 Memory reminder:** If you learned something non-obvious in this task, call `memory_store` to save it as a memory for future sessions. Update existing memories instead of creating duplicates.', display: false }],
  ['memory-wake', { type: 'custom', id: 'ev04', timestamp: ts(4), customType: 'unipi-memory-session-card', data: { project: 'oino', count: 215, wakeUp: 'Wake-up text (~790 tokens):\n==================================================\nUnipi app on Oino (v2 branch)\n\n## L1 — ESSENTIAL STORY\n\n[decision]\n  - Builds run on Agent37, never on this PC.  (oino_build_offload_agent37_owner_pref.md)\n  - Release keys live in ~/.oino-release-keys.  (oino_release_keys_update_feeds_android_id_20261103.md)' } }],
  ['memory-saved', { type: 'custom', id: 'ev05', timestamp: ts(5), customType: 'unipi-memory-save-card', data: { titles: ['oino_phone_sheet_scrim_above_sheet_zindex_specificity', 'oino_chat_events_parity_findings'], usage: { input: 86858, cacheRead: 864064, cacheWrite: 0, output: 11638 } } }],
  ['subagent-done', { type: 'custom_message', id: 'ev06', timestamp: ts(6), customType: 'subagent-completion', content: 'Subagent finished: Audit sidebar rows', display: true, details: { id: 'a1b2c3', title: 'Audit sidebar rows', status: 'completed', durationMs: 219696, usage: { cost: 0.4213 }, report: '**Result**: 3 issues found.\n\n1. Row height jumps on hover.\n2. Unread dot overlaps the time.\n3. `SessionRow.tsx:111` pads twice.' } }],
  ['subagent-notification', { type: 'custom_message', id: 'ev07', timestamp: ts(7), customType: 'subagent-notification', content: '<task-notification>\n<task-id>225f9f88</task-id>\n<status>Done</status>\n<summary>Agent "Trace film sessions" completed</summary>\n<result>Report: **`/tmp/report/provenance.md`**\n\nKey findings:\n\n- The three exports are deployed-job outputs.\n- Audio configuration is unverified.</result>\n<usage><total_tokens>488676</total_tokens><tool_uses>10</tool_uses><duration_ms>219696</duration_ms></usage>\n</task-notification>', display: true }],
  ['sidekick-done', { type: 'custom_message', id: 'ev08', timestamp: ts(8), customType: 'sidekick-completion', content: '<subagent_completion_notification agent_id="da38" status="completed">\n**Result**: DONE — all 7 rework items implemented; 95 tests passed.\n\n1. Story schema now serialized into the request.\n</subagent_completion_notification>', display: true, details: { id: 'da38', status: 'completed', durationMs: 65000, usage: { cost: 0.081 } } }],
  ['background-task', { type: 'custom_message', id: 'ev09', timestamp: ts(9), customType: 'background-task-notification', content: '<background-task-notification>\n  <task-id>bd82ffeb4</task-id>\n  <task-name>Thinking contract validation</task-name>\n  <status>failed</status>\n  <exit-code>1</exit-code>\n  <error>Exited with code 1</error>\n</background-task-notification>', display: true, details: { id: 'bd82ffeb4', name: 'Thinking contract validation', command: 'python -m pytest tests/api/test_thinking_config.py -q', status: 'failed', exitCode: 1, startTime: 1762160000000, endTime: 1762160042000, outputTail: ['FAILED tests/api/test_thinking_config.py::test_levels - KeyError: xhigh', '1 failed, 41 passed in 3.12s'] } }],
  ['background-task', { type: 'custom_message', id: 'ev10', timestamp: ts(10), customType: 'background-task-notification', content: '<background-task-notification>\n  <task-id>c11</task-id>\n  <task-name>vitest run</task-name>\n  <status>completed</status>\n  <exit-code>0</exit-code>\n</background-task-notification>', display: true, details: { id: 'c11', name: 'vitest run', command: 'npx vitest run', status: 'completed', exitCode: 0, outputTail: ['Test Files  37 passed (37)', 'Tests  288 passed (288)'] } }],
  ['compaction-card', { type: 'custom', id: 'ev11', timestamp: ts(11), customType: 'unipi-compaction', data: { method: 'vcc', trigger: 'threshold', tokensBefore: 182000, tokensAfter: 41000, keptTurns: 6, totalTurns: 48, keptTokens: 23000, summaryTokens: 4100, sections: ['Goal and constraints', 'Files touched (12)', 'Open questions'], percent: 82 } }],
  ['compactor-recall', { type: 'custom_message', id: 'ev12', timestamp: ts(12), customType: 'compactor-recall', content: '## Session recall: "phone sheet"\n\n- #412 sheet scrim z-index fix\n- #518 detents for the tool sheet', display: true }],
  ['compactor-resume', { type: 'custom_message', id: 'ev13', timestamp: ts(13), customType: 'unipi-compactor-resume', content: '<session_resume events="1000" compact_count="8">\n  <files count="384">\n    stages.py (read×1)\n    ChatContainer.tsx (edit×1)\n  </files>\n</session_resume>', display: false }],
  ['mode-switch', { type: 'custom', id: 'ev14', timestamp: ts(14), customType: 'long-horizon-decision', data: { mode: 'graph', source: 'explicit' } }],
  ['goal-progress', { type: 'custom', id: 'ev15', timestamp: ts(15), customType: 'unipi-progress', data: { kind: 'goal', percent: 40, summary: 'timeline rows done, QA shots pending', objective: 'Every chat event kind renders and expands' } }],
  ['lh-continue', { type: 'custom_message', id: 'ev16', timestamp: ts(16), customType: 'unipi:lh-continue', content: 'Continue the graph: 2 nodes ready (chat-events, rel-build).', display: true }],
  ['ralph-iteration', { type: 'custom_message', id: 'ev17', timestamp: ts(17), customType: 'unipi-ralph-loop-reminder', content: '[RALPH LOOP - agent-only-subtraction - Iteration 18/30]\n\nYou are in a Ralph loop working on: .unipi/ralph/agent-only-subtraction.md\n- Work on ~3 items this iteration', display: false }],
  ['plan-mode', { type: 'custom_message', id: 'ev18', timestamp: ts(18), customType: 'unipi:plan-mode-message', content: 'Plan mode is on: read-only tools until the plan is approved.', display: true, details: { oinoProvenance: { source: 'Plan mode', title: 'Plan mode on' } } }],
  ['watchdog', { type: 'custom_message', id: 'ev19', timestamp: ts(19), customType: 'unipi-watchdog', content: 'Tool `bash` has been running for 12m (limit 10m).\n\nConsider bg_run for long commands.', display: true }],
  ['skills-revealed', { type: 'custom_message', id: 'ev20', timestamp: ts(20), customType: 'unipi-skills-revealed', content: 'Skills now available: **browser**, **image**.', display: true }],
  ['response', { type: 'custom_message', id: 'ev21', timestamp: ts(21), customType: 'unipi-response', content: 'Mode command: Graph\nLast resolved: graph (explicit)\nJudge: off — default mode: none', display: true, details: { unipiHarness: { version: 1, id: 'hm-8f7940fc', source: 'Long-horizon', title: 'Response', synopsis: 'Mode command response', delivery: 'followUp' } } }],
  ['kanboard-next', { type: 'custom_message', id: 'ev22', timestamp: ts(22), customType: 'unipi:kanboard-next', content: 'Next task: **OINO-42** Chat events parity.', display: true }],
  ['kanboard-badge', { type: 'custom', id: 'ev23', timestamp: ts(23), customType: 'unipi:kanboard-badge', data: { id: 'OINO-42', kind: 'claimed' } }],
  ['dream-report', { type: 'custom', id: 'ev24', timestamp: ts(24), customType: 'unipi-dream-report-card', data: { report: '~/.oino/dream/runs/2026-11-03/DREAM_REPORT.md', pending: 3 } }],
  ['btw', { type: 'custom', id: 'ev25', timestamp: ts(25), customType: 'btw-thread-entry', data: { question: 'What is the overall progress right now?', answer: '**Core is in place**; QA shots are pending.\n\n- rows: done\n- tests: done', model: 'glm-5.3' } }],
  ['attachments', { type: 'custom', id: 'ev26', timestamp: ts(26), customType: 'unipi:attachments', data: { items: [{ id: 1, kind: 'file', name: 'design.pdf' }, { id: 2, kind: 'image', name: 'shot.png' }] } }],
  ['browser-shot', { type: 'custom', id: 'ev27', timestamp: ts(27), customType: 'unipi.browser/turn-shot', data: { url: 'https://oino.dev/', path: '/tmp/shot.png' } }],
  ['user-bash', { type: 'message', id: 'ev28', timestamp: ts(28), message: { role: 'bashExecution', command: 'git status --short', output: ' M apps/unipi/src/lib/session/turns.ts\n?? docs/dev/v2/qa/chat-events-parity.md\n', exitCode: 0, cancelled: false, truncated: false, timestamp: 1762164480000 } }],
  ['user-bash', { type: 'message', id: 'ev29', timestamp: ts(29), message: { role: 'bashExecution', command: 'cargo test -p oino-core', output: 'error[E0432]: unresolved import `crate::x`\n', exitCode: 101, cancelled: false, truncated: false, timestamp: 1762164540000 } }],
  ['branch-summary', { type: 'branch_summary', id: 'ev30', timestamp: ts(30), fromId: 'x1', summary: 'The other branch tried a CSS-only fix for the sheet scrim; it failed on WebKit.\n\n- reverted `phone.css`\n- kept the z-index token' } as unknown as SessionEntry],
  ['system', { type: 'message', id: 'ev31', timestamp: ts(31), message: { role: 'system', content: 'Mode: Graph — Available long-horizon tools: update_agent_graph, read_agent_graph, graph_checkpoint, graph_wait, graph_done.', timestamp: 1762164660000 } }],
  ['custom', { type: 'custom_message', id: 'ev32', timestamp: ts(32), customType: 'acme:weekly-report', content: '', display: true, details: { rows: 3, ok: true } }],
  [null, { type: 'custom_message', id: 'ev33', timestamp: ts(33), customType: 'unipi-continue', content: '', display: false }],
  [null, { type: 'custom', id: 'ev34', timestamp: ts(34), customType: 'oino.tool-dispatch', data: { toolCallId: 'c1', toolName: 'bash', input: { command: 'ls' } } }],
  [null, { type: 'custom', id: 'ev35', timestamp: ts(35), customType: 'long-horizon-decision', data: { mode: 'none', source: 'default' } }],
  [null, { type: 'custom', id: 'ev36', timestamp: ts(36), customType: 'unipi-child-usage', data: { model: 'x', input: 10, output: 5, cost: 0.2 } }],
];
