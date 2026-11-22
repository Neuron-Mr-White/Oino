/**
 * zod schemas for the app-level host/pairing commands (DESIGN.md §3.2,
 * §3.3, §3.10, §5.3). Every Tauri command result is parsed through one
 * of these before the UI ever touches it (DESIGN.md §5.5).
 */
import { z } from 'zod';

/** How a host was reached: the four routes iroh can take (DESIGN.md §5.3). */
export const PathKindSchema = z.enum(['lan', 'tailnet', 'direct', 'relay', 'local', 'unknown']);
export type PathKind = z.infer<typeof PathKindSchema>;

/** A paired host, as returned by `list_hosts` (DESIGN.md §3.10). */
export const HostInfoSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  ticket: z.string().min(1),
  paired_at: z.number(),
  last_path: PathKindSchema.optional().default('unknown'),
  /** UNI-231: 'local' = this PC's own host over its local socket (desktop only, not persisted). */
  transport: z.enum(['local', 'iroh']).optional(),
});
export type HostInfo = z.infer<typeof HostInfoSchema>;
