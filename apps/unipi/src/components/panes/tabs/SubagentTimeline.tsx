/**
 * A subagent's transcript, drawn exactly like the main chat: the child's messages become
 * session entries, `buildTurns` makes the same turns (user bubble, "Worked for…" fold, tool
 * rows, markdown answers) and `TurnRow` renders them in the same `.tl` scroller. Read only
 * (`ctx.readOnly`): no edit / retry / return-here on a child.
 *
 * Scrolling is the main chat's bottom lock (`lib/timeline/bottomLock`, the ONE writer of
 * `scrollTop`): the view opens at the bottom and follows new output while it is there; a
 * reader gesture up detaches it (the jump button re-attaches), and nothing else does.
 */
import { createEffect, createMemo, For, onCleanup, onMount, createSignal } from 'solid-js';
import { createBottomLock } from '../../../lib/timeline/bottomLock';
import { createInnerFollow, INNER_SCROLLERS } from '../../../lib/timeline/innerFollow';
import { buildTurns } from '../../../lib/session/turns';
import type { AgentMessage, SessionEntry } from '../../../lib/session/types';
import type { SessionHandle } from '../../../lib/session/store';
import { sessionRefChat } from '../../../lib/timeline/refChat';
import { commandNames, type TimelineCtx } from '../../timeline/context';
import { TurnRow } from '../../timeline/TurnRow';
import { JumpButton } from '../../timeline/JumpButton';
import '../../timeline/timeline.css';

/** The child's messages as a linear branch of session entries (ids are scoped to the agent). */
export function entriesOfMessages(agentId: string, messages: readonly AgentMessage[]): SessionEntry[] {
  let prev: string | null = null;
  return messages.map((message, i) => {
    const id = `sa:${agentId}:${i}`;
    const ts = (message as { timestamp?: unknown }).timestamp;
    const entry: SessionEntry = {
      type: 'message',
      id,
      parentId: prev,
      ...(typeof ts === 'number' && Number.isFinite(ts) ? { timestamp: new Date(ts).toISOString() } : {}),
      message,
    };
    prev = id;
    return entry;
  });
}

export function SubagentTimeline(props: { session: SessionHandle; agentId: string; cwd?: string; messages: readonly AgentMessage[]; running: boolean }) {
  const lock = createBottomLock();
  const inner = createInnerFollow();
  const [scrollEl, setScrollEl] = createSignal<HTMLDivElement>();
  const [overflowing, setOverflowing] = createSignal(false);
  let contentEl: HTMLDivElement | undefined;

  const entries = createMemo(() => entriesOfMessages(props.agentId, props.messages));
  const turns = createMemo(() => {
    const list = entries();
    return buildTurns({ entries: list, leafId: list[list.length - 1]?.id ?? null, entriesRev: list.length, liveTools: {}, run: { running: props.running, startedAt: undefined } });
  });
  const [width, setWidth] = createSignal(0);
  const ctx: TimelineCtx = {
    session: props.session,
    hostId: props.session.hostId,
    sessionId: props.session.sessionId,
    cwd: () => props.cwd,
    refs: sessionRefChat(props.session, () => props.cwd),
    commands: () => commandNames(props.session.state.commands),
    columnWidth: () => width() || 640,
    readOnly: true,
  };

  /** The one place that scrolls: keep the bottom while locked. */
  const settle = () => {
    const el = scrollEl();
    if (!el) return;
    if (lock.locked()) lock.pin(el);
    lock.reconcile(el);
    setOverflowing(el.scrollHeight - el.clientHeight > 4);
    setWidth(el.clientWidth);
  };

  // New output after the detach is "unread"; a locked view just follows it.
  let seenTurns = turns().length;
  let seenLast = props.messages.length;
  createEffect(() => {
    const n = turns().length;
    const last = props.messages.length;
    const grew = n > seenTurns || last > seenLast;
    seenTurns = n;
    seenLast = last;
    queueMicrotask(() => {
      const el = scrollEl();
      if (!el) return;
      if (grew && !lock.locked()) lock.arrived();
      settle();
    });
  });

  onMount(() => {
    const el = scrollEl()!;
    // Open at the bottom: the lock starts locked; pin now and again after the first paint
    // (markdown / fonts settle the height).
    lock.jump(el);
    settle();
    requestAnimationFrame(() => {
      settle();
      requestAnimationFrame(settle);
    });
    const up = () => lock.gestureUp(el);
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) up();
    };
    let touchY: number | undefined;
    const onTouchStart = (e: TouchEvent) => {
      touchY = e.touches[0]?.clientY;
    };
    const onTouchMove = (e: TouchEvent) => {
      const y = e.touches[0]?.clientY;
      if (y !== undefined && touchY !== undefined && y > touchY + 2) up();
      touchY = y;
    };
    const onPointerDown = (e: PointerEvent) => {
      if (e.target === el) up();
    };
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.closest?.('input, textarea, [contenteditable="true"]')) return;
      if (['PageUp', 'ArrowUp', 'Home'].includes(e.key)) up();
    };
    el.addEventListener('wheel', onWheel, { passive: true });
    el.addEventListener('touchstart', onTouchStart, { passive: true });
    el.addEventListener('touchmove', onTouchMove, { passive: true });
    el.addEventListener('pointerdown', onPointerDown, { passive: true });
    el.addEventListener('keydown', onKey);
    onCleanup(() => {
      el.removeEventListener('wheel', onWheel);
      el.removeEventListener('touchstart', onTouchStart);
      el.removeEventListener('touchmove', onTouchMove);
      el.removeEventListener('pointerdown', onPointerDown);
      el.removeEventListener('keydown', onKey);
    });
    if (typeof ResizeObserver !== 'undefined') {
      const ro = new ResizeObserver(() => settle());
      if (contentEl) ro.observe(contentEl);
      ro.observe(el);
      onCleanup(() => ro.disconnect());
    }
    // Output growing inside a capped accordion (tool output, a nested fold) follows its own bottom.
    if (contentEl && typeof MutationObserver !== 'undefined') {
      let queued = false;
      const mo = new MutationObserver(() => {
        if (queued) return;
        queued = true;
        requestAnimationFrame(() => {
          queued = false;
          if (contentEl && inner.sweep(contentEl.querySelectorAll<HTMLElement>(INNER_SCROLLERS)) && !lock.locked()) lock.arrived();
        });
      });
      mo.observe(contentEl, { childList: true, subtree: true, characterData: true });
      onCleanup(() => mo.disconnect());
    }
  });

  const onScroll = () => {
    const el = scrollEl();
    if (!el) return;
    if (lock.onScroll(el)) settle();
    else setOverflowing(el.scrollHeight - el.clientHeight > 4);
  };

  return (
    <div class="tl-outer stab-tl">
      <div class="tl" ref={setScrollEl} data-testid="stab-timeline" onScroll={onScroll}>
        <div class="tl-content" ref={contentEl}>
          <div class="tl-list">
            <For each={turns()}>{(turn, i) => <div class="tl-vrow"><TurnRow turn={turn} ctx={ctx} last={i() === turns().length - 1} /></div>}</For>
          </div>
        </div>
        <div class="tl-dock">
          <JumpButton show={lock.detached() && overflowing()} unread={lock.unread()} onJump={() => scrollEl() && (lock.jump(scrollEl()!), settle())} />
        </div>
      </div>
    </div>
  );
}

