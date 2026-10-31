/**
 * Lazy full output (§3.4 step 7): a body that shows a trimmed tail/head can
 * pull the entry's complete result text with `get_entries{ids}` — remembered
 * in a small LRU so virtual-scroll remounts never re-ask. Never replaces the
 * already-rendered truncated text while loading; exposes `loading` for an
 * inline "Loading full output…" affordance.
 */
import { batch, createSignal } from 'solid-js';
import type { ToolRun } from '../../../../lib/session/turns';
import type { TimelineCtx } from '../../context';
import { cachedFullOutput, rememberFullOutput, resultEntryOf, resultTextOf } from '../toolData';

export function useFullOutput(ctx: TimelineCtx, tool: () => ToolRun) {
  const initial = () => {
    const id = tool().resultId;
    return id ? cachedFullOutput(id) : undefined;
  };
  const [fetched, setFetched] = createSignal<string | undefined>(initial());
  const [loading, setLoading] = createSignal(false);

  const full = () => fetched() ?? initial();

  /** True when the host snapshot marked the tool result entry/details as truncated. */
  const snapshotTruncated = () => {
    const t = tool();
    const det = t.details as { truncated?: unknown } | null | undefined;
    if (det?.truncated === true) return true;
    const entry = resultEntryOf(ctx, t);
    const msg = entry?.message as { truncated?: unknown } | undefined;
    return msg?.truncated === true || (entry as { truncated?: unknown } | undefined)?.truncated === true;
  };

  const load = () => {
    const id = tool().resultId;
    if (!id || full() !== undefined || loading()) return;
    setLoading(true);
    ctx.session
      .getEntries([id])
      .then((entries) => {
        const found = entries.find((e) => e.id === id);
        batch(() => {
          if (found) {
            const text = resultTextOf(found);
            rememberFullOutput(id, text);
            setFetched(text);
          }
          setLoading(false);
        });
      })
      .catch(() => {
        /* offline: the in-memory tail stays */
        setLoading(false);
      });
  };

  return { full, loading, snapshotTruncated, load };
}
