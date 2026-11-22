import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, NotifyConfigDataSchema } from '../src/index.js';

type Schema = 'Control' | 'NotifyConfigData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/notify.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = { Control: ControlSchema, NotifyConfigData: NotifyConfigDataSchema };

describe('notify_config / notify_register / notify_test contract fixtures', () => {
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
});
