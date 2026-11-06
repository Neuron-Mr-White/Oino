/**
 * `turnFooter` slot host (WP-U10, A27's feeder; ui-protocol §6.7): renderer
 * entries render **inside the turn's virtual row**, after the turn tail —
 * one `_oino/render{target:"turnFooter"}` per (entry, turn) through the
 * session's render cache (keyed `(target, key, entryId, rev, width)`), so a
 * virtual-scroll remount never re-asks. `null` renders nothing (§3.4);
 * invalidations (§6.7) bump `footerRev` in the cache key; the running turn
 * re-asks at most every 500 ms (§6.7). Footers never virtualize internally
 * and the row's height change re-measures only that turn (§13.4).
 */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import type { Turn } from '../../lib/session/turns';
import { sessionRenderCache, widthClassOf } from '../../lib/session/renderCache';
import { NodeView } from '../NodeView';
import { EntryView } from '../NodeView';
import { sessionUi } from '../registries';
import { isReservedEntry, reservedBodyOf, reservedRowFor } from './reserved';
import { layout } from '../../lib/layout';
import { footerRev } from './effects';
import type { TimelineCtx } from '../../components/timeline/context';
import type { UiEntry } from '../types';

const RUN_REFRESH_MS = 500;

function rendererKey(entry: UiEntry): string {
  return entry.view.kind === 'renderer' ? entry.view.key : entry.id;
}

/** `unipi.file-summary` registers only its terminal-style `files` renderer; the app draws its own
 *  file summary for it (owner 2026-11-06: the node tree looked wrong in a chat). */
const isFileSummary = (entry: UiEntry) => entry.extensionId === 'unipi.file-summary' && rendererKey(entry) === 'files';

export function TurnFooters(props: { ctx: TimelineCtx; turn: Turn }) {
  const registry = () => sessionUi(props.ctx.hostId, props.ctx.sessionId());
  // The phone shows no file summary under a turn (owner 2026-11-06): the Git view has it.
  const entries = () => registry().entriesFor('turnFooter')().filter((e) => layout().desktop() || (!isFileSummary(e) && reservedRowFor(e)?.component !== 'turnFiles'));
  const [tick, setTick] = createSignal(0);

  // The running turn re-requests at most every 500 ms while it changes (§6.7).
  // perf-app: the timer exists only while the turn is live (a settled turn's footer never wakes).
  createEffect(() => {
    if (!props.turn.live) return;
    const timer = setInterval(() => setTick((t) => t + 1), RUN_REFRESH_MS);
    onCleanup(() => clearInterval(timer));
  });

  const cache = () => sessionRenderCache(props.ctx.session);
  const width = () => widthClassOf(props.ctx.columnWidth());

  return (
    <Show when={entries().length > 0}>
      <div class="ui-turnfooters" data-testid="ui-turnfooters" data-turn={props.turn.turnId}>
        <For each={entries()}>
          {(entry) => {
            // §8 (U12): a reserved row renders its native component — or the owner's
            // fallback tree — right here; only `renderer` entries are pulled.
            if (isReservedEntry(entry))
              return (
                <div class="ui-turnfooter" data-entry={entry.id}>
                  <EntryView
                    entry={entry}
                    target={{ scope: 'session', hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), turnEntryId: props.turn.turnId }}
                    mode="auto"
                  />
                </div>
              );
            if (isFileSummary(entry)) {
              const Native = reservedBodyOf('turnFiles');
              if (Native)
                return (
                  <div class="ui-turnfooter" data-entry={entry.id}>
                    <Native entry={entry} target={{ scope: 'session', hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), turnEntryId: props.turn.turnId }} />
                  </div>
                );
            }
            // The running turn re-asks every 500 ms under a new cache key: keep showing the last
            // answer until the next one lands, or the footer blinks out on every tick.
            let last: ReturnType<ReturnType<ReturnType<typeof cache>['node']>> = undefined;
            const node = () => {
              const fresh = cache().node(
                { target: 'turnFooter', key: rendererKey(entry), entryId: entry.id, rev: (entry.rev ?? 0) + footerRev(entry.id) + tick(), width: width() },
                { turnEntryId: props.turn.turnId, running: props.turn.live, density: 'regular', data: null },
              )();
              if (fresh !== undefined) last = fresh;
              return fresh ?? last;
            };
            return (
              <Show when={node()} keyed>
                {(n) => (
                  <div class="ui-turnfooter" data-entry={entry.id}>
                    <NodeView node={n} path={['turnFooter']} mode="auto" />
                  </div>
                )}
              </Show>
            );
          }}
        </For>
      </div>
    </Show>
  );
}
