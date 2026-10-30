/**
 * Per-file diagnostic badges (WP-A21): the pool funnels every
 * `textDocument/publishDiagnostics` it sees into this store; the file
 * tree reads it to badge rows (worst severity wins, errors over
 * warnings). `files.diag`-style — computed client-side because the host
 * doesn't run the servers, it only pipes them.
 */
import { createStore, produce } from 'solid-js/store';

/** Worst-severity diagnostic count per absolute path, per host. */
export type DiagSummary = { errors: number; warnings: number };

type State = {
  byPath: Record<string, DiagSummary>;
};

const [state, setState] = createStore<State>({ byPath: {} });

/** The badge a tree row shows (`undefined` = clean/none). */
export function diagFor(path: string): DiagSummary | undefined {
  return state.byPath[path];
}

/** LSP `publishDiagnostics.params` → the store. */
export function recordDiagnostics(uri: string, diagnostics: readonly { severity?: number }[]): void {
  const path = uri.startsWith('file://') ? decodeURIComponent(uri.slice('file://'.length)) : uri;
  let errors = 0;
  let warnings = 0;
  for (const diag of diagnostics) {
    if (diag.severity === 1) errors++;
    else if (diag.severity === 2) warnings++;
  }
  setState(produce((s) => {
    if (errors === 0 && warnings === 0) delete s.byPath[path];
    else s.byPath[path] = { errors, warnings };
  }));
}

/** Tests/scenarios reset. */
export function clearDiagnostics(): void {
  setState('byPath', {});
}
