import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, CronJobSchema, CronSettingsDataSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'CronJob' | 'CronSettingsData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/cron.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = {
  Control: ControlSchema,
  CronJob: CronJobSchema,
  CronSettingsData: CronSettingsDataSchema,
};

describe('cron_* contract fixtures (UNI-317/UNI-318)', () => {
  it('proto is 1.19.0+ (cron; 1.20.0 adds cron_open_session + notify/wake)', () => expect(atLeast(PROTO_VERSION, '1.19.0')).toBe(true));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
});
