/**
 * Test helpers for the Board (UNI-130): an in-memory fake board
 * transport (`setBoardTransport(fake.transport)`) that routes daemon
 * paths to handlers and records every request, plus task fixtures.
 * Used by vitest only.
 */
import type { BoardAvailability, BoardEvent, BoardRequest, BoardResponse, BoardStreamState, BoardTransport } from './transport';
import type { ProjectSummary, Task } from './api';

export type Handler = (req: BoardRequest & { json?: unknown }, hostId: string) => { status?: number; json?: unknown; text?: string } | Promise<{ status?: number; json?: unknown; text?: string }>;

const enc = new TextEncoder();
const dec = new TextDecoder();

export type FakeBoardOptions = {
  routes?: Record<string, Handler>;
  availability?: Record<string, BoardAvailability>;
};

/** One `events()` subscription of the fake transport (UNI-132 tests drive it). */
export type FakeStream = {
  hostId: string;
  project: string;
  closed: boolean;
  /** Pushes an SSE message (default: a `revision` event). */
  revision: (data?: string) => void;
  emit: (event: BoardEvent) => void;
  state: (state: BoardStreamState, message?: string) => void;
};

export function fakeBoard(options: FakeBoardOptions = {}) {
  const routes: Record<string, Handler> = { ...(options.routes ?? {}) };
  const requests: Array<BoardRequest & { hostId: string; json?: unknown }> = [];
  const availability: Record<string, BoardAvailability> = { ...(options.availability ?? {}) };
  const streams: FakeStream[] = [];
  const transport: BoardTransport = {
    async availability(hostId) {
      return availability[hostId] ?? { state: 'ok', version: 'test' };
    },
    async request(hostId, req): Promise<BoardResponse> {
      const json = typeof req.body === 'string' ? JSON.parse(req.body) : undefined;
      requests.push({ ...req, hostId, json });
      const key = `${req.method} ${req.path.split('?')[0]}`;
      const handler = routes[key];
      if (!handler) return { status: 404, contentType: 'application/json', body: enc.encode(JSON.stringify({ error: `no route ${key}` })) };
      const result = await handler({ ...req, json }, hostId);
      const text = result.text ?? (result.json === undefined ? '' : JSON.stringify(result.json));
      return { status: result.status ?? 200, contentType: 'application/json', body: enc.encode(text) };
    },
    events(hostId, project, onEvent, onState) {
      const stream: FakeStream = {
        hostId,
        project,
        closed: false,
        emit: (event) => {
          if (!stream.closed) onEvent(event);
        },
        revision: (data = '1') => stream.emit({ event: 'revision', data }),
        state: (state, message) => {
          if (!stream.closed) onState(state, message);
        },
      };
      streams.push(stream);
      return () => {
        stream.closed = true;
      };
    },
  };
  /** Subscriptions still open. */
  const open = () => streams.filter((stream) => !stream.closed);
  return { transport, routes, requests, availability, streams, open, decode: (bytes: Uint8Array) => dec.decode(bytes) };
}

let seq = 0;
export function task(partial: Partial<Task> & { id: string }): Task {
  seq += 1;
  return {
    title: `Task ${partial.id}`,
    status: 'todo',
    priority: 'none',
    order: seq * 1000,
    deps: [],
    labels: [],
    created: `2026-10-0${1 + (seq % 8)}T10:00:00Z`,
    updated: '2026-10-07T10:00:00Z',
    activity: [],
    ...partial,
  };
}

export function project(partial: Partial<ProjectSummary> & { slug: string }): ProjectSummary {
  return { name: partial.slug, counts: {}, ...partial };
}
