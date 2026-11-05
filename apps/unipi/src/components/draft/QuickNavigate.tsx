/**
 * Quick navigate (owner 2026-11-05): type a few letters, get the folder `z` (zoxide) would
 * jump to on this PC, confirm it. Nothing is picked without the confirm.
 */
import { createSignal, For, Show } from 'solid-js';
import { Folder, Zap } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { localZoxide, type ZoxideHit } from '../../lib/openLocal';

export function QuickNavigate(props: { onUse: (path: string) => void; onClose: () => void; short?: (path: string) => string }) {
  const [query, setQuery] = createSignal('');
  const [hits, setHits] = createSignal<ZoxideHit[] | undefined>();
  const [chosen, setChosen] = createSignal<string | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  const [busy, setBusy] = createSignal(false);
  const find = async () => {
    if (!query().trim() || busy()) return;
    setBusy(true);
    setError(undefined);
    const r = await localZoxide(query());
    setBusy(false);
    if (!r.ok) {
      setHits(undefined);
      setError(r.message);
      return;
    }
    setHits(r.hits);
    setChosen(r.hits[0]?.path);
    if (!r.hits.length) setError(`z found nothing for “${query().trim()}”.`);
  };
  return (
    <BoardSheet
      open
      onClose={props.onClose}
      label="Quick navigate"
      testId="quick-navigate"
      footer={
        <div class="qn-foot">
          <button type="button" class="btn-secondary settings-btn" onClick={props.onClose}>Cancel</button>
          <button type="button" class="btn-primary settings-btn" disabled={!chosen()} onClick={() => chosen() && props.onUse(chosen()!)} data-testid="quick-navigate-confirm">
            Use this folder
          </button>
        </div>
      }
    >
      <form
        class="qn-form"
        onSubmit={(e) => {
          e.preventDefault();
          void find();
        }}
      >
        <input class="qn-input" autofocus placeholder="Like z: proj oino" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} aria-label="Folder to jump to" data-testid="quick-navigate-input" />
        <button type="submit" class="btn-secondary settings-btn" disabled={!query().trim() || busy()} data-testid="quick-navigate-find">
          <Zap size={13} /> {busy() ? 'Finding…' : 'Find'}
        </button>
      </form>
      <Show when={error()}>
        <p class="qn-note" role="alert">{error()}</p>
      </Show>
      <Show when={hits()?.length}>
        <ul class="qn-list" role="listbox" aria-label="Suggestions">
          <For each={hits()}>
            {(h, i) => (
              <li>
                <button type="button" role="option" aria-selected={chosen() === h.path} class="qn-row" onClick={() => setChosen(h.path)} onDblClick={() => props.onUse(h.path)}>
                  <Folder size={14} />
                  <span class="qn-path" title={h.path}>{props.short ? props.short(h.path) : h.path}</span>
                  <Show when={i() === 0}><span class="qn-best">best match</span></Show>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </BoardSheet>
  );
}
