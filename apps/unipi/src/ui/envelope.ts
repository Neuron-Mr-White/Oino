/**
 * zod for the registry envelopes only (ui-protocol D-U14, §13.1): `UiSnapshot`,
 * `UiPatch` and the `RegistryOp` / `UiEntry` frames. Node trees stay unparsed —
 * core is the validator of record and zod on 10k-node trees is too slow on
 * phones.
 */
import { z } from 'zod';
import { UiPatchEnvelopeSchema, UiSnapshotEnvelopeSchema } from '@unipi/protocol';
import type { RegistryOp, UiEntry, UiPatch, UiRequest, UiSnapshot } from './types';

const SLOTS = [
  'overlay', 'sidePane', 'bottomPane', 'editorReplace', 'fullscreen', 'widget', 'widgetBelow', 'status', 'turnFooter',
  'header', 'footer', 'settings',
] as const;

export const UiEntrySchema = z
  .object({
    id: z.string().min(1),
    extensionId: z.string().default(''),
    scope: z.enum(['session', 'host']).default('session'),
    slot: z.enum(SLOTS),
    order: z.number().default(0),
    seq: z.number().default(0),
    view: z.object({ kind: z.enum(['tree', 'lines', 'status', 'reserved', 'renderer', 'settings']) }).passthrough(),
    rev: z.number().default(0),
  })
  .passthrough();

const RequestSchema = z.object({ id: z.union([z.string(), z.number()]).transform(String), method: z.string() }).passthrough();

export const RegistryOpSchema = z.discriminatedUnion('op', [
  z.object({ op: z.literal('put'), entry: UiEntrySchema }).passthrough(),
  z.object({ op: z.literal('del'), id: z.string() }).passthrough(),
  z.object({ op: z.literal('meta'), id: z.string(), set: z.record(z.string(), z.unknown()) }).passthrough(),
  z.object({ op: z.literal('tree'), id: z.string(), rev: z.number(), ops: z.array(z.object({ op: z.string() }).passthrough()) }).passthrough(),
  z.object({ op: z.literal('invalidate'), id: z.string(), turns: z.array(z.string()).optional() }).passthrough(),
  z.object({ op: z.literal('dialog'), request: RequestSchema }).passthrough(),
  z.object({ op: z.literal('dismiss'), id: z.union([z.string(), z.number()]).transform(String) }).passthrough(),
  z.object({ op: z.literal('sessionMeta'), set: z.record(z.string(), z.unknown()) }).passthrough(),
  z.object({ op: z.literal('effect'), request: RequestSchema }).passthrough(),
]);

export type EnvelopeError = { error: string };

/** Parses a `UiSnapshot` envelope; entries are checked as envelopes, their views stay raw. */
export function parseSnapshot(raw: unknown): UiSnapshot | EnvelopeError {
  const env = UiSnapshotEnvelopeSchema.safeParse(raw);
  if (!env.success) return { error: env.error.message };
  const entries: UiEntry[] = [];
  for (const e of env.data.entries) {
    const parsed = UiEntrySchema.safeParse(e);
    if (parsed.success) entries.push(parsed.data as unknown as UiEntry);
  }
  const dialogs: UiRequest[] = [];
  for (const d of env.data.dialogs) {
    const parsed = RequestSchema.safeParse(d);
    if (parsed.success) dialogs.push(parsed.data as UiRequest);
  }
  return {
    scope: env.data.scope,
    sessionId: env.data.sessionId,
    rev: env.data.rev,
    entries,
    dialogs,
    meta: (env.data.meta ?? {}) as UiSnapshot['meta'],
  };
}

/** Parses a `UiPatch` envelope; an unreadable op fails the whole patch (⇒ resync). */
export function parsePatch(raw: unknown): UiPatch | EnvelopeError {
  const env = UiPatchEnvelopeSchema.safeParse(raw);
  if (!env.success) return { error: env.error.message };
  const ops: RegistryOp[] = [];
  for (const op of env.data.ops) {
    const parsed = RegistryOpSchema.safeParse(op);
    if (!parsed.success) return { error: `bad registry op: ${parsed.error.issues[0]?.message ?? 'unreadable'}` };
    ops.push(parsed.data as unknown as RegistryOp);
  }
  return { scope: env.data.scope, sessionId: env.data.sessionId, rev: env.data.rev, ops };
}

export function isEnvelopeError(v: unknown): v is EnvelopeError {
  return typeof v === 'object' && v !== null && 'error' in v && !('rev' in v);
}
