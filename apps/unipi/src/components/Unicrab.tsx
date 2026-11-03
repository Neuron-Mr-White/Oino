/**
 * `<Unicrab state size>` — the animated mascot (UNI-200). One SVG of crisp
 * rects (`shape-rendering: crispEdges`) built from unicrab.sprite.json,
 * parts in `<g data-track>` groups that CSS keyframes (lib/unicrab.ts
 * `keyframesCss`, `steps()` + whole-cell translations) move per state.
 * Blinks are a JS timer flipping `data-blink`. prefers-reduced-motion
 * holds frame 0 and never blinks. See DESIGN.md §1.5.1 for how the rig works.
 *
 * perf-app (idle CPU): the loop only runs while it means something. A crab
 * that is off screen (IntersectionObserver), in a hidden app, or `still`
 * (the draft watermark) holds frame 0 (`data-paused`) and does not blink;
 * a resting state (idle / sleep / offline / done) plays for `REST_PLAY_MS`
 * after it appears, changes or is hovered, then holds frame 0 and only
 * blinks. Working / needs-you keep looping while visible.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import './mascot.css';
import { anim, nextBlinkDelay, sprite, viewBox, zzzCells, keyframesCss, type Cell, type EyeMask, type MascotState, EYE_MASKS, STATE_LABEL } from '../lib/unicrab';

let styleInjected = false;
function injectStyle(): void {
  if (styleInjected || typeof document === 'undefined') return;
  styleInjected = true;
  const el = document.createElement('style');
  el.dataset.unicrab = '';
  el.textContent = keyframesCss();
  document.head.append(el);
}

/** Horizontal runs of one colour → one rect each (fewer nodes, no hairline seams inside a run). */
export function runs(cells: readonly Cell[]): { x: number; y: number; w: number; c: number }[] {
  const sorted = [...cells].sort((a, b) => a[1] - b[1] || a[0] - b[0]);
  const out: { x: number; y: number; w: number; c: number }[] = [];
  for (const [x, y, c] of sorted) {
    const last = out[out.length - 1];
    if (last && last.y === y && last.c === c && last.x + last.w === x) last.w += 1;
    else out.push({ x, y, w: 1, c });
  }
  return out;
}

function Cells(props: { cells: readonly Cell[] }) {
  return (
    <For each={runs(props.cells)}>
      {(r) => <rect x={r.x} y={r.y} width={r.w} height={1} fill={sprite.palette[r.c]} />}
    </For>
  );
}

/** States that settle to a still pose (with blinks) after a short play. */
const REST_STATES: ReadonlySet<MascotState> = new Set(['idle', 'sleep', 'offline', 'done']);
/** How long a resting state loops after it appears / changes / is hovered. */
export const REST_PLAY_MS = 6_000;

/** `document.hidden`, as a signal shared by every crab. */
const [docHidden, setDocHidden] = createSignal(typeof document !== 'undefined' && document.hidden === true);
let docHook = false;
function hookDocHidden(): void {
  if (docHook || typeof document === 'undefined') return;
  docHook = true;
  document.addEventListener('visibilitychange', () => setDocHidden(document.hidden === true));
}

const reducedMotion = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

export function Unicrab(props: {
  state: MascotState;
  size?: number;
  class?: string;
  title?: string;
  style?: JSX.CSSProperties;
  /** Never animates (a watermark / decoration): frame 0, no blinks. */
  still?: boolean;
}) {
  injectStyle();
  hookDocHidden();
  const [blink, setBlink] = createSignal(false);
  let svg: SVGSVGElement | undefined;
  // Without IntersectionObserver (jsdom, old webviews) a crab counts as on screen.
  const [onScreen, setOnScreen] = createSignal(true);
  onMount(() => {
    if (!svg || typeof IntersectionObserver === 'undefined') return;
    const io = new IntersectionObserver((entries) => {
      const e = entries[entries.length - 1];
      if (e) setOnScreen(e.isIntersecting);
    });
    io.observe(svg);
    onCleanup(() => io.disconnect());
  });
  /** A resting state is "awake" (looping) for REST_PLAY_MS after it appears, changes or is hovered. */
  const [awake, setAwake] = createSignal(true);
  let restTimer: ReturnType<typeof setTimeout> | undefined;
  const wake = () => {
    setAwake(true);
    clearTimeout(restTimer);
    restTimer = setTimeout(() => setAwake(false), REST_PLAY_MS);
  };
  createEffect(() => {
    props.state;
    wake();
  });
  onCleanup(() => clearTimeout(restTimer));
  /** May anything move at all (visible, shown, not a decoration)? */
  const visible = () => !props.still && onScreen() && !docHidden();
  const looping = () => visible() && (!REST_STATES.has(props.state) || awake());
  createEffect(() => {
    const s = anim.states[props.state];
    if (!s?.blink || reducedMotion() || !visible()) return;
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      timer = setTimeout(() => {
        setBlink(true);
        timer = setTimeout(() => {
          setBlink(false);
          schedule();
        }, anim.blinkMs);
      }, nextBlinkDelay());
    };
    schedule();
    onCleanup(() => {
      clearTimeout(timer);
      setBlink(false);
    });
  });
  const size = () => props.size ?? 40;
  const [, , vw, vh] = viewBox().split(' ').map(Number) as [number, number, number, number];
  return (
    <svg
      class={`unicrab ${props.class ?? ''}`}
      data-state={props.state}
      data-blink={blink() ? '' : undefined}
      data-paused={looping() ? undefined : ''}
      ref={svg}
      onPointerEnter={() => REST_STATES.has(props.state) && wake()}
      viewBox={viewBox()}
      width={size()}
      height={Math.round((size() * vh) / vw)}
      shape-rendering="crispEdges"
      role="img"
      aria-label={props.title ?? `Unicrab: ${STATE_LABEL[props.state]}`}
      style={props.style}
    >
      <g data-track="all">
        <g data-track="legs-left"><Cells cells={sprite.parts['legs-left']} /></g>
        <g data-track="legs-right"><Cells cells={sprite.parts['legs-right']} /></g>
        <g data-track="torso">
          <Cells cells={sprite.parts.body} />
          <Cells cells={sprite.parts.visor} />
          <For each={EYE_MASKS as readonly EyeMask[]}>
            {(mask) => <g data-eyes={mask}><Cells cells={sprite.eyes[mask]} /></g>}
          </For>
        </g>
        <g data-track="claw-left"><Cells cells={sprite.parts['claw-left']} /></g>
        <g data-track="claw-right"><Cells cells={sprite.parts['claw-right']} /></g>
      </g>
      <Show when={props.state === 'sleep'}>
        <g data-track="zzz"><Cells cells={zzzCells()} /></g>
      </Show>
    </svg>
  );
}
