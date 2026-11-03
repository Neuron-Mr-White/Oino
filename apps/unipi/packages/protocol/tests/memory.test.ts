import { describe, expect, it } from 'vitest';
import { MemoryBrowseDataSchema, MemoryBrowseRequestSchema, memoryProjectName } from '../src/index.js';

describe('memory_browse', () => {
  it('parses a page and the request', () => {
    const page = MemoryBrowseDataSchema.parse({ items: [{ id: 'a', title: 'T', kind: 'decision', project: 'oino', updated_at: 1, body: 'b' }], total: 7000, next_cursor: '50', store: true });
    expect(page.projects).toEqual([]);
    expect(page.next_cursor).toBe('50');
    expect(MemoryBrowseRequestSchema.safeParse({ op: 'memory_browse', limit: 101 }).success).toBe(false);
  });
  it('names projects like the memory extension', () => {
    expect(memoryProjectName('/home/u/Projects/llm/oino')).toBe('oino');
    expect(memoryProjectName('/home/u/My Project-2/')).toBe('my_project_2');
    expect(memoryProjectName('/')).toBe('unknown');
  });
});
