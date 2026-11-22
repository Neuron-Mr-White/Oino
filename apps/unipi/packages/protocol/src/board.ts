/**
 * Board wire types, v2 (app.md §3.14 "Board" delta, WP-A22; ext-abi §5.5).
 * Mirrors `apps/unipi/crates/unipi-proto/src/board.rs`.
 *
 * The v1 daemon relay (`board_http`/`board_sse`) is gone: the Board talks to
 * the `unipi-kanboard` **CLI** through the host. `board_cli{args, project}`
 * runs one process per call (`--json`, actor pinned to `user`, subcommand
 * allow-list); `board_watch{on}` turns the file watcher on and
 * `HubEvent{kind:"board_changed"}` (session.ts) is its refresh signal.
 */
import { z } from "zod";

/// One argv element of a `board_cli` call.
export const BOARD_ARG_MAX = 1024;
/// Most arguments one call may carry.
export const BOARD_ARGS_MAX = 64;
/// A project slug pinned per view (`--project <slug>`).
export const BOARD_SLUG_MAX = 128;

/** The CLI's failure vocabulary (ext-abi §5.5): `usage` (exit 2), the rest exit 1. */
export const BoardErrorKindSchema = z.enum(["usage", "not_found", "rule", "io"]);
export type BoardErrorKind = z.infer<typeof BoardErrorKindSchema>;

/**
 * `ok.data` of `board_cli`: the §5.5 contract decoded by the host — a
 * success (exit 0, one JSON value on stdout) or a failure (exit 1/2, the
 * `{ok:false, error, kind}` stderr line — a domain failure the app surfaces,
 * not a transport failure). Wire shape: one object with `ok` plus the
 * matching fields.
 */
export const BoardCliDataSchema = z.union([
  z.object({ ok: z.literal(true), payload: z.unknown().optional() }),
  z.object({
    ok: z.literal(false),
    error: z.string().optional(),
    kind: BoardErrorKindSchema.optional(),
  }),
]);
export type BoardCliData = z.infer<typeof BoardCliDataSchema>;

/** `ok.data` of `board_watch`: whether the watcher is on after the call. */
export const BoardWatchDataSchema = z.object({ watching: z.boolean() });
export type BoardWatchData = z.infer<typeof BoardWatchDataSchema>;

/**
 * Validates one `board_cli` argument: non-empty, ≤ [`BOARD_ARG_MAX`] bytes,
 * printable. Returns the reason, or null when allowed.
 */
export function boardArgError(value: string): string | null {
  if (value.length === 0) return "board_cli arguments must not be empty";
  if (value.length > BOARD_ARG_MAX)
    return `board_cli arguments must be at most ${BOARD_ARG_MAX} bytes, got ${value.length}`;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f-\u009f]/.test(value)) return "board_cli arguments must be printable";
  return null;
}

/**
 * Validates a `board_cli` argv (subcommand first, bounded length), the way
 * `serde_args` does on the Rust side. Returns the reason, or null.
 */
export function boardArgsError(args: string[]): string | null {
  if (args.length === 0) return "board_cli needs a subcommand";
  if (args.length > BOARD_ARGS_MAX)
    return `board_cli takes at most ${BOARD_ARGS_MAX} arguments, got ${args.length}`;
  for (const arg of args) {
    const reason = boardArgError(arg);
    if (reason) return reason;
  }
  return null;
}

/** A project slug pinned per view (`--project <slug>`). */
export const BoardSlugSchema = z
  .string()
  .min(1, "board project must be at least 1 character")
  .max(BOARD_SLUG_MAX, `board project must be at most ${BOARD_SLUG_MAX} characters`)
  .refine((value) => !/[\u0000-\u001f\u007f-\u009f]/.test(value), "board project must be printable");
export type BoardSlug = z.infer<typeof BoardSlugSchema>;
