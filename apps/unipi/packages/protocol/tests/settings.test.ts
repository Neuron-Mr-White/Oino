import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, HostSettingsSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Fixture = { name: string; schema: string; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/settings.json', import.meta.url), 'utf8'));
const schemas: Record<string, z.ZodTypeAny> = {
  Control: ControlSchema,
  HostSettingsData: HostSettingsSchema,
};

describe('host settings contract fixtures (v2 host.md §9.9)', () => {
  it('proto is 2.0.0 (host settings)', () => {
    expect(atLeast(PROTO_VERSION, '2.0.0')).toBe(true);
  });
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const schema = schemas[fixture.schema];
      expect(schema, fixture.schema).toBeDefined();
      expect(schema!.safeParse(fixture.value).success).toBe(fixture.ok);
    });
  }
});
