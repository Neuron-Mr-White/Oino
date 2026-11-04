import { describe, expect, it } from 'vitest';
import { bgExitText, formatBytes, parseBgTasks, toTermText } from './bgTasks';
import { filterCounts } from './subagentRows';
import { BG_TASKS_TOPIC, bgTasksTopic, bgTasksTopics, bgTopicData, topicRunningCount } from './running';

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
  it("a chat reads its own task topic, not the global one another session overwrote", () => {
    const mine = { tasks: [{ id: 't1', command: 'sleep 9', status: 'running' }] };
    const topics = {
      [BG_TASKS_TOPIC]: { data: { tasks: [] } },
      [bgTasksTopic('s1')]: { data: mine },
    } as never;
    expect(bgTasksTopics('s1')).toEqual(['unipi:bg:tasks:s1', 'unipi:bg:tasks']);
    expect(parseBgTasks(bgTopicData(topics, 's1'))?.map((r) => r.id)).toEqual(['t1']);
    expect(topicRunningCount(topics, 's1')).toBe(1);
    // an older extension publishes only the global topic
    expect(bgTopicData({ [BG_TASKS_TOPIC]: { data: mine } } as never, 's1')).toBe(mine);
  });
});
