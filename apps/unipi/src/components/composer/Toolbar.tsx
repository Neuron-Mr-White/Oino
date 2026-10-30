/**
 * The composer toolbar (WP-A13, Z-64): one row under the field with the
 * attach `+` on the left, the controls in the middle and Send/Stop on the
 * right. The middle holds the permission chip + Plan toggle (this WP) and,
 * from WP-A14, the model / thinking / context chips through the same slot
 * wrappers.
 *
 * Narrow widths: the bar is the single layout owner — it measures the real
 * widths of its items and applies ZCode's seven cumulative compression steps
 * (see `lib/composer/compression.ts`), smallest step that fits, restoring in
 * reverse when it widens. The current step is published as `data-step` and as
 * per-control flags (`data-c-permission`, `data-c-plan`, `data-c-think`,
 * `data-c-model`), which the stylesheet uses to fold a chip into its icon
 * form; `useToolbarStep()` is there for chips that need more than CSS.
 */
import { createContext, createEffect, createSignal, onCleanup, onMount, Show, untrack, useContext, type Accessor, type JSX } from 'solid-js';
import { compressionStep, type ToolbarItem as ToolbarItemSize, type ToolbarSlot } from '../../lib/composer/compression';

/** Gap between toolbar items (ZCode keeps 12 px between groups; ours is 8). */
export const TOOLBAR_GAP = 8;

const StepContext = createContext<Accessor<number>>(() => 0);

/** The bar's current compression step (0 = full), inside a `Toolbar`. */
export function useToolbarStep(): Accessor<number> {
  return useContext(StepContext);
}

/** Wraps one compressible control so the bar can measure and fold it. */
export function ToolbarSlot(props: { slot: ToolbarSlot; children: JSX.Element }) {
  return (
    <div class="tb-slot" data-tb-item data-slot={props.slot}>
      {props.children}
    </div>
  );
}

/** A non-compressible toolbar item (`+`, context ring, send). */
export function ToolbarItem(props: { children: JSX.Element }) {
  return (
    <div class="tb-item" data-tb-item>
      {props.children}
    </div>
  );
}

export function Toolbar(props: {
  /** The attach `+` (left). */
  lead?: JSX.Element;
  /** Send / Stop (right). */
  end?: JSX.Element;
  /** Controls between them — plain `ToolbarItem`s and `ToolbarSlot`s. */
  children?: JSX.Element;
  /** Phone shell: the bar wraps nothing, it only folds harder. */
  phone?: boolean;
}) {
  let bar: HTMLDivElement | undefined;
  const [step, setStep] = createSignal(0);
  let measuring = false;
  let signature = '';

  const measure = () => {
    const el = bar;
    if (!el || measuring) return;
    measuring = true;
    const applied = step();
    try {
      // Measure the full layout: undo our own folds first (synchronous, so
      // nothing is painted in between).
      if (applied !== 0) setStep(0);
      const items: ToolbarItemSize[] = [...el.querySelectorAll<HTMLElement>('[data-tb-item]')].map((item) => {
        const slot = item.dataset.slot as ToolbarSlot | undefined;
        return { ...(slot ? { slot } : {}), width: item.getBoundingClientRect().width };
      });
      const available = el.clientWidth;
      const next = compressionStep(available, items, TOOLBAR_GAP);
      const nextSignature = `${available}|${items.map((i) => `${i.slot ?? ''}:${Math.round(i.width)}`).join(',')}`;
      if (nextSignature !== signature || next !== applied) {
        signature = nextSignature;
        setStep(next);
      } else if (applied !== 0) {
        setStep(applied);
      }
    } finally {
      measuring = false;
    }
  };

  onMount(() => {
    measure();
    const ro = new ResizeObserver(() => measure());
    if (bar) ro.observe(bar);
    for (const item of bar?.querySelectorAll<HTMLElement>('[data-tb-item]') ?? []) ro.observe(item);
    // A web font landing changes every label's width.
    void document.fonts?.ready.then(() => measure()).catch(() => undefined);
    window.addEventListener('resize', measure);
    onCleanup(() => {
      ro.disconnect();
      window.removeEventListener('resize', measure);
    });
  });

  // A chip that comes or goes (A14's model chip, the permission control on a
  // session without the workflow extension) re-runs the decision — untracked,
  // so the step this very measurement sets is not a dependency of it.
  createEffect(() => {
    void props.children;
    untrack(measure);
  });

  const folded = (threshold: number) => (step() >= threshold ? '' : undefined);

  return (
    <StepContext.Provider value={step}>
      <div
        class="comp-bar"
        data-testid="composer-toolbar"
        data-step={step()}
        data-phone={props.phone ? '' : undefined}
        data-c-computer={folded(1)}
        data-c-permission={folded(2)}
        data-c-plan={folded(3)}
        data-c-think={step() >= 6 ? 'square' : step() >= 4 ? 'lite' : undefined}
        data-c-model={folded(7)}
        ref={bar}
      >
        <Show when={props.lead}>
          <ToolbarItem>{props.lead}</ToolbarItem>
        </Show>
        {props.children}
        <Show when={props.end}>
          <div class="tb-spacer" />
          <ToolbarItem>{props.end}</ToolbarItem>
        </Show>
      </div>
    </StepContext.Provider>
  );
}
