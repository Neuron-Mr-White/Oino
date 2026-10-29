/**
 * Terminal session wire types (DESIGN.md §5.4): the `term_open`
 * result and the events streamed on `onEvent`. Every event from the
 * core is zod-parsed before the terminal sees it (DESIGN §5.5).
 */
import { z } from 'zod';
import { AppErrorSchema, type AppError } from '@unipi/protocol';
import { PathKindSchema, type PathKind } from './schemas';

export const TermEventSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('title'), title: z.string().min(1).max(512) }),
  z.object({ type: z.literal('exit'), code: z.number().int().nullable() }),
  z.object({
    type: z.literal('closed'),
    error: AppErrorSchema.optional(),
  }),
]);
export type TermEvent = z.infer<typeof TermEventSchema>;

export const TermOpenResultSchema = z.object({
  termId: z.string().min(1),
  effective_size_policy: z.enum(['fit_phone', 'keep_pc']),
  path: PathKindSchema,
});
export type TermOpenResult = z.infer<typeof TermOpenResultSchema>;

export type TermEventParsed =
  | { ok: true; event: TermEvent }
  | { ok: false; error: AppError };

export function parseTermEvent(raw: unknown): TermEventParsed {
  const parsed = TermEventSchema.safeParse(raw);
  if (parsed.success) return { ok: true, event: parsed.data };
  return {
    ok: false,
    error: {
      code: 'E_SCHEMA',
      message: 'Invalid terminal event from host',
      retryable: false,
    },
  };
}
