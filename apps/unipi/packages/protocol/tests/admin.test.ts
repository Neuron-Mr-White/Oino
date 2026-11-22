import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, DevicesListDataSchema, HostInfoDataSchema, PairBeginDataSchema, PairStatusDataSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'HostInfoData' | 'PairBeginData' | 'PairStatusData' | 'DevicesListData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/admin.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = { Control: ControlSchema, HostInfoData: HostInfoDataSchema, PairBeginData: PairBeginDataSchema, PairStatusData: PairStatusDataSchema, DevicesListData: DevicesListDataSchema };

describe('local admin ops contract fixtures (UNI-232)', () => {
  it('proto is 1.14.0+', () => expect(atLeast(PROTO_VERSION, '1.14.0')).toBe(true));
  for (const fixture of fixtures) {
    it(fixture.name, () => expect(schemas[fixture.schema].safeParse(fixture.value).success).toBe(fixture.ok));
  }
});
