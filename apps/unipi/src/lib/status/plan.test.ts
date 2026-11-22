/**
 * `lib/status/plan.ts` (WP-A16, Z-43): the workflow topic, the plan entry and
 * the latest todowrite checklist from the session's entries.
 */
import { describe, expect, it } from 'vitest';
import { latestPlanEntry, latestTodos, parseTodos, parseWorkflowStatus, planFileLabel, todoCapsuleLabel, todoProgress, todoSummary, type TodoItem } from './plan';

const entry = (id: string, extra: Record<string, unknown>) => ({ type: 'message', id, ...extra });

describe('parseWorkflowStatus', () => {
  it('camelCase topic, narrowed', () => {
    expect(parseWorkflowStatus({ planMode: true, permissionMode: 'default' })).toEqual({ planMode: true, permissionMode: 'default' });
    expect(parseWorkflowStatus({ planMode: false, permissionMode: null })).toEqual({ planMode: false });
    expect(parseWorkflowStatus(undefined)).toBeUndefined();
    expect(parseWorkflowStatus({ planMode: 'yes' })).toBeUndefined();
  });
});

describe('parseTodos', () => {
  it('keeps valid items, drops broken ones', () => {
    expect(parseTodos([{ content: ' a ', status: 'in_progress' }, { content: 'b', status: 'completed', priority: 'high' }, { content: '', status: 'pending' }, { content: 'x', status: 'nope' }, 'junk'])).toEqual([
      { content: 'a', status: 'in_progress' },
      { content: 'b', status: 'completed', priority: 'high' },
    ]);
    expect(parseTodos(undefined)).toEqual([]);
  });
});

const ENTRIES = [
  entry('e1', { message: { role: 'user', content: 'go' } }),
  entry('e2', {
    message: {
      role: 'assistant',
      content: [{ type: 'toolCall', id: 't1', name: 'todowrite', arguments: { todos: [{ content: 'old list', status: 'completed' }] } }],
    },
  }),
  entry('e3', { customType: 'unipi:plan-mode', data: { active: false } }),
  entry('e4', {
    message: {
      role: 'assistant',
      content: [
        { type: 'text', text: 'planning' },
        { type: 'toolCall', id: 't2', name: 'todowrite', arguments: { todos: [{ content: 'first', status: 'completed' }, { content: 'second', status: 'in_progress' }, { content: 'dropped', status: 'cancelled' }] } },
      ],
    },
  }),
  entry('e5', { customType: 'unipi:plan-mode', data: { active: true, planFile: '/p/2026-02-14-plan.md' } }),
];

describe('latestTodos', () => {
  it('the newest todowrite call wins (scanning backwards)', () => {
    const todos = latestTodos(ENTRIES, {});
    expect(todos.map((t) => t.content)).toEqual(['first', 'second', 'dropped']);
    expect(todoProgress(todos)).toEqual({ done: 1, total: 2, inProgress: 1 });
  });

  it('a live in-flight call beats the entries', () => {
    const todos = latestTodos(ENTRIES, { live: { id: 'live', name: 'todowrite', args: { todos: [{ content: 'live one', status: 'in_progress' }] }, startedAt: 1 } });
    expect(todos.map((t) => t.content)).toEqual(['live one']);
  });

  it('nothing yet → empty', () => {
    expect(latestTodos([ENTRIES[0]!], {})).toEqual([]);
  });
});

describe('latestPlanEntry / planFileLabel', () => {
  it('the latest plan-mode entry', () => {
    expect(latestPlanEntry(ENTRIES)).toEqual({ active: true, planFile: '/p/2026-02-14-plan.md' });
    expect(latestPlanEntry([])).toBeUndefined();
    expect(planFileLabel('/p/2026-02-14-plan.md')).toBe('2026-02-14-plan.md');
    expect(planFileLabel(undefined)).toBeUndefined();
  });
});

describe('todoSummary / todoCapsuleLabel', () => {
  const t = (content: string, status: TodoItem['status']): TodoItem => ({ content, status });

  it('no todos → null (also when only cancelled ones remain)', () => {
    expect(todoSummary([])).toBeNull();
    expect(todoSummary([t('x', 'cancelled')])).toBeNull();
    expect(todoSummary(latestTodos([ENTRIES[0]!], {}))).toBeNull();
  });

  it('cancelled items are excluded from the total', () => {
    const s = todoSummary([t('a', 'completed'), t('b', 'cancelled'), t('c', 'pending')])!;
    expect([s.done, s.total]).toEqual([1, 2]);
  });

  it('the in-progress item is current, even after earlier pending ones', () => {
    const s = todoSummary([t('a', 'completed'), t('b', 'pending'), t('c', 'in_progress'), t('d', 'in_progress')])!;
    expect(s.current?.content).toBe('c');
    expect(todoCapsuleLabel(s)).toBe('1/4 \u00b7 c');
  });

  it('falls back to the first pending item', () => {
    const s = todoSummary([t('a', 'completed'), t('b', 'cancelled'), t('c', 'pending'), t('d', 'pending')])!;
    expect(s.current?.content).toBe('c');
    expect(s.allDone).toBe(false);
  });

  it('all done → no current, "All N done"', () => {
    const s = todoSummary([t('a', 'completed'), t('b', 'completed'), t('c', 'cancelled')])!;
    expect(s.allDone).toBe(true);
    expect(s.current).toBeUndefined();
    expect(todoCapsuleLabel(s)).toBe('All 2 done');
  });
});
