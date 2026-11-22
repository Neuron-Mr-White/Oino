// @vitest-environment jsdom
/**
 * WP-U11 acceptance: the OSS-2 semantics port (ui-protocol §7.1–7.4) — layered
 * resolve with invalid-value fallback + diagnostics, simplify (a default write
 * is an unset), `visibleIf` / `disabledIf`, validation and `toZod`, against
 * `oino_ui::settings`'s rules (crates/oino-ui/src/settings/semantics.rs).
 */
import { describe, expect, it } from 'vitest';
import { parseField, parseNamespace, type Namespace } from '../types';
import {
  checkValue, condEval, fieldEnabled, fieldVisible, jsonEqual, merge, resolve,
  setPath, simplify, simplifyKey, toZod, zodMessage,
} from '../semantics';

function namespace(overrides: Record<string, unknown> = {}): Namespace {
  return parseNamespace({
    namespace: 'demo',
    title: 'Demo',
    extensionId: 'unipi.demo',
    defaults: { second: 5 },
    sections: [
      {
        id: 'main',
        title: 'Main',
        fields: [
          { key: 'first', type: 'string', label: 'First', default: 'a' },
          { key: 'second', type: 'number', label: 'Second', default: 1, min: 0 },
          { key: 'both', type: 'boolean', label: 'Both', default: false, scope: 'both' },
          { key: 'only', type: 'string', label: 'Only', scope: 'global' },
        ],
      },
    ],
    ...overrides,
  })!;
}

describe('OSS-2 resolve', () => {
  it('layers defaults ← namespace defaults ← global ← project', () => {
    const ns = namespace();
    const { values } = resolve(ns, { global: { first: 'g' }, project: { second: 9 } });
    expect(values.first).toBe('g');
    expect(values.second).toBe(9);
    expect(values.both).toBe(false);
  });

  it('an invalid stored value falls back a layer and reports a diagnostic', () => {
    const ns = namespace();
    const { values, diagnostics } = resolve(ns, { global: { second: -3 } });
    // `second` is below its min: the namespace default (5) wins, not the field default (1).
    expect(values.second).toBe(5);
    expect(diagnostics).toEqual([{ namespace: 'demo', key: 'second', message: 'must be at least 0' }]);
  });

  it('scope limits which layers a field reads', () => {
    const ns = namespace();
    const { values } = resolve(ns, { global: { only: 'g' }, project: { only: 'p' } });
    // `only` is global-scoped: the project write is invisible to it.
    expect(values.only).toBe('g');
  });

  it('a key that is invalid everywhere and has no default is unset', () => {
    const ns = parseNamespace({
      namespace: 'demo',
      sections: [{ id: 's', title: 'S', fields: [{ key: 'n', type: 'number', label: 'N', min: 10 }] }],
    })!;
    const { values } = resolve(ns, { global: { n: 3 } });
    expect('n' in values).toBe(false);
  });
});

describe('OSS-2 simplify', () => {
  it('writes only keys that differ from the layer below', () => {
    const ns = namespace();
    const writes = simplify(ns, 'global', { defaults: ns.defaults, global: {}, project: {} }, { first: 'a', second: 7, both: false });
    expect(writes).toEqual([
      { key: 'first', value: undefined },
      { key: 'second', value: 7 },
      { key: 'both', value: undefined },
    ]);
  });

  it('a project write compares against defaults ⊕ global', () => {
    const ns = namespace();
    const writes = simplify(ns, 'project', { defaults: ns.defaults, global: { second: 9 }, project: {} }, { first: 'g', second: 9, both: true });
    expect(writes.find((write) => write.key === 'second')?.value).toBeUndefined();
    expect(writes.find((write) => write.key === 'both')?.value).toBe(true);
  });

  it('simplifyKey: the inherited value unsets the key, a different one stores it', () => {
    const field = parseField({ key: 'k', type: 'number', label: 'K', default: 4 });
    expect(simplifyKey(field, undefined, 4)).toBeUndefined();
    expect(simplifyKey(field, 9, 9)).toBeUndefined();
    expect(simplifyKey(field, 9, 4)).toBe(4);
  });

  it('scope decides the layers a write may target', () => {
    const ns = namespace();
    const globalWrites = simplify(ns, 'global', { defaults: ns.defaults }, { first: 'a', only: 'x' }).map((write) => write.key);
    const projectWrites = simplify(ns, 'project', { defaults: ns.defaults, global: {} }, { first: 'a', only: 'x' }).map((write) => write.key);
    expect(globalWrites).toContain('only');
    expect(projectWrites).not.toContain('only');
  });
});

