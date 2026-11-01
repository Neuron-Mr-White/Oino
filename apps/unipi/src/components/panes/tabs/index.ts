/**
 * Session side tabs (WP-A10, app spec §3.8): the four session-scoped pane
 * kinds' views and the openers every entry point uses.
 *
 * Importing this module registers the views (`setPaneComponent`, one line per
 * kind in its own file). `lib/panes/kinds.ts` loads it lazily so the kinds
 * registry never sits in an import cycle with the shell.
 */
import './DiffTab';
import './TreeTab';
import './BtwTab';
import './SubagentTab';
// P1-05: the port-forward tab registers with the other pane views (SidePane imports it too).
import '../ForwardTab';

export { SessionTabLayer } from './SessionTabLayer';
export { BtwTab } from './BtwTab';
export { DiffTab, editLine, editsOfArgs, findToolCall, pathOfArgs, renderSplitHtml, splitRows, type DiffEdit, type ToolCallSite } from './DiffTab';
export { SubagentTab, SubagentTranscript, fetchAgentMessages, fetchAgents, SUBAGENT_POLL_MS } from './SubagentTab';
export { TreeTab } from './TreeTab';
export { closePhoneSessionTab, openBtwTab, openDiffTab, openSessionTab, openSubagentTab, openTreeTab, phoneSessionTab, seedPhoneSessionTab, tabBaseOf, type PhoneSessionTab, type SessionTabBase } from './sessionTabs';
export { agentOfEntry, agentOfStepData, AGENT_CARD_TYPES, AGENT_STEP_TYPES, type AgentRef } from './agentEntry';
