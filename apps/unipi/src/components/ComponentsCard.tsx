/**
 * Settings → About → Components (P0-01): what this install actually ships —
 * the UniPi app, the bundled agent host, the oino engine and the bundled
 * extensions, from `about_components` (lib/about.ts). Read-only: the installer
 * bundles everything, so there is nothing to install or update here (the
 * app's own updater is the Update card above). A genuinely broken install
 * shows its problem with the fix.
 */
import { For, Show, onMount, type JSX } from 'solid-js';
import { Cpu, Download, Monitor, Puzzle, Server, TriangleAlert } from 'lucide-solid';
import { ABOUT_ANCHORS } from '../screens/settings/anchors';
import { componentsState, refreshComponents, type Components } from '../lib/about';
import { openExternal } from '../lib/board/share';

/** Where a broken install gets a fresh copy. */
export const RELEASES_URL = 'https://github.com/Neuron-Mr-White/Oino/releases/latest';

export function ComponentsCard() {
  onMount(() => void refreshComponents());
  const info = (): Components | undefined => {
    const s = componentsState();
    return s.kind === 'ready' ? s.info : undefined;
  };
  const pending = () => {
    const k = componentsState().kind;
    return k === 'idle' || k === 'loading';
  };
  /** A version cell: the value, "…" while reading, "Not found" when absent. */
  const version = (value: string | undefined): string => (value ? value : pending() ? '…' : 'Not found');

  return (
    // A fragment, not a wrapper: `.settings-row + .settings-row` draws the
    // dividers, so these rows stay siblings of the About section's other rows.
    <>
      <For each={info()?.problems ?? []}>
        {(problem) => (
          <div class="settings-row" role="alert" data-testid={`components-problem-${problem.component}`}>
            <span class="settings-icon text-wm-red">
              <TriangleAlert size={18} />
            </span>
            <div class="min-w-0">
              <p class="text-[15px] text-wm-red">{problem.message}</p>
              <p class="text-xs text-text-dim mt-0.5">{problem.fix}</p>
            </div>
            <div class="shrink-0">
              <button type="button" class="btn btn-secondary btn-sm" data-testid="components-download" onClick={() => void openExternal(RELEASES_URL)}>
                <Download size={14} /> Download
              </button>
            </div>
          </div>
        )}
      </For>
      <Item id={ABOUT_ANCHORS.components} icon={<Monitor size={18} />} label="UniPi app" testId="about-version" value={version(info()?.app)} />
      <Item icon={<Server size={18} />} label="Agent host" description={info()?.host.path} testId="components-host" value={version(info()?.host.version)} />
      <Item icon={<Cpu size={18} />} label="oino engine" description={info()?.oino.path} testId="components-oino" value={version(info()?.oino.version)} />
      <Item
        icon={<Puzzle size={18} />}
        label="Extensions"
        description={info()?.extensions.dir}
        testId="components-extensions"
        value={pending() ? '…' : `${info()?.extensions.count ?? 0} bundled`}
      />
      <Show when={componentsState().kind === 'error'}>
        <p class="px-4 pb-3 text-xs text-text-dim" data-testid="components-error">
          Could not read this install: {(componentsState() as { message: string }).message}
        </p>
      </Show>
    </>
  );
}

/** One component row: label + where it lives, the version on the right. */
function Item(props: { id?: string; icon: JSX.Element; label: string; description?: string; value: string; testId: string }) {
  return (
    <div class="settings-row settings-anchored" id={props.id}>
      <span class="settings-icon">{props.icon}</span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">{props.label}</p>
        <Show when={props.description}>
          <p class="text-xs text-text-dim mt-0.5 truncate" title={props.description}>
            {props.description}
          </p>
        </Show>
      </div>
      <div class="shrink-0 text-right">
        <span class="text-sm text-text-muted tabular-nums" data-testid={props.testId}>
          {props.value}
        </span>
      </div>
    </div>
  );
}
