import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, TreeDataSchema, PreviewDataSchema, PROTO_VERSION } from '../src/index.js';

type Schema='Control'|'TreeData'|'PreviewData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean; };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/m2.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = { Control: ControlSchema, TreeData: TreeDataSchema, PreviewData: PreviewDataSchema };

describe('M2 contract fixtures', () => {
  it('keeps proto major 1', () => expect(PROTO_VERSION.split('.')[0]).toBe('1'));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
});
