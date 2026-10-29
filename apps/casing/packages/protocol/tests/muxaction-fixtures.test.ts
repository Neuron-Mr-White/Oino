import { readFileSync } from 'node:fs';
import { expect, test } from 'vitest';
import { MuxActionSchema, RequestSchema } from '../src/control.js';

const valid = JSON.parse(readFileSync('../../protocol/fixtures/control.valid.json', 'utf8'));
const invalid = JSON.parse(readFileSync('../../protocol/fixtures/control.invalid.json', 'utf8'));

test('mux_action valid fixtures parse', () => {
  for (const f of valid.filter((x: { value: { req?: { op?: string } } }) => x.value.req?.op === 'mux_action')) {
    expect(RequestSchema.safeParse(f.value.req).success, f.name).toBe(true);
  }
});
test('mux_action invalid fixtures reject', () => {
  for (const f of invalid.filter((x: { value: { req?: { op?: string } } }) => x.value.req?.op === 'mux_action')) {
    expect(RequestSchema.safeParse(f.value.req).success, f.name).toBe(false);
  }
});
test('MuxActionSchema alone still accepts a bare action', () => {
  expect(MuxActionSchema.safeParse({ action: 'tab_next', session: 'default' }).success).toBe(true);
});
