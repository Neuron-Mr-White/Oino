/**
 * zod schemas for the M1 host/pairing/session commands (DESIGN.md §3.2,
 * §3.3, §3.10, §5.3). Every Tauri command result is parsed through one
 * of these before the UI ever touches it (DESIGN.md §5.5).
 */
import { z } from 'zod';
import { PreviewDataSchema, TreeDataSchema } from '@unipi/protocol';

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

/** One herdr/tmux session entry, as listed by `list_sessions` (DESIGN.md §2). */
export const SessionEntrySchema = z.object({
  name: z.string().min(1),
  running: z.boolean(),
  default: z.boolean(),
});
export type SessionEntry = z.infer<typeof SessionEntrySchema>;

/** The result of `list_sessions`: the session list plus the reachable path. */
export const SessionsResultSchema = z.object({
  sessions: z.array(SessionEntrySchema),
  path: PathKindSchema,
});
export type SessionsResult = z.infer<typeof SessionsResultSchema>;

/** The result of `host_status`: reachability plus optional round-trip time. */
export const HostStatusSchema = z.object({
  path: PathKindSchema,
  rtt_ms: z.number().nullable().optional(),
});
export type HostStatus = z.infer<typeof HostStatusSchema>;

/** Result of `mux_tree`: both multiplexer trees plus the live path. */
export const MuxTreeResultSchema = z.object({
  tree: TreeDataSchema,
  path: PathKindSchema,
});
export type MuxTreeResult = z.infer<typeof MuxTreeResultSchema>;

/** Result of `mux_preview`: recent pane text (ANSI kept). */
export const MuxPreviewResultSchema = z.object({
  text: z.string().max(64 * 1024),
});
export type MuxPreviewResult = z.infer<typeof MuxPreviewResultSchema>;
