/**
 * Shortcuts panel (DESIGN.md §3.3): herdr and tmux tabs with the
 * prefix and the real action keys. herdr defaults come from the
 * host's `herdr --default-config`; tmux keys are the stock list with
 * the live prefix.
 */
import { createEffect, createSignal, For, Show } from 'solid-js';
import Drawer from '@corvu/drawer';
import { call } from '../lib/ipc';
import {
  ShortcutsDataSchema,
  type ShortcutsData,
} from '@unipi/protocol';

export type ShortcutsSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  hostId: string;
  relay?: string;
};

type Mux = 'herdr' | 'tmux';

export default function ShortcutsSheet(props: ShortcutsSheetProps) {
  const [mux, setMux] = createSignal<Mux>('herdr');
  const [data, setData] = createSignal<ShortcutsData | undefined>(undefined);
  const [error, setError] = createSignal<string | undefined>(undefined);

  // Load whenever the sheet opens (controlled open prop included).
  createEffect(() => {
    if (props.open) void load();
  });

  async function load() {
    if (!props.open) return;
    const result = await call(
      'shortcuts',
      { hostId: props.hostId, relay: props.relay },
      ShortcutsDataSchema,
    );
    if (result.ok) {
      setData(result.value);
      setError(undefined);
    } else {
      setError(result.error.message);
    }
  }

  const current = () => (mux() === 'herdr' ? data()?.herdr : data()?.tmux);

  return (
    <Drawer
      open={props.open}
      onOpenChange={(open: boolean) => {
        if (open) void load();
        props.onOpenChange(open);
      }}
      side="bottom"
      // Snap points are fractions of the sheet's own (75dvh) height:
      // open fully, close below. A lone 0.75 made every drag settle a
      // quarter lower than where the sheet opened.
      snapPoints={[0, 1]}
      allowSkippingSnapPoints={false}
    >
      <Drawer.Portal>
        <Drawer.Overlay class="sheet-overlay" />
        <Drawer.Content class="picker-sheet flex h-[75dvh] flex-col" aria-label="Shortcuts">
          <div class="mx-auto mb-3 mt-3 h-1 w-9 rounded-pill bg-line" />
          <div class="flex items-center justify-between px-5">
            <div class="flex h-12 items-center gap-1 rounded-pill bg-surface-2 p-1">
              <button
                type="button"
                class={`flex h-10 items-center gap-2 rounded-pill px-4 text-[15px] font-medium ${
                  mux() === 'herdr' ? 'bg-surface-1 text-text ring-1 ring-inset ring-line' : 'text-text-muted'
                }`}
                onClick={() => setMux('herdr')}
              >
                Herdr
              </button>
              <button
                type="button"
                class={`flex h-10 items-center gap-2 rounded-pill px-4 text-[15px] font-medium ${
                  mux() === 'tmux' ? 'bg-surface-1 text-text ring-1 ring-inset ring-line' : 'text-text-muted'
                }`}
                onClick={() => setMux('tmux')}
              >
                Tmux
              </button>
            </div>
          </div>
          <div class="mt-3 flex-1 overflow-y-auto px-5 pb-6">
            <Show
              when={!error()}
              fallback={<p class="px-1 py-6 text-sm text-wm-red">{error()}</p>}
            >
              <Show when={current()} fallback={<div class="skeleton mx-1 my-3 h-12 rounded-input" />}>
                {(entry) => (
                  <>
                    <p class="mb-3 px-1 text-[13px] text-text-muted">
                      Prefix: <span class="font-mono text-text">{entry().prefix}</span> — press the
                      prefix, then the key.
                    </p>
                    <div class="rounded-[20px] bg-surface-2">
                      <For each={entry().keys}>
                        {(shortcut) => (
                          <div class="flex items-center justify-between px-5 py-2.5 [&:not(:last-child)]:border-b [&:not(:last-child)]:border-line/40">
                            <span class="text-[15px] text-text">{shortcut.name}</span>
                            <span class="rounded-pill bg-surface-1 px-2.5 py-0.5 font-mono text-[13px] text-text-muted">
                              {shortcut.key}
                            </span>
                          </div>
                        )}
                      </For>
                    </div>
                  </>
                )}
              </Show>
            </Show>
          </div>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer>
  );
}
