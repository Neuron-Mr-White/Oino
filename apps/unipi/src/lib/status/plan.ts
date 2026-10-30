/**
 * Plan & todos data (WP-A16, Z-43): the Plan & todos section of the status
 * pane reads (a) the workflow extension's sticky `unipi:workflow:status`
 * topic (plan mode / permission mode), (b) the `unipi:plan-mode` custom
 * entries (active + plan file) and (c) the latest todo tool call (`todowrite`,
 * long-horizon's todo tool) from the session's entries and live tools. Plan
 * *approval* stays a dialog (the dock's dialog panel) — this section only
 * reports state. Pure helpers; the component owns the store reads.
 */
import type { AgentMessage, LiveTool, SessionEntry } from '../session/types';

export const WORKFLOW_EXTENSION_ID = 'unipi.workflow';
export const WORKFLOW_STATUS_TOPIC = 'unipi:workflow:status';
export const PLAN_ENTRY_TYPE = 'unipi:plan-mode';
/** The todo tool (long-horizon's `todowrite`; a `todo_write` alias is accepted). */
export const TODO_TOOLS = ['todowrite', 'todo_write'];

export type WorkflowStatus = { planMode: boolean; permissionMode?: string };

export function parseWorkflowStatus(v: unknown): WorkflowStatus | undefined {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return undefined;
  const o = v as Record<string, unknown>;
  if (typeof o.planMode !== 'boolean') return undefined;
  return { planMode: o.planMode, ...(typeof o.permissionMode === 'string' && o.permissionMode ? { permissionMode: o.permissionMode } : {}) };
}

export type TodoStatus = 'pending' | 'in_progress' | 'completed' | 'cancelled';
export type TodoItem = { content: string; status: TodoStatus; priority?: 'high' | 'medium' | 'low' };

const TODO_STATUSES: TodoStatus[] = ['pending', 'in_progress', 'completed', 'cancelled'];

/** The `todos` argument of a todo tool call, narrowed like ralph::validate_todos. */
export function parseTodos(v: unknown): TodoItem[] {
  if (!Array.isArray(v)) return [];
  const out: TodoItem[] = [];
  for (const raw of v) {
    if (!raw || typeof raw !== 'object') continue;
    const o = raw as Record<string, unknown>;
    const content = typeof o.content === 'string' ? o.content.trim() : '';
    const status = o.status as TodoStatus;
    if (!content || !TODO_STATUSES.includes(status)) continue;
    const priority = o.priority === 'high' || o.priority === 'medium' || o.priority === 'low' ? o.priority : undefined;
    out.push({ content: content.slice(0, 200), status, ...(priority ? { priority } : {}) });
  }
  return out;
}

/** The tool-call parts of one assistant message, in order. */
function toolCalls(message: AgentMessage | undefined): Array<{ name: string; args: unknown }> {
  const content = message?.content;
  if (!Array.isArray(content)) return [];
  return content.flatMap((part) => {
    if (!part || typeof part !== 'object' || (part as { type?: unknown }).type !== 'toolCall') return [];
    const call = part as { name?: unknown; arguments?: unknown };
    return typeof call.name === 'string' ? [{ name: call.name, args: call.arguments }] : [];
  });
}

/** The latest `todowrite` args over the entries (newest first wins) and any live call. */
export function latestTodos(entries: readonly SessionEntry[], liveTools: Record<string, LiveTool>): TodoItem[] {
  const live = Object.values(liveTools).reverse().find((t) => TODO_TOOLS.includes(t.name));
  if (live) {
    const todos = parseTodos(live.args?.todos);
    if (todos.length) return todos;
  }
  for (let i = entries.length - 1; i >= 0; i--) {
    for (const call of toolCalls(entries[i]!.message).reverse()) {
      if (!TODO_TOOLS.includes(call.name)) continue;
      const todos = parseTodos((call.args as { todos?: unknown } | undefined)?.todos ?? call.args);
      if (todos.length) return todos;
    }
  }
  return [];
}

/** The latest `unipi:plan-mode` custom entry's data (`{active, planFile?}`). */
export function latestPlanEntry(entries: readonly SessionEntry[]): { active: boolean; planFile?: string } | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i]!;
    if (e.customType !== PLAN_ENTRY_TYPE) continue;
    const data = e.data as { active?: unknown; planFile?: unknown } | undefined;
    return { active: data?.active === true, ...(typeof data?.planFile === 'string' && data.planFile ? { planFile: data.planFile } : {}) };
  }
  return undefined;
}

/** Done (completed) of non-cancelled todos, for the progress bar and count chip. */
export function todoProgress(todos: readonly TodoItem[]): { done: number; total: number; inProgress: number } {
  const live = todos.filter((t) => t.status !== 'cancelled');
  return { done: live.filter((t) => t.status === 'completed').length, total: live.length, inProgress: live.filter((t) => t.status === 'in_progress').length };
}

/** The plan file's basename for the section's title row. */
export function planFileLabel(path: string | undefined): string | undefined {
  if (!path) return undefined;
  const trimmed = path.replace(/\/+$/, '');
  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || path;
}
