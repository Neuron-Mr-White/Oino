import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, FeatureLeaveDataSchema, FeatureRequestsDataSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'FeatureRequestsData' | 'FeatureLeaveData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/sharing.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = { Control: ControlSchema, FeatureRequestsData: FeatureRequestsDataSchema, FeatureLeaveData: FeatureLeaveDataSchema };

describe('feature_request / feature_requests / feature_leave contract fixtures (UNI-248, UNI-253)', () => {
  it('proto is 1.11.0+ (feature requests)', () => expect(atLeast(PROTO_VERSION, '1.11.0')).toBe(true));
  it('proto is 1.15.0+ (feature_leave)', () => expect(atLeast(PROTO_VERSION, '1.15.0')).toBe(true));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
});
