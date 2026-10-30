/**
 * `NodeView` (ui-protocol §13.1–13.3): one Solid component per node kind,
 * picked by `t`; unknown kinds render `fallback` (or nothing, D-U14); a leaf's
 * `block` prop wraps it in the Block chrome. `EntryView` renders one registry
 * entry inside an ErrorBoundary so one bad tree never breaks the shell.
 */
import { ErrorBoundary, Match, Show, Switch, createMemo, type Component, type JSX } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { EmptyState } from '../components/EmptyState';
import { EntryContext, createEntryActions, type ActionHook, type UiTarget } from './actions';
import { BlockFrame } from './nodes/Block';
import { NODE_COMPONENTS } from './nodes';
import { LineView } from './nodes/Text';
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

/** One registry entry: its view by kind, inside an ErrorBoundary + the entry's action context. */
export function EntryView(props: EntryViewProps) {
  const actions = createMemo(() => createEntryActions(props.entry.id, props.target, props.onAction));
  const mode = () => props.mode ?? 'auto';
  const path: NodePath = [];
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <EntryFailed
          onRetry={reset}
          onReport={() => {
            console.error('[ui] entry failed to render', props.entry.id, error);
            props.onReport?.(props.entry.id, error);
          }}
        />
      )}
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
              {/* The native component is routed by U12 (ReservedHost); elsewhere its fallback tree shows. */}
              {(v) => <NodeView node={(v() as { fallback?: UiNode }).fallback} path={['fallback']} mode={mode()} />}
            </Match>
          </Switch>
        </div>
      </EntryContext.Provider>
    </ErrorBoundary>
  );
}

function EntryFailed(props: { onRetry: () => void; onReport: () => void }): JSX.Element {
  return (
    <EmptyState
      compact
      tone="error"
      title="This panel failed to render"
      testId="ui-entry-failed"
      action={
        <>
          <button type="button" class="btn btn-secondary btn-sm" onClick={() => props.onRetry()}>
            Retry
          </button>
          <button type="button" class="btn btn-ghost btn-sm" onClick={() => props.onReport()}>
            Report
          </button>
        </>
      }
    />
  );
}
