import { z } from "zod";

/**
 * Filesystem browser/editor wire types (UNI-155 "files — browser +
 * editor"). Mirrors `crates/proto/src/fs.rs`.
 */

export const FsPathSchema = z
  .string()
  .min(1, "path must be at least 1 character")
  .max(4096, "path must be at most 4096 characters")
  .refine((v) => !v.includes("\u0000"), "path must not contain NUL bytes");

export const SearchQuerySchema = z
  .string()
  .min(1, "query must be at least 1 character")
  .max(512, "query must be at most 512 characters");

export const EntryKindSchema = z.enum(["file", "dir", "symlink"]);
export type EntryKind = z.infer<typeof EntryKindSchema>;

export const SearchKindSchema = z.enum(["name", "content"]);
export type SearchKind = z.infer<typeof SearchKindSchema>;

export const FS_LIST_CAP = 2000;
export const FS_SEARCH_CAP = 200;
export const FS_READ_CAP = 2 * 1024 * 1024;

export const FsEntrySchema = z.object({
  name: z.string(),
  kind: EntryKindSchema,
  size: z.number().int().min(0),
  mtime: z.number().int(),
  git: z.enum(["M", "A", "D", "?"]).optional(),
  ignored: z.boolean().default(false),
});
export type FsEntry = z.infer<typeof FsEntrySchema>;

export const FsListDataSchema = z.object({
  entries: z.array(FsEntrySchema),
  more: z.boolean(),
});
export type FsListData = z.infer<typeof FsListDataSchema>;

export const FsReadDataSchema = z.object({
  size: z.number().int().min(0),
  hash: z.string(),
  mtime: z.number().int(),
  truncated: z.boolean().default(false),
});
export type FsReadData = z.infer<typeof FsReadDataSchema>;

export const FsStatDataSchema = z.object({
  kind: EntryKindSchema,
  size: z.number().int().min(0),
  mtime: z.number().int(),
  git: z.enum(["M", "A", "D", "?"]).optional(),
});
export type FsStatData = z.infer<typeof FsStatDataSchema>;

export const FsWriteDataSchema = z.object({
  hash: z.string(),
  mtime: z.number().int(),
});
export type FsWriteData = z.infer<typeof FsWriteDataSchema>;

export const FsSearchResultSchema = z.object({
  path: z.string(),
  line: z.number().int().optional(),
  preview: z.string().optional(),
});
export type FsSearchResult = z.infer<typeof FsSearchResultSchema>;

export const FsSearchDataSchema = z.object({
  results: z.array(FsSearchResultSchema),
  more: z.boolean(),
});
export type FsSearchData = z.infer<typeof FsSearchDataSchema>;

export const FsRootsDataSchema = z.object({
  roots: z.array(z.string()),
});
export type FsRootsData = z.infer<typeof FsRootsDataSchema>;

export const FsConflictDetailSchema = z.object({
  hash: z.string(),
  mtime: z.number().int(),
});
export type FsConflictDetail = z.infer<typeof FsConflictDetailSchema>;
