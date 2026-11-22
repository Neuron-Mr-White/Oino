import { z } from "zod";
import { FsPathSchema } from "./fs.js";

/**
 * Basic git on the PC (UNI-217). Mirrors `crates/proto/src/git.rs`:
 * `{op:"git", path, outside?, cmd:{action,…}}` → `ok{data}` per action.
 */

export const GIT_LOG_MAX = 200;
export const GIT_PATHS_MAX = 1000;
export const GIT_DIFF_CAP = 256 * 1024;

const BAD_REF_CHARS = /[\s~^:?*[\\\u0000-\u001f\u007f]/;
export const GitRefNameSchema = z
  .string()
  .min(1)
  .max(255)
  .refine(
    (v) =>
      !v.startsWith("-") && !v.startsWith("/") && !v.endsWith("/") && !v.endsWith(".") && !v.endsWith(".lock") &&
      !v.includes("..") && !v.includes("//") && !v.includes("@{") && !BAD_REF_CHARS.test(v),
    "not a valid branch name",
  );

export const GitPathSchema = z
  .string()
  .min(1)
  .max(4096)
  .refine((v) => !v.includes("\u0000"), "path must not contain NUL bytes")
  .refine((v) => !v.startsWith("/") && !v.split("/").includes(".."), "path must be relative to the repo, without `..`");

export const GitMessageSchema = z
  .string()
  .max(20000)
  .refine((v) => v.trim().length > 0, "commit message must not be empty");

const Paths = z.array(GitPathSchema).max(GIT_PATHS_MAX);

export const GitCmdSchema = z.discriminatedUnion("action", [
  z.object({ action: z.literal("repo") }),
  z.object({ action: z.literal("status") }),
  z.object({ action: z.literal("diff"), file: GitPathSchema, staged: z.boolean().optional(), untracked: z.boolean().optional() }),
  z.object({ action: z.literal("stage"), paths: Paths.optional(), all: z.boolean().optional() }),
  z.object({ action: z.literal("unstage"), paths: Paths.optional(), all: z.boolean().optional() }),
  z.object({ action: z.literal("discard"), paths: Paths, confirm: z.boolean().optional() }),
  z.object({ action: z.literal("commit"), message: GitMessageSchema, amend: z.boolean().optional() }),
  z.object({ action: z.literal("log"), skip: z.number().int().min(0).optional(), limit: z.number().int().min(1).max(GIT_LOG_MAX).optional() }),
  z.object({ action: z.literal("show"), rev: GitRefNameSchema }),
  z.object({ action: z.literal("branches") }),
  z.object({ action: z.literal("switch"), branch: GitRefNameSchema, create: z.boolean().optional() }),
  z.object({ action: z.literal("fetch") }),
  z.object({ action: z.literal("pull"), rebase: z.boolean().optional() }),
  z.object({ action: z.literal("push") }),
  z.object({ action: z.literal("stash_push"), message: GitMessageSchema.optional() }),
  z.object({ action: z.literal("stash_pop") }),
  // UNI-257 (proto 1.12.0): total +/- for the chat menu's Git row.
  z.object({ action: z.literal("shortstat") }),
  // v2 (host.md §9.4): the Git pane's "Last turn" source — status rows plus
  // each file's diff, restricted to the files the last turn touched.
  z.object({ action: z.literal("turn_files"), files: Paths, staged: z.boolean().optional(), untracked: z.boolean().optional() }),
  // v2 (WP-A15, Z-96 "Branch (vs base)"): files HEAD changed vs the base
  // ref (upstream, else main/master), rows plus each file's diff.
  z.object({ action: z.literal("branch_files"), base: GitRefNameSchema.optional() }),
]);
export type GitCmd = z.infer<typeof GitCmdSchema>;

export const GitRequestSchema = z.object({
  op: z.literal("git"),
  path: FsPathSchema,
  outside: z.boolean().optional(),
  cmd: GitCmdSchema,
});

export const GitRepoDataSchema = z.object({
  repo: z.boolean(),
  root: z.string().optional(),
  branch: z.string().optional(),
  head: z.string().optional(),
  detached: z.boolean().default(false),
  upstream: z.string().optional(),
  ahead: z.number().int().min(0).default(0),
  behind: z.number().int().min(0).default(0),
  remotes: z.array(z.string()).default([]),
  stashes: z.number().int().min(0).default(0),
  operation: z.string().optional(),
});
export type GitRepoData = z.infer<typeof GitRepoDataSchema>;

export const GitAreaSchema = z.enum(["staged", "unstaged", "untracked", "conflict"]);
export type GitArea = z.infer<typeof GitAreaSchema>;

export const GitFileChangeSchema = z.object({
  path: z.string(),
  orig: z.string().optional(),
  area: GitAreaSchema,
  status: z.string(),
  add: z.number().int().min(0).optional(),
  del: z.number().int().min(0).optional(),
  binary: z.boolean().default(false),
});
export type GitFileChange = z.infer<typeof GitFileChangeSchema>;

export const GitStatusDataSchema = GitRepoDataSchema.extend({
  files: z.array(GitFileChangeSchema),
  more: z.boolean().default(false),
});
export type GitStatusData = z.infer<typeof GitStatusDataSchema>;

export const GitDiffDataSchema = z.object({
  text: z.string(),
  truncated: z.boolean().default(false),
  binary: z.boolean().default(false),
});
export type GitDiffData = z.infer<typeof GitDiffDataSchema>;

/** One file of `turn_files`: its status row plus the diff of that change. */
export const GitTurnFileSchema = GitFileChangeSchema.extend({ diff: GitDiffDataSchema });
export type GitTurnFile = z.infer<typeof GitTurnFileSchema>;

/** `ok.data` for `turn_files` (v2 host.md §9.4). */
export const GitTurnFilesDataSchema = GitRepoDataSchema.extend({
  files: z.array(GitTurnFileSchema).default([]),
});
export type GitTurnFilesData = z.infer<typeof GitTurnFilesDataSchema>;

/** One file of `branch_files`: its change row vs the base plus the diff. */
export const GitBranchFileSchema = GitFileChangeSchema.extend({ diff: GitDiffDataSchema });
export type GitBranchFile = z.infer<typeof GitBranchFileSchema>;

/** `ok.data` for `branch_files` (v2 WP-A15, Z-96): `base` null = no base ref
 *  (no upstream, no main/master, no commits) — not an error. */
export const GitBranchFilesDataSchema = GitRepoDataSchema.extend({
  base: z.string().nullable().default(null),
  files: z.array(GitBranchFileSchema).default([]),
});
export type GitBranchFilesData = z.infer<typeof GitBranchFilesDataSchema>;

/** `ok.data` for `git_watch` (v2 WP-A15): the repo root the watcher watches
 *  (the root `git_changed` hub events carry); null = not a repo. */
export const GitWatchDataSchema = z.object({ root: z.string().nullable().default(null) });
export type GitWatchData = z.infer<typeof GitWatchDataSchema>;

/** `git_watch{root, on}` (v2 app.md §3.14): refcount the repo watcher. */
export const GitWatchRequestSchema = z.object({
  op: z.literal("git_watch"),
  root: FsPathSchema,
  on: z.boolean().default(true),
});
export type GitWatchRequest = z.infer<typeof GitWatchRequestSchema>;

export const GitCommitDataSchema = z.object({ hash: z.string(), subject: z.string() });
export type GitCommitData = z.infer<typeof GitCommitDataSchema>;

export const GitCommitInfoSchema = z.object({
  hash: z.string(),
  short: z.string(),
  subject: z.string(),
  author: z.string(),
  time: z.number().int(),
  refs: z.array(z.string()).default([]),
});
export type GitCommitInfo = z.infer<typeof GitCommitInfoSchema>;

export const GitLogDataSchema = z.object({ commits: z.array(GitCommitInfoSchema), more: z.boolean() });
export type GitLogData = z.infer<typeof GitLogDataSchema>;

export const GitShowFileSchema = z.object({
  path: z.string(),
  status: z.string(),
  add: z.number().int().min(0).optional(),
  del: z.number().int().min(0).optional(),
});
export const GitShowDataSchema = z.object({
  commit: GitCommitInfoSchema,
  body: z.string().default(""),
  files: z.array(GitShowFileSchema),
  diff: z.string(),
  truncated: z.boolean().default(false),
});
export type GitShowData = z.infer<typeof GitShowDataSchema>;

export const GitBranchSchema = z.object({
  name: z.string(),
  remote: z.boolean().default(false),
  current: z.boolean().default(false),
  upstream: z.string().optional(),
  ahead: z.number().int().min(0).default(0),
  behind: z.number().int().min(0).default(0),
  time: z.number().int().default(0),
  subject: z.string().default(""),
});
export type GitBranch = z.infer<typeof GitBranchSchema>;

export const GitBranchesDataSchema = z.object({ branches: z.array(GitBranchSchema) });
export type GitBranchesData = z.infer<typeof GitBranchesDataSchema>;

export const GitSyncDataSchema = z.object({ output: z.string() });

/** UNI-257: `shortstat` — total lines vs HEAD (staged + unstaged) plus
 *  untracked files' lines (≤ GIT_SHORTSTAT_UNTRACKED_MAX files; `capped`
 *  when there were more). `repo:false` outside a repo; zeros = clean. */
export const GIT_SHORTSTAT_UNTRACKED_MAX = 300;
export const GitShortstatDataSchema = z.object({
  repo: z.boolean(),
  files: z.number().int().min(0).default(0),
  add: z.number().int().min(0).default(0),
  del: z.number().int().min(0).default(0),
  capped: z.boolean().default(false),
});
export type GitShortstatData = z.infer<typeof GitShortstatDataSchema>;
export type GitSyncData = z.infer<typeof GitSyncDataSchema>;
