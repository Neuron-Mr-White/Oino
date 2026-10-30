/**
 * `NodeView` (ui-protocol §13.1–13.3): one Solid component per node kind,
 * picked by `t`; unknown kinds render `fallback` (or nothing, D-U14); a leaf's
 * `block` prop wraps it in the Block chrome. `EntryView` renders one registry
 * entry inside a `SlotBoundary` so one bad tree never breaks the shell.
 */
import { Match, Show, Switch, createMemo, type Component } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { SlotBoundary } from '../components/ErrorBoundaries';
import { EntryContext, createEntryActions, type ActionHook, type UiTarget } from './actions';
import { BlockFrame } from './nodes/Block';
import { NODE_COMPONENTS } from './nodes';
import { LineView } from './nodes/Text';
import { reservedBodyFor, type ReservedBodyProps } from './slots/reserved';
import type { Mode, NodeProps } from './nodes/props';
import type { BlockProps, Common, NodePath, UiEntry, UiNode } from './types';
import './ui.css';

/** Kinds ratatui doesn't allow `.block()` on (§4.1). */
const BLOCKLESS = new Set(['Layout', 'Block', 'Spinner', 'KeyHints', 'Scrollbar']);

export function NodeView(props: NodeProps<UiNode | (Common & { t: string }) | undefined | null>) {
  const kind = createMemo(() => props.node?.t);
  return (
    <Show when={kind()} keyed>
      {(k) => {
        const C = (NODE_COMPONENTS as Record<string, Component<NodeProps<never>>>)[k];
        if (!C)
          return (
            <Show when={props.node?.fallback}>
              {(fb) => <NodeView node={fb()} path={[...props.path, 'fallback']} mode={props.mode} />}
            </Show>
          );
        const body = () => <Dynamic component={C} node={props.node as never} path={props.path} mode={props.mode} />;
        const block = () => (BLOCKLESS.has(k) ? undefined : (props.node as Common | undefined)?.block);
        return (
          <Show when={block()} fallback={body()}>
            {(b) => (
              <BlockFrame block={b() as BlockProps} path={props.path} mode={props.mode}>
                {body()}
              </BlockFrame>
            )}
          </Show>
        );
      }}
    </Show>
  );
}

export type EntryViewProps = {
  entry: UiEntry;
  target: UiTarget;
  mode?: Mode;
  /** Observes every action fired from the entry (gallery, tests). */
  onAction?: ActionHook;
  /** "Report" on a render failure (U10 turns it into an extension notice). */
  onReport?: (entryId: string, error: unknown) => void;
};

/**
 * The body of a `reserved` entry (ui-protocol §8, WP-U12): the native component
 * when the app renders this row in place (A27's turn footer registers here),
 * else the owner's `fallback` tree — the same degradation the TUI gets (§10).
 * A row whose native surface is a page, a pane tab or the Session modal is
 * focused by `ReservedHost` instead; wherever it still lands in a slot, its
 * fallback tree is what shows (rule 3).
 */
function ReservedEntryBody(props: { entry: UiEntry; target: UiTarget; mode: Mode }) {
  const body = createMemo(() => reservedBodyFor(props.entry));
  return (
    <Show
      when={body()}
      fallback={<NodeView node={props.entry.view.kind === 'reserved' ? props.entry.view.fallback : undefined} path={['fallback']} mode={props.mode} />}
    >
      {(component) => {
        const C = component() as unknown as Component<ReservedBodyProps>;
        return <C entry={props.entry} target={props.target} mode={props.mode} />;
      }}
    </Show>
  );
}

/** One registry entry: its view by kind, inside an ErrorBoundary + the entry's action context. */
export function EntryView(props: EntryViewProps) {
  const actions = createMemo(() => createEntryActions(props.entry.id, props.target, props.onAction));
  const mode = () => props.mode ?? 'auto';
  const path: NodePath = [];
  return (
    <SlotBoundary
      entryId={props.entry.id}
      onReport={(id, error) => {
        console.error('[ui] entry failed to render', id, error);
        props.onReport?.(id, error);
      }}
    >
      <EntryContext.Provider value={actions()}>
        <div class="ui-entry" data-entry={props.entry.id} data-mode={mode()}>
          <Switch>
            <Match when={props.entry.view.kind === 'tree' && props.entry.view}>
              {(v) => <NodeView node={(v() as { root: UiNode }).root} path={path} mode={mode()} />}
            </Match>
            <Match when={props.entry.view.kind === 'lines' && props.entry.view}>
              {(v) => (
                <div class="ui-p ui-lines">
                  {((v() as { lines: string[] }).lines ?? []).map((l) => (
                    <LineView line={l} />
                  ))}
                </div>
              )}
            </Match>
            <Match when={props.entry.view.kind === 'status' && props.entry.view}>
              {(v) => <LineView line={(v() as { text: string }).text} class="ui-status-line" />}
            </Match>
            <Match when={props.entry.view.kind === 'reserved' && props.entry.view}>
              <ReservedEntryBody entry={props.entry} target={props.target} mode={mode()} />
            </Match>
          </Switch>
        </div>
      </EntryContext.Provider>
    </SlotBoundary>
  );
}
