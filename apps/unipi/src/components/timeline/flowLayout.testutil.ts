/**
 * [chat-load] A tiny FLOW layout for the timeline in jsdom (which has no layout): enough
 * geometry to drive the real `Timeline` like a browser does and to assert what the reader
 * would see — row rects, the anchor turn's on-screen position across chunk loads, and how
 * many loads / row mounts a scroll through the whole history takes.
 *
 *   .tl            the viewport (`viewport` px tall, scrollTop clamped like the DOM)
 *   .tl-content    14 px padding top + its children stacked + 20 px bottom
 *   .tl-earlier    52 px · .tl-skeleton 120 px · .tl-empty 200 px
 *   .tl-list       padding-top + rows stacked + padding-bottom (from its inline style)
 *   .tl-vrow       `rowHeight(turnId)` (the test owns it and may change it: a row "growing")
 *   .tl-tail-wrap  `tailHeight()`
 *   anything else  inside a row: 10 px below the row's top, 20 px tall (headers, toggles)
 *
 * Browser timing it reproduces: a programmatic `scrollTop` write fires `scroll` LATER (one
 * frame), ResizeObserver reports after layout (once per frame, only changed sizes, and a
 * first observation always), `requestAnimationFrame` runs per frame. `frame()` runs one
 * frame; `frames()` runs until nothing is pending.
 */

type Cb = (entries: Array<{ target: Element; borderBoxSize: Array<{ blockSize: number; inlineSize: number }>; contentRect: { height: number; width: number } }>) => void;

export type FlowLayout = {
  viewport: number;
  rowHeight: (turnId: string) => number;
  tailHeight: () => number;
  /** One browser frame: scroll events, rAF callbacks, ResizeObserver deliveries (+ microtasks between). */
  frame(): Promise<void>;
  /** Frames until nothing is pending (bounded). */
  frames(max?: number): Promise<void>;
  /** The reader scrolls the timeline to `top` with a wheel (an input event, then the scroll event in the next frame). */
  wheelTo(tl: HTMLElement, top: number): Promise<void>;
  /** On-screen rects of the mounted rows, top to bottom. */
  rows(tl: HTMLElement): Array<{ id: string; top: number; bottom: number }>;
  restore(): void;
};

const num = (v: string | undefined) => (v ? Number.parseFloat(v) || 0 : 0);

