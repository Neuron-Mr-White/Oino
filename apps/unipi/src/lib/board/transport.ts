/**
 * Board transport v2 (WP-A22, G-18): the ONE seam between the Board UI and
 * the `unipi-kanboard` CLI, run by a host through the `board_cli` op
 * (app.md §3.14; ext-abi §5.5). The daemon relay of v1 is gone; there is no
 * web mode (a browser has no host — only the capture harness's mock answers).
 *
 * The UI never calls `invoke` directly: `cli()` goes through `lib/host`'s
 * HostClient (the mock host stands in for captures and tests), and live
 * changes arrive as `board_changed` hub events on the host event stream
 * (`board_watch{on}`, wired in live.ts) — not through this seam.
 */
import { hostClient } from '../host/factory';
import { toAppError, type AppError } from '@unipi/protocol';
import { BoardCliDataSchema, type BoardCliData, type BoardErrorKind } from '@unipi/protocol';

export type BoardAvailability =
  | { state: 'ok'; home?: string; version?: string }
  | { state: 'offline' }
  | { state: 'not_installed'; hint: string }
  | { state: 'error'; message: string };

/** The decoded §5.5 outcome of one `board_cli` call. */
export type BoardCliOutcome = BoardCliData;

export interface BoardTransport {
  /**
   * Runs one CLI call on the host: `unipi-kanboard --json --actor user
   * [--project <slug>] <args>`. Never throws for a CLI refusal — that is the
   * `{ok:false, error, kind}` outcome. Throws (`AppError`) only when the host
   * itself is unreachable or refuses the argv (allow-list).
   */
  cli(hostId: string, args: string[], project?: string): Promise<BoardCliOutcome>;
  /** Is the board usable on this PC (a `status` call)? */
  availability(hostId: string): Promise<BoardAvailability>;
}

/** Maps an `AppError` from the host op onto the availability chip's states. */
export function availabilityFromError(error: unknown): BoardAvailability {
  const app: AppError = error as AppError;
  const code = app?.code ?? 'E_INTERNAL';
  const reason = (app as { detail?: { reason?: string } })?.detail?.reason;
  if (code === 'E_BOARD_UNAVAILABLE' && reason === 'not_installed') {
    return { state: 'not_installed', hint: app.hint ?? 'Install the Unipi distribution on this PC (it ships unipi-kanboard)' };
  }
  if (code === 'E_TRANSPORT' || code === 'E_TIMEOUT') return { state: 'offline' };
  return { state: 'error', message: app?.message ?? String(error) };
}

const tauriTransport: BoardTransport = {
  async cli(hostId, args, project) {
    const value = await hostClient(hostId).request('board_cli', { args, ...(project ? { project } : {}) });
    const parsed = BoardCliDataSchema.safeParse(value);
    if (!parsed.success) return { ok: false, error: 'the host answered with an unknown board reply', kind: 'io' };
    return parsed.data;
  },
  async availability(hostId) {
    try {
      const outcome = await tauriTransport.cli(hostId, ['status']);
      if (!outcome.ok) return { state: 'error', message: outcome.error ?? 'unipi-kanboard refused to answer' };
      const payload = outcome.payload as { home?: string; binary?: string } | undefined;
      return { state: 'ok', home: payload?.home, version: undefined };
    } catch (error) {
      return availabilityFromError(toAppError(error));
    }
  },
};

let override: BoardTransport | undefined;
/** Tests inject a fake transport. */
export function setBoardTransport(transport: BoardTransport | undefined): void {
  override = transport;
}
export function boardTransport(): BoardTransport {
  return override ?? tauriTransport;
}

/** True when the outcome is a domain failure (never throws for these). */
export function boardFailed(outcome: BoardCliOutcome): outcome is { ok: false; error: string; kind: BoardErrorKind } {
  return !outcome.ok;
}
