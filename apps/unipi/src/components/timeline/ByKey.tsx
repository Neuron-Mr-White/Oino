/**
 * Keyed list that keeps a row mounted while its **key** survives (ui-stream).
 *
 * The live tail turn is rebuilt on every stream flush (`turns.ts` builds
 * fresh segment / work-item objects), so a plain `<For>` — keyed by object
 * identity — disposed and re-created the whole live segment (fold, tool rows,
 * the streaming prose) ~60×/s: DOM churn, lost text selection, re-hydrated
 * code blocks, and no place for a smoother to keep its state. `ByKey` maps
 * over the stable string keys instead and hands each row an accessor to the
 * latest object with that key, so updates flow through props reactively.
 */
import { createMemo, For, type Accessor, type JSX } from 'solid-js';

export function ByKey<T extends object>(props: {
  each: readonly T[] | undefined;
  key: (item: T) => string;
  children: (item: Accessor<T>) => JSX.Element;
}): JSX.Element {
  const byKey = createMemo(() => {
    const m = new Map<string, T>();
    for (const it of props.each ?? []) m.set(props.key(it), it);
    return m;
  });
  const keys = createMemo(() => (props.each ?? []).map(props.key));
  return (
    <For each={keys()}>
      {(k) => {
        let last = byKey().get(k)!;
        const item = createMemo<T>(() => {
          const cur = byKey().get(k);
          if (cur !== undefined) last = cur;
          return last;
        });
        return props.children(item);
      }}
    </For>
  );
}
