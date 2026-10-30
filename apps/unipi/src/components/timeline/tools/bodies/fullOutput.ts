/**
 * Lazy full output (§3.4 step 7): a body that shows a trimmed tail can pull
 * the entry's complete result text with `get_entries{ids}` — remembered in
 * a small LRU so virtual-scroll remounts never re-ask.
 */
import { createSignal } from 'solid-js';
import type { ToolRun } from '../../../../lib/session/turns';
import type { TimelineCtx } from '../../context';
import { cachedFullOutput, rememberFullOutput, resultTextOf } from '../toolData';

export function useFullOutput(ctx: TimelineCtx, tool: ToolRun) {
  const [full, setFull] = createSignal<string | undefined>(tool.resultId ? cachedFullOutput(tool.resultId) : undefined);
  const load = () => {
    const id = tool.resultId;
    if (!id || full() !== undefined) return;
    ctx.session
      .getEntries([id])
      .then((entries) => {
        const text = resultTextOf(entries.find((e) => e.id === id));
        rememberFullOutput(id, text);
        setFull(text);
      })
      .catch(() => {
        /* offline: the in-memory tail stays */
      });
  };
  return { full, load };
}
