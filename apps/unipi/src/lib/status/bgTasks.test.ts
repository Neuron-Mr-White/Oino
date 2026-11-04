import { describe, expect, it } from 'vitest';
import { bgExitText, formatBytes, parseBgTasks, toTermText } from './bgTasks';
import { filterCounts } from './subagentRows';

describe('bgTasks', () => {
  const rows = parseBgTasks({
    tasks: [
      { id: 'a', name: 'build', command: 'npm run build', status: 'completed', startTime: 1, endTime: 5, exitCode: 0, bytesWritten: 2048, triggerOnCompletion: true },
      { id: 'b', command: 'sleep 9', status: 'running', startTime: 10, bytesWritten: 0, triggerOnCompletion: true },
      { id: 'c', status: 'killed', startTime: 3 },
    ],
  })!;
  it('maps statuses, newest first, same filter counts as subagents', () => {
    expect(rows.map((r) => r.id)).toEqual(['b', 'c', 'a']);
    expect(rows.map((r) => r.status)).toEqual(['running', 'cancelled', 'done']);
    expect(filterCounts(rows)).toMatchObject({ all: 3, running: 1, done: 1, cancelled: 1 });
    expect(rows[0]!.wakes).toBe(true);
    expect(rows[0]!.title).toBe('sleep 9');
  });
  it('formats exit, bytes and terminal text', () => {
    expect(bgExitText(rows[2]!)).toBe('exit 0');
    expect(bgExitText(rows[1]!)).toBe('stopped');
    expect(formatBytes(1_258_000)).toBe('1.2 MB');
    expect(toTermText('a\nb\r\nc')).toBe('a\r\nb\r\nc');
    expect(parseBgTasks({})).toBeUndefined();
  });
});
