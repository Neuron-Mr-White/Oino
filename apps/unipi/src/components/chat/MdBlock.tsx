/**
 * One rendered markdown block (chat prose/thinking/cards, UNI-156): sets
 * the sanitised HTML then hydrates closed code fences (lazy
 * CodeMirror/Lezer highlight) and mermaid diagrams in place.
 *
 * UNI-204 (was UNI-157): file paths anywhere in the block — prose, inline
 * code, fenced code, markdown link targets — are scanned, verified to
 * exist on the PC (`paths_stat`, lib/chat/fileRefs.ts) and only then made
 * tappable (`[data-file-ref]`): tap opens the file (or the editor at
 * `:line`, or Files for a dir), long-press/right-click opens the file
 * menu. Only when a `chat` is given (board/card call sites have no
 * session, so no refs). Inline code that is exactly a verified path also
 * gets a small copy affordance (`.code-copy`, UNI-208).
 *
 * UNI-208: any other inline code (not a path, not in a fenced block) is
 * tap-to-copy instead — a brief `.code-copied` class flashes on the
 * element, no toast plumbing. Never calls `preventDefault` on
 * `touchstart`/`contextmenu` so long-press text selection still works.
 */
import { createEffect, onCleanup } from 'solid-js';
import { hydrateCode } from '../../lib/chat/markdown';
import { openMermaidViewer } from './MermaidViewer';
import { openLinkMenu } from '../LinkAnchorMenu';
import { anchorFromEvent, isExternalHref, openLink } from '../../lib/markdown/links';
import { copyText } from '../../lib/board/share';
import type { Chat } from '../../lib/chat/store';
import { hydrateTaskRefs, knownPrefixes } from '../../lib/board/portal';
import { openTaskFromChat } from '../../lib/chat/boardPortal';
import { hydrateFileRefs } from '../../lib/chat/fileRefs';
import { attachFileRefGestures, handleFileRefClick } from '../../lib/chat/fileRefGestures';

const COPY_FLASH_MS = 1200;

/** Briefly flashes `.code-copied` on `el` (CSS handles the visual, no toast). */
function flashCopied(el: HTMLElement): void {
  el.classList.add('code-copied');
  setTimeout(() => el.classList.remove('code-copied'), COPY_FLASH_MS);
}

/** Adds the tiny copy-icon affordance after a path-like inline `<code>`
 * (UNI-208) — idempotent, so re-renders don't double it up. */
function ensurePathCopyButton(code: HTMLElement): void {
  if (code.nextElementSibling?.classList.contains('code-copy')) return;
  const btn = code.ownerDocument.createElement('button');
  btn.type = 'button';
  btn.className = 'code-copy';
  btn.setAttribute('aria-label', 'Copy path');
  btn.dataset.copyPath = '1';
  code.after(btn);
}

export function MdBlock(props: { html: string; class?: string; chat?: Chat }) {
  let el: HTMLDivElement | undefined;
  let pass = 0;
  createEffect(() => {
    const html = props.html;
    if (!el) return;
    el.innerHTML = html;
    const mine = ++pass;
    const highlighted = hydrateCode(el);
    // Task refs (UNI-181): only in chat, against this PC's real prefixes.
    // File refs (UNI-204) run after highlighting (it replaces fenced code
    // contents) and task refs, so the passes never fight over text nodes.
    const chat = props.chat;
    if (chat) {
      const target = el;
      const current = () => mine === pass;
      void Promise.all([knownPrefixes(chat.hostId), highlighted.catch(() => undefined)]).then(async ([prefixes]) => {
        if (!current()) return;
        hydrateTaskRefs(target, prefixes);
        await hydrateFileRefs(target, chat, current);
        if (!current()) return;
        for (const code of target.querySelectorAll<HTMLElement>('code.file-ref-code')) ensurePathCopyButton(code);
      });
    }
  });
  createEffect(() => {
    if (!el || !props.chat) return;
    onCleanup(attachFileRefGestures(el, () => props.chat));
  });
  const onClick = (event: MouseEvent) => {
    const svg = (event.target as HTMLElement).closest<HTMLElement>('[data-mmd-tap]');
    if (svg) {
      const source = decodeURIComponent(svg.closest<HTMLElement>('.mmd')?.dataset.mmdSource ?? '');
      openMermaidViewer({ svg: svg.innerHTML, source });
      return;
    }
    // Task refs (UNI-181): open the task's pane on the board.
    const taskRef = (event.target as HTMLElement).closest<HTMLElement>('[data-task]');
    if (taskRef && props.chat && el?.contains(taskRef)) {
      event.preventDefault();
      event.stopPropagation();
      openTaskFromChat(props.chat.hostId, taskRef.dataset.task!, props.chat.state?.session?.cwd);
      return;
    }
    // File refs (UNI-204): verified paths in prose, code and link targets.
    if (el && handleFileRefClick(event, el, props.chat)) return;
    // Links (UNI-170): external anchors open outside the app (copy
    // fallback); any other anchor is inert but must not navigate the
    // WebView away.
    const anchor = el ? anchorFromEvent(event, el) : null;
    if (anchor) {
      const href = anchor.getAttribute('href') ?? '';
      if (isExternalHref(href)) {
        event.preventDefault();
        event.stopPropagation();
        void openLink(href);
      } else {
        event.preventDefault();
      }
      return;
    }
    // Path-copy button beside a path-like inline code (UNI-208).
    const copyBtn = (event.target as HTMLElement).closest<HTMLElement>('[data-copy-path]');
    if (copyBtn && el?.contains(copyBtn)) {
      event.stopPropagation();
      const code = copyBtn.previousElementSibling as HTMLElement | null;
      void copyText(code?.textContent ?? '').then((ok) => {
        if (ok) flashCopied(copyBtn);
      });
      return;
    }
    const code = (event.target as HTMLElement).closest<HTMLElement>('code');
    if (!code || code.closest('pre') || code.classList.contains('task-ref') || !el?.contains(code)) return; // inline only, not a fenced block or task ref
    // Any other inline code copies its text (UNI-208) — no preventDefault,
    // so long-press text selection still works.
    void copyText(code.textContent ?? '').then((ok) => {
      if (ok) flashCopied(code);
    });
  };
  const onContextMenu = (event: MouseEvent) => {
    const anchor = el ? anchorFromEvent(event, el) : null;
    if (!anchor || !isExternalHref(anchor.getAttribute('href') ?? '')) return;
    event.preventDefault();
    event.stopPropagation();
    openLinkMenu(anchor);
  };
  return <div ref={el} class={props.class} onClick={onClick} onContextMenu={onContextMenu} />;
}
