import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, BoardHttpDataSchema, boardPathError, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'BoardHttpData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(
  readFileSync(new URL('../../../protocol/fixtures/board.json', import.meta.url), 'utf8'),
);
const schemas: Record<Schema, z.ZodTypeAny> = { Control: ControlSchema, BoardHttpData: BoardHttpDataSchema };

describe('board relay contract fixtures', () => {
  it('proto is at least 1.4.0 (board + chat + blob ops)', () => expect(atLeast(PROTO_VERSION, '1.4.0')).toBe(true));
  it('has the full fixture set', () => expect(fixtures.length).toBeGreaterThanOrEqual(30));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success, JSON.stringify(result.error?.issues)).toBe(fixture.ok);
    });
  }
});

describe('boardPathError', () => {
  it('allows api paths', () => {
    for (const path of ['/api/health', '/api/files/p/P-1/my file.png', '/api/tasks/p/P-1/attachments?name=a/../b']) {
      expect(boardPathError(path)).toBeNull();
    }
  });
  it('rejects everything else', () => {
    for (const path of ['/', '/events?project=x', '/api', '/api/./x', '/api/%2e%2E/x', '/api//x', '/api/x#y', '/api/x\u007f']) {
      expect(boardPathError(path)).not.toBeNull();
    }
  });
});
