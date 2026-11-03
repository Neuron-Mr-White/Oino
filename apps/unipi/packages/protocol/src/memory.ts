/**
 * Settings › Memory › Browse memories (owner fix 2026-11-03): `memory_browse`
 * pages the memory extension's store on a host — newest first, optionally one
 * project and a word search — instead of loading every memory.
 * Mirrors `unipi_proto::memory`.
 */
import { z } from "zod";

export const MEMORY_PAGE_DEFAULT = 50;
export const MEMORY_PAGE_MAX = 100;

export const MemoryBrowseRequestSchema = z.object({
  op: z.literal("memory_browse"),
  project: z.string().min(1).optional(),
  query: z.string().max(256).optional(),
  limit: z.number().int().min(1).max(MEMORY_PAGE_MAX).optional(),
  cursor: z.string().optional(),
});

export const MemoryItemSchema = z
  .object({
    id: z.string(),
    title: z.string(),
    kind: z.string(),
    project: z.string(),
    updated_at: z.number(),
    body: z.string(),
    truncated: z.boolean().optional(),
  })
  .passthrough();
export type MemoryItem = z.infer<typeof MemoryItemSchema>;

export const MemoryProjectSchema = z.object({ name: z.string(), count: z.number().int().min(0) }).passthrough();
export type MemoryProject = z.infer<typeof MemoryProjectSchema>;

export const MemoryBrowseDataSchema = z
  .object({
    items: z.array(MemoryItemSchema),
    total: z.number().int().min(0),
    next_cursor: z.string().optional(),
    projects: z.array(MemoryProjectSchema).default([]),
    store: z.boolean(),
  })
  .passthrough();
export type MemoryBrowseData = z.infer<typeof MemoryBrowseDataSchema>;

/** The memory extension's project name for a folder (`files::project_name`). */
export function memoryProjectName(cwd: string): string {
  const base = cwd.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
  const runs = base.match(/[A-Za-z0-9]+/g) ?? [];
  const name = runs.join("_").toLowerCase();
  return name || "unknown";
}
