import { afterEach, describe, expect, it } from 'vitest';
import { api, ApiError, canMove, displayTitle, needsComment, parseError, PERMISSIVE_RULES } from './api';
import { setBoardTransport, type BoardTransport } from './transport';
import { fakeBoard, task } from './testing';

afterEach(() => setBoardTransport(undefined));

describe('parseError', () => {
  it('reads the daemon error JSON incl. needsComment / needsAgent', () => {
    const error = parseError(409, JSON.stringify({ error: 'run summary required', needsComment: true }));
    expect(error).toBeInstanceOf(ApiError);
    expect(error.message).toBe('run summary required');
    expect(error.status).toBe(409);
    expect(error.needsComment).toBe(true);
    expect(error.needsAgent).toBe(false);
    expect(parseError(403, '{"error":"only an agent may start","needsAgent":true}').needsAgent).toBe(true);
  });
  it('falls back to a status message for empty / non-JSON bodies', () => {
    expect(parseError(500, '').message).toBe('request failed (500)');
    expect(parseError(502, 'Bad gateway').message).toBe('Bad gateway');
    expect(parseError(500, '<html>' + 'x'.repeat(300)).message).toBe('request failed (500)');
  });
});

describe('api over the board transport', () => {
  it('sends method, daemon path and JSON body with the host id', async () => {
    const fake = fakeBoard({
      routes: { 'POST /api/tasks/my-proj/create': (req) => ({ json: task({ id: 'P-9', title: (req.json as { title: string }).title }) }) },
    });
    setBoardTransport(fake.transport);
    const created = await api.create('pc-1', 'my-proj', { title: 'Hi', status: 'todo', priority: 'high', labels: ['ui'] });
    expect(created.id).toBe('P-9');
    expect(fake.requests).toEqual([
      expect.objectContaining({
        hostId: 'pc-1',
        method: 'POST',
        path: '/api/tasks/my-proj/create',
        json: { title: 'Hi', status: 'todo', priority: 'high', labels: ['ui'] },
      }),
    ]);
  });

  it('encodes path segments and maps order targets to the daemon field names', async () => {
    const fake = fakeBoard({ routes: { 'POST /api/tasks/a%20b/X%2F1/order': () => ({ json: task({ id: 'X/1' }) }) } });
    setBoardTransport(fake.transport);
    await api.order('h', 'a b', 'X/1', { afterPos: 'X-2' });
    expect(fake.requests[0]!.json).toEqual({ after_pos: 'X-2' });
  });

  it('throws ApiError with the parsed flags on a refusal', async () => {
    const fake = fakeBoard({
      routes: { 'POST /api/tasks/p/T-1/move': () => ({ status: 422, json: { error: 'needs a comment', needsComment: true } }) },
    });
    setBoardTransport(fake.transport);
    await expect(api.move('h', 'p', 'T-1', 'todo')).rejects.toMatchObject({ status: 422, needsComment: true, message: 'needs a comment' });
  });

  it('turns a transport failure into ApiError status 0', async () => {
    const broken: BoardTransport = {
      availability: async () => ({ state: 'offline' }),
      request: async () => {
        throw new Error('host unreachable');
      },
      events: () => () => {},
    };
    setBoardTransport(broken);
    await expect(api.projects('h')).rejects.toMatchObject({ status: 0, message: 'host unreachable' });
  });

  it('uploads raw bytes and downloads files as Blobs', async () => {
    const fake = fakeBoard({
      routes: {
        'POST /api/tasks/p/T-1/attachments': () => ({ json: { name: 'abc-shot.png', markdown: '![shot](att:T-1/abc-shot.png)' } }),
        'GET /api/files/p/T-1/abc-shot.png': () => ({ text: 'PNGDATA' }),
      },
    });
    setBoardTransport(fake.transport);
    const attachment = await api.upload('h', 'p', 'T-1', 'shot.png', new Uint8Array([1, 2, 3]));
    expect(attachment.markdown).toContain('att:T-1');
    expect(fake.requests[0]!.path).toBe('/api/tasks/p/T-1/attachments?name=shot.png');
    expect(fake.requests[0]!.body).toBeInstanceOf(Uint8Array);
    const blob = await api.file('h', '/api/files/p/T-1/abc-shot.png');
    expect(blob.size).toBe(7);
  });

  it('covers every endpoint path', async () => {
    const seen: string[] = [];
    const fake = fakeBoard();
    const any = () => ({ json: {} });
    for (const key of [
      'GET /api/projects', 'GET /api/running', 'GET /api/rules', 'GET /api/projects/p/tasks', 'GET /api/tasks/p/T',
      'POST /api/tasks/p/T/undo', 'POST /api/tasks/p/T/note', 'POST /api/tasks/p/T/edit', 'POST /api/tasks/p/T/link',
      'POST /api/tasks/p/T/unlink', 'POST /api/tasks/p/T/duplicate', 'GET /api/settings', 'PUT /api/settings', 'GET /api/models',
      'POST /api/projects/p/archive-lane', 'POST /api/projects/p/done-lane', 'PUT /api/projects/p', 'POST /api/projects/p/summarize',
      'POST /api/projects/p/archive-summary',
    ]) {
      fake.routes[key] = (req) => {
        seen.push(`${req.method} ${req.path}`);
        return any();
      };
    }
    setBoardTransport(fake.transport);
    await Promise.all([
      api.projects('h'), api.running('h'), api.rules('h'), api.tasks('h', 'p'), api.task('h', 'p', 'T'),
      api.undoMove('h', 'p', 'T', 'tok'), api.note('h', 'p', 'T', 'n'), api.edit('h', 'p', 'T', { title: 't' }), api.link('h', 'p', 'T', 'D'),
      api.unlink('h', 'p', 'T', 'D'), api.duplicate('h', 'p', 'T'), api.settings('h'), api.saveSettings('h', { maxSessions: 3 }),
      api.models('h', true), api.archiveLane('h', 'p', 'done'), api.doneLane('h', 'p'), api.updateProject('h', 'p', { archived: true }),
      api.summarize('h', 'p', { scope: 'board', instruction: '  ' }), api.archiveSummary('h', 'p', { markdown: '# s', taskIds: ['T'] }),
    ]);
    expect(seen).toContain('GET /api/models?refresh=1');
    expect(fake.requests.find((r) => r.path.endsWith('/summarize'))!.json).toEqual({ scope: 'board' });
    expect(seen).toHaveLength(19);
  });
});

describe('rule helpers', () => {
  it('canMove prefers the task allowedMoves, then the rules table', () => {
    const rules = { ...PERMISSIVE_RULES, allowedMoves: { todo: ['backlog', 'cancelled'] } };
    expect(canMove(rules, task({ id: 'A', status: 'todo' }), 'backlog')).toBe(true);
    expect(canMove(rules, task({ id: 'A', status: 'todo' }), 'done')).toBe(false);
    expect(canMove(rules, task({ id: 'A', status: 'todo', allowedMoves: ['done'] }), 'done')).toBe(true);
    expect(canMove(rules, task({ id: 'A', status: 'blocked' }), 'done')).toBe(true);
    expect(needsComment({ ...rules, commentRequired: { in_progress: { todo: 'why' } } }, 'in_progress', 'todo')).toBe('why');
  });
  it('displayTitle falls back to the derived title', () => {
    expect(displayTitle({ title: '', displayTitle: 'From body' })).toBe('From body');
    expect(displayTitle({ title: '' })).toBe('(untitled)');
  });
});
