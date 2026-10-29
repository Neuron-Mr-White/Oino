import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import {
  ControlSchema,
  FsListDataSchema,
  FsReadDataSchema,
  FsWriteDataSchema,
  FsSearchDataSchema,
  FsRootsDataSchema,
  PROTO_VERSION,
} from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'FsListData' | 'FsReadData' | 'FsWriteData' | 'FsSearchData' | 'FsRootsData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(
  readFileSync(new URL('../../../protocol/fixtures/fs.json', import.meta.url), 'utf8'),
);
const schemas: Record<Schema, z.ZodTypeAny> = {
  Control: ControlSchema,
  FsListData: FsListDataSchema,
  FsReadData: FsReadDataSchema,
  FsWriteData: FsWriteDataSchema,
  FsSearchData: FsSearchDataSchema,
  FsRootsData: FsRootsDataSchema,
};

describe('filesystem browser/editor contract fixtures', () => {
  it('proto is at least 1.5.0 (fs ops)', () => expect(atLeast(PROTO_VERSION, '1.5.0')).toBe(true));
  it('has the full fixture set', () => expect(fixtures.length).toBeGreaterThanOrEqual(15));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success, JSON.stringify(result.error?.issues)).toBe(fixture.ok);
    });
  }
});