export function installFlowLayout(opts: { viewport?: number; rowHeight?: (turnId: string) => number; tailHeight?: () => number } = {}): FlowLayout {
  const layout: FlowLayout = {
    viewport: opts.viewport ?? 600,
    rowHeight: opts.rowHeight ?? (() => 100),
    tailHeight: opts.tailHeight ?? (() => 0),
  } as FlowLayout;
  const proto = HTMLElement.prototype;
  const saved = new Map<string, PropertyDescriptor | undefined>();
  const keep = (name: string) => saved.set(name, Object.getOwnPropertyDescriptor(proto, name));
  ['offsetHeight', 'offsetWidth', 'offsetTop', 'clientHeight', 'scrollHeight', 'scrollTop', 'getBoundingClientRect', 'scrollTo'].forEach(keep);
  const savedRO = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
  const savedRAF = globalThis.requestAnimationFrame;
  const savedCAF = globalThis.cancelAnimationFrame;

  const has = (el: Element, c: string) => el.classList?.contains(c);
  function height(el: HTMLElement): number {
    if (has(el, 'tl')) return layout.viewport;
    if (has(el, 'tl-content')) {
      let h = 34;
      for (const c of Array.from(el.children)) h += height(c as HTMLElement);
      return h;
    }
    if (has(el, 'tl-list')) {
      let h = num(el.style.paddingTop) + num(el.style.paddingBottom);
      for (const c of Array.from(el.children)) h += height(c as HTMLElement);
      return h;
    }
    if (has(el, 'tl-vrow')) return layout.rowHeight(el.dataset.turn ?? '');
    if (has(el, 'tl-tail-wrap')) return layout.tailHeight();
    if (has(el, 'tl-earlier')) return 52;
    if (has(el, 'tl-skeleton')) return 120;
    if (has(el, 'tl-empty')) return 200;
    if (el.parentElement && has(el.parentElement, 'tl-content')) return 0;
    return el.closest('.tl-vrow') ? 20 : 0;
  }
  /** Top in .tl-content coordinates. */
  function top(el: HTMLElement): number {
    const parent = el.parentElement;
    if (!parent || has(el, 'tl-content') || has(el, 'tl')) return 0;
    if (has(parent, 'tl-content')) {
      let y = 14;
      for (const c of Array.from(parent.children)) {
        if (c === el) return y;
        y += height(c as HTMLElement);
      }
      return y;
    }
    if (has(parent, 'tl-list')) {
      let y = top(parent) + num(parent.style.paddingTop);
      for (const c of Array.from(parent.children)) {
        if (c === el) return y;
        y += height(c as HTMLElement);
      }
      return y;
    }
    const row = el.closest('.tl-vrow') as HTMLElement | null;
    if (row && row !== el) return top(row) + 10;
    return top(parent);
  }
  const scroller = (el: HTMLElement) => el.closest('.tl') as HTMLElement | null;
  const tops = new WeakMap<HTMLElement, number>();
  const maxTop = (tl: HTMLElement) => Math.max(0, (tl.querySelector('.tl-content') ? height(tl.querySelector('.tl-content') as HTMLElement) : 0) - layout.viewport);

  // ── frame queues ─────────────────────────────────────────────
  const pendingScroll = new Set<HTMLElement>();
  let rafs: Array<{ id: number; cb: FrameRequestCallback }> = [];
  let rafId = 0;
  const observers = new Set<{ cb: Cb; targets: Map<Element, number> }>();

  Object.defineProperty(proto, 'offsetHeight', { configurable: true, get(this: HTMLElement) { return height(this); } });
  Object.defineProperty(proto, 'offsetWidth', { configurable: true, get: () => 720 });
  Object.defineProperty(proto, 'offsetTop', { configurable: true, get(this: HTMLElement) { return top(this); } });
  Object.defineProperty(proto, 'clientHeight', { configurable: true, get(this: HTMLElement) { return has(this, 'tl') ? layout.viewport : 0; } });
  Object.defineProperty(proto, 'scrollHeight', {
    configurable: true,
    get(this: HTMLElement) {
      if (!has(this, 'tl')) return 0;
      return Math.max(layout.viewport, maxTop(this) + layout.viewport);
    },
  });
  Object.defineProperty(proto, 'scrollTop', {
    configurable: true,
    get(this: HTMLElement) {
      if (!has(this, 'tl')) return tops.get(this) ?? 0;
      const v = tops.get(this) ?? 0;
      return Math.max(0, Math.min(v, maxTop(this)));
    },
    set(this: HTMLElement, v: number) {
      if (!has(this, 'tl')) {
        tops.set(this, v);
        return;
      }
      const before = this.scrollTop;
      tops.set(this, Math.max(0, Math.min(v, maxTop(this))));
      if (this.scrollTop !== before) pendingScroll.add(this);
    },
  });
  proto.scrollTo = function scrollTo(this: HTMLElement, arg?: number | ScrollToOptions) {
    if (typeof arg === 'object' && arg && typeof arg.top === 'number') this.scrollTop = arg.top;
  } as typeof proto.scrollTo;
  proto.getBoundingClientRect = function rect(this: HTMLElement): DOMRect {
    const tl = has(this, 'tl') ? null : scroller(this);
    const y = tl ? top(this) - tl.scrollTop : 0;
    const h = height(this);
    return { x: 0, y, top: y, left: 0, right: 720, width: 720, height: h, bottom: y + h, toJSON: () => ({}) } as DOMRect;
  };
  globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    rafs.push({ id: ++rafId, cb });
    return rafId;
  }) as typeof requestAnimationFrame;
  globalThis.cancelAnimationFrame = ((id: number) => {
    rafs = rafs.filter((r) => r.id !== id);
  }) as typeof cancelAnimationFrame;
  class FlowResizeObserver {
    private readonly entry: { cb: Cb; targets: Map<Element, number> };
    constructor(cb: Cb) {
      this.entry = { cb, targets: new Map() };
      observers.add(this.entry);
    }
    observe(t: Element) {
      if (!this.entry.targets.has(t)) this.entry.targets.set(t, Number.NaN);
    }
    unobserve(t: Element) {
      this.entry.targets.delete(t);
    }
    disconnect() {
      this.entry.targets.clear();
      observers.delete(this.entry);
    }
  }
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = FlowResizeObserver;

  const micro = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  const deliverRO = () => {
    let any = false;
    for (const o of [...observers]) {
      const changed: Parameters<Cb>[0] = [];
      for (const [t, last] of o.targets) {
        if (!(t as HTMLElement).isConnected) continue;
        const h = height(t as HTMLElement);
        if (h !== last) {
          o.targets.set(t, h);
          changed.push({ target: t, borderBoxSize: [{ blockSize: h, inlineSize: 720 }], contentRect: { height: h, width: 720 } });
        }
      }
      if (changed.length) {
        any = true;
        o.cb(changed);
      }
    }
    return any;
  };
  layout.frame = async () => {
    await micro();
    for (const el of [...pendingScroll]) {
      pendingScroll.delete(el);
      el.dispatchEvent(new Event('scroll'));
    }
    await micro();
    const run = rafs;
    rafs = [];
    for (const r of run) r.cb(performance.now());
    await micro();
    // ResizeObserver: after layout; repeated while it keeps reporting (the browser's loop depth).
    for (let i = 0; i < 4 && deliverRO(); i++) await micro();
    await micro();
  };
  layout.frames = async (max = 30) => {
    for (let i = 0; i < max; i++) {
      await layout.frame();
      if (!pendingScroll.size && !rafs.length) {
        await new Promise((r) => setTimeout(r, 0));
        if (!pendingScroll.size && !rafs.length && !deliverRO()) return;
      }
    }
  };
  layout.wheelTo = async (tl, y) => {
    tl.dispatchEvent(new WheelEvent('wheel', { deltaY: y < tl.scrollTop ? -100 : 100 }));
    tops.set(tl, Math.max(0, Math.min(y, maxTop(tl))));
    pendingScroll.add(tl);
    await layout.frame();
  };
  layout.rows = (tl) =>
    [...tl.querySelectorAll<HTMLElement>('.tl-vrow')].map((el) => {
      const r = el.getBoundingClientRect();
      return { id: el.dataset.turn ?? '', top: r.top, bottom: r.bottom };
    });
  layout.restore = () => {
    for (const [name, d] of saved) {
      if (d) Object.defineProperty(proto, name, d);
      else delete (proto as unknown as Record<string, unknown>)[name];
    }
    (globalThis as { ResizeObserver?: unknown }).ResizeObserver = savedRO;
    globalThis.requestAnimationFrame = savedRAF;
    globalThis.cancelAnimationFrame = savedCAF;
    observers.clear();
  };
  return layout;
}
