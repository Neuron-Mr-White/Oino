import { describe, expect, it } from 'vitest';
import { parseMcpServers } from './McpServers';
import { parseDreamStatus } from '../memory/DreamPanel';

describe('native MCP / Dream data', () => {
  it('keeps the full server error and tools', () => {
    const long = 'MCP initialize handshake failed: HTTP 415: {"jsonrpc":"2.0","error":{"code":-32600,"message":"Unsupported Media Type: Content-Type must be application/json"}}';
    const d = parseMcpServers({ servers: [{ name: 'refero', source: 'project-override', status: 'error', error: long, toolCount: 0, tools: [] }, { bad: 1 }], errors: ['x'], totalTools: 3 });
    expect(d.servers).toHaveLength(1);
    expect(d.servers[0]!.error).toBe(long);
    expect(d.errors).toEqual(['x']);
  });
  it('parses dream status with runs and proposals', () => {
    const s = parseDreamStatus({ enabled: false, running: false, runs: [{ id: 'r1', status: 'done', sessions: 3, proposals: [{ id: 'p1', name: 'skill-a', kind: 'skill', decision: 'pending' }] }], due: { due: false, reason: '2 of 5 sessions' } });
    expect(s.enabled).toBe(false);
    expect(s.runs[0]!.proposals[0]).toMatchObject({ id: 'p1', decision: 'pending' });
    expect(s.due).toEqual({ due: false, reason: '2 of 5 sessions' });
  });
});
