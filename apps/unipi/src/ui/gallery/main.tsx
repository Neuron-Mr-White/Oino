/**
 * UI renderer gallery (ui-protocol §13.7): every node fixture of the shared
 * corpus through `EntryView`, grouped by kind, plus composite panels in
 * definite boxes and the perf page. Served by `vite.config.ui-capture.ts`.
 *
 * URL: `?shell=desktop|phone` (default desktop), `?kind=<Kind>` to show one
 * kind, `?page=demos|perf`.
 */
import { render } from 'solid-js/web';
import { For, Show, createSignal } from 'solid-js';
import '../../style.css';
import '../../ui.css';
import '../../components/shell/shell.css';
import { EntryView } from '../NodeView';
import { setUiTransport, type UiActionPayload } from '../actions';
import type { UiEntry, UiNode } from '../types';
import { DASHBOARD, FORM, USAGE } from './demos';
import { PerfPage } from './perf';
import './gallery.css';

const params = new URLSearchParams(location.search);
const shell = params.get('shell') === 'phone' ? 'phone' : 'desktop';
if (shell === 'desktop') document.documentElement.dataset.shell = 'desktop';
const kindFilter = params.get('kind');
const page = params.get('page');

const files = import.meta.glob<UiNode>('../../../../../crates/oino-ui/fixtures/nodes/*.json', { eager: true, import: 'default' });
const fixtures = Object.entries(files)
  .map(([path, node]) => ({ name: path.split('/').pop()!.replace(/\.json$/, ''), node }))
  .sort((a, b) => a.name.localeCompare(b.name));
const kinds = [...new Set(fixtures.map((f) => f.node.t))];

const [last, setLast] = createSignal<UiActionPayload | undefined>();
const log: UiActionPayload[] = ((window as unknown as { __UI_ACTIONS__: UiActionPayload[] }).__UI_ACTIONS__ = []);
setUiTransport({
  action: async (_t, p) => {
    log.push(p);
    setLast(p);
    return {};
  },
  visible: async () => ({}),
  resync: async () => ({}),
});

const entry = (id: string, root: UiNode): UiEntry => ({ id: `gallery/${id}`, extensionId: 'gallery', scope: 'session', slot: 'sidePane', order: 0, seq: 1, view: { kind: 'tree', root }, rev: 1 });
const target = { scope: 'session' as const, hostId: 'gallery', sessionId: 'gallery' };

function Card(props: { name: string; node: UiNode; definite?: number }) {
  return (
    <article class="g-card" data-definite={props.definite ? '' : undefined}>
      <header class="g-card-head">
        <span class="g-name">{props.name}</span>
        <Show when={props.definite}>
          <span class="g-tag">definite · {props.definite}px</span>
        </Show>
      </header>
      <div class="g-body" style={props.definite ? { height: `${props.definite}px` } : undefined}>
        <EntryView entry={entry(props.name, props.node)} target={target} mode={props.definite ? 'definite' : 'auto'} />
      </div>
    </article>
  );
}

function Fixtures() {
  const shown = () => (kindFilter ? kinds.filter((k) => k === kindFilter) : kinds);
  return (
    <For each={shown()}>
      {(kind) => (
        <section class="g-section">
          <h2 class="g-kind">{kind}</h2>
          <div class="g-grid">
            <For each={fixtures.filter((f) => f.node.t === kind)}>{(f) => <Card name={f.name} node={f.node} />}</For>
          </div>
        </section>
      )}
    </For>
  );
}

function Demos() {
  return (
    <section class="g-section">
      <h2 class="g-kind">Composite panels</h2>
      <div class="g-grid g-grid-wide">
        <Card name="dashboard" node={DASHBOARD} definite={shell === 'phone' ? 560 : 360} />
        <Card name="usage" node={USAGE} definite={360} />
        <Card name="form" node={FORM} />
      </div>
    </section>
  );
}

function Gallery() {
  return (
    <div class="g-page">
      <header class="g-top">
        <div>
          <h1 class="g-title">UI renderer gallery</h1>
          <p class="g-meta">
            {fixtures.length} fixtures · {kinds.length} kinds · {shell} shell
          </p>
        </div>
        <Show when={last()}>
          {(p) => (
            <code class="g-last" aria-live="polite">
              {p().actionId}
              {p().value !== undefined ? ` → ${JSON.stringify(p().value)}` : ''}
            </code>
          )}
        </Show>
      </header>
      <Show when={page === 'perf'} fallback={<Show when={page === 'demos'} fallback={<><Show when={!kindFilter}><Demos /></Show><Fixtures /></>}><Demos /></Show>}>
        <PerfPage />
      </Show>
    </div>
  );
}

render(() => <Gallery />, document.getElementById('root')!);
