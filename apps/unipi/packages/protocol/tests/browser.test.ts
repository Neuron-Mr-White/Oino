import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { BrowserOpenDataSchema, BrowserProfilesDataSchema, BrowserStatusDataSchema, ControlSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'BrowserStatusData' | 'BrowserOpenData' | 'BrowserProfilesData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/browser.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = {
  Control: ControlSchema,
  BrowserStatusData: BrowserStatusDataSchema,
  BrowserOpenData: BrowserOpenDataSchema,
  BrowserProfilesData: BrowserProfilesDataSchema,
};

describe('browser_* contract fixtures (UNI-215)', () => {
  it('proto is 1.8.0+ (remote browser)', () => expect(atLeast(PROTO_VERSION, '1.8.0')).toBe(true));
  it('proto is 1.16.0+ (UNI-292 engine/stealth/profile)', () => expect(atLeast(PROTO_VERSION, '1.16.0')).toBe(true));
  it('proto is 1.17.0+ (UNI-296 agent / takeover / profiles)', () => expect(atLeast(PROTO_VERSION, '1.17.0')).toBe(true));
  it('proto is 1.18.0+ (UNI-299 hidden display / stealth_note)', () => expect(atLeast(PROTO_VERSION, '1.18.0')).toBe(true));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
});