describe('OSS-2 conditions', () => {
  const cond = parseField({ key: 'k', type: 'boolean', label: 'K', visibleIf: { key: 'mode', eq: 'on' } });

  it('eq / ne / in / truthy evaluate on the effective values', () => {
    expect(condEval({ key: 'x', eq: 2 }, { x: 2 })).toBe(true);
    expect(condEval({ key: 'x', ne: 2 }, { x: 2 })).toBe(false);
    expect(condEval({ key: 'x', in: ['a', 'b'] }, { x: 'b' })).toBe(true);
    expect(condEval({ key: 'x', truthy: true }, { x: [] })).toBe(false);
    expect(condEval({ key: 'x', truthy: true }, { x: [1] })).toBe(true);
  });

  it('visibleIf hides a row; disabledIf keeps it but greys it', () => {
    expect(fieldVisible(cond, { mode: 'on' })).toBe(true);
    expect(fieldVisible(cond, { mode: 'off' })).toBe(false);
    const disabled = parseField({ key: 'k', type: 'boolean', label: 'K', disabledIf: { key: 'off', eq: true } });
    expect(fieldEnabled(disabled, { off: true })).toBe(false);
    expect(fieldEnabled(disabled, { off: false })).toBe(true);
    // No condition at all: enabled (and invisible still means not editable).
    expect(fieldEnabled(parseField({ key: 'k', type: 'boolean', label: 'K' }), {})).toBe(true);
    expect(fieldEnabled(cond, { mode: 'off' })).toBe(false);
  });

  it('a nested dot path resolves through objects', () => {
    const values: Record<string, unknown> = {};
    setPath(values, 'a.b.c', 3);
    expect(condEval({ key: 'a.b.c', eq: 3 }, values)).toBe(true);
  });
});

describe('OSS-2 validation', () => {
  it('checks the type, enum membership, lengths and ranges', () => {
    expect(checkValue(parseField({ key: 'b', type: 'boolean', label: 'B' }), 'yes')).toBe('expected a boolean');
    expect(checkValue(parseField({ key: 'e', type: 'enum', label: 'E', options: ['a', 'b'] }), 'c')).toBe('expected one of a, b');
    expect(checkValue(parseField({ key: 's', type: 'string', label: 'S', maxLength: 2 }), 'abcd')).toBe('must be at most 2 characters');
    expect(checkValue(parseField({ key: 'n', type: 'number', label: 'N', integer: true }), 1.5)).toBe('expected an integer');
    expect(checkValue(parseField({ key: 'm', type: 'multiselect', label: 'M', options: ['a'], max: 1 }), ['a', 'a'])).toBe('must have at most 1 items');
  });

  it('toZod mirrors the same rules for instant feedback', () => {
    const field = parseField({ key: 'p', type: 'string', label: 'P', pattern: '^[a-z]+$' });
    expect(zodMessage(toZod(field), 'abc')).toBeUndefined();
    expect(zodMessage(toZod(field), 'ABC')).toBeTruthy();
    const number = parseField({ key: 'n', type: 'number', label: 'N', min: 2, max: 4 });
    expect(zodMessage(toZod(number), 5)).toContain('4');
  });

  it('jsonEqual compares objects and arrays structurally', () => {
    expect(jsonEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toBe(true);
    expect(jsonEqual([1, 2], [2, 1])).toBe(false);
    expect(merge({ a: { b: 1, c: 2 } }, { a: { c: 3 } })).toEqual({ a: { b: 1, c: 3 } });
  });
});

describe('OSS-2 tolerant parsing', () => {
  it('v1 namespaces load: bare-string options, flat fields, missing titles', () => {
    const ns = parseNamespace({
      namespace: 'memory',
      fields: [{ key: 'model', type: 'string' }, { key: 'mode', type: 'enum', options: ['a', 'b'] }],
    })!;
    expect(ns.title).toBe('memory');
    expect(ns.sections).toHaveLength(1);
    expect(ns.sections[0]!.fields[0]!.label).toBe('model');
    expect(ns.sections[0]!.fields[1]!.options).toEqual([{ value: 'a', label: 'a' }, { value: 'b', label: 'b' }]);
  });

  it('an unknown field type stays, read-only; a bad namespace name is dropped', () => {
    expect(parseField({ key: 'x', type: 'quantum', label: 'X' }).type).toBe('unknown');
    expect(parseNamespace({ namespace: 'Bad Name' })).toBeUndefined();
    expect(parseNamespace({ namespace: 'ok.name' })!.title).toBe('ok.name');
  });

  it('v1 order items (objects) keep their value', () => {
    const field = parseField({ key: 'o', type: 'order', label: 'O', items: [{ value: 'a', label: 'A' }, 'b'] });
    expect(field.items).toEqual(['a', 'b']);
  });
});
