/**
 * Rendered task markdown (UNI-131): body, notes, reports, summaries.
 * Sanitised HTML from lib/board/markdown.ts; `att:` media are fetched
 * through the board transport (hydrateAttachments). Taps are handled
 * here so nothing navigates the WebView away: inline images open the
 * full-screen viewer (paging through every image of this block), file
 * chips open/save the file, external links open outside the app (or are
 * copied when the WebView can't).
 */
import { createEffect, on } from 'solid-js';
import { api, describe, filePath } from '../../lib/board/api';
import { attDisplayName, attKind, hydrateAttachments, renderMarkdown } from '../../lib/board/markdown';
import { hydrateCode } from '../../lib/markdown';
import { openMermaidViewer } from '../chat/MermaidViewer';
import { boardStore } from '../../lib/board/store';
import { boardUi, type ViewerItem } from '../../lib/board/ui';
import { copyText, saveFile } from '../../lib/board/share';
import { anchorFromEvent, isExternalHref, openLink } from '../../lib/markdown/links';
import { openLinkMenu } from '../LinkAnchorMenu';

/** Opens one attachment by daemon path: image → viewer, text → reader, other → share/download. */
export async function openFile(path: string, name: string, images: ViewerItem[] = [], size?: number): Promise<void> {
  const ui = boardUi();
  const at = boardStore().target();
  if (!at) return;
  const kind = attKind(name);
  const shown = attDisplayName(name);
  if (kind === 'image') {
    const items = images.length > 0 ? images : [{ path, name: shown, size }];
    ui.setViewer({ items, index: Math.max(0, items.findIndex((item) => item.path === path)) });
    return;
  }
  try {
    const blob = await api.file(at.hostId, path);
    if (kind === 'text') {
      ui.setReader({ title: shown, text: await blob.text(), mono: true });
      return;
    }
    const result = await saveFile(blob, shown);
    if (result === 'unsupported') {
      ui.toast(`Saving ${shown} isn't available in the app yet`, 'info', {
        label: 'Copy name',
        run: async () => void (await copyText(shown)),
      });
    }
  } catch (error) {
    ui.toast(describe(error), 'error');
  }
}

/** Attachment path for a task file (by stored name). */
export function taskFilePath(taskId: string, name: string): string {
  return filePath(boardStore().target()?.slug ?? '', taskId, name);
}

export function Markdown(props: { text: string | undefined; class?: string; empty?: string; label?: string }) {
  let el: HTMLDivElement | undefined;
  createEffect(
    on(
      () => [props.text, boardStore().target()?.hostId] as const,
      ([text, hostId]) => {
        if (!el) return;
        el.innerHTML = renderMarkdown(text) || (props.empty ? `<p class="md-empty">${props.empty}</p>` : '');
        void hydrateCode(el);
        if (hostId) void hydrateAttachments(el, hostId);
      },
    ),
  );
  const onClick = (event: MouseEvent) => {
    const target = event.target as HTMLElement;
    const copy = target.closest<HTMLButtonElement>('[data-copy]');
    if (copy && el?.contains(copy)) {
      const code = copy.closest('.cb')?.querySelector('code')?.textContent ?? '';
      void copyText(code).then((ok) => {
        copy.textContent = ok ? 'Copied' : 'Copy';
        setTimeout(() => (copy.textContent = 'Copy'), 1200);
      });
      return;
    }
    const mmd = target.closest<HTMLElement>('[data-mmd-tap]');
    if (mmd && el?.contains(mmd)) {
      const source = decodeURIComponent(mmd.closest<HTMLElement>('.mmd')?.dataset.mmdSource ?? '');
      openMermaidViewer({ svg: mmd.innerHTML, source });
      return;
    }
    const wrapToggle = target.closest<HTMLButtonElement>('[data-wrap-toggle]');
    if (wrapToggle && el?.contains(wrapToggle)) {
      wrapToggle.closest('.cb')?.classList.toggle('wrapped');
      return;
    }
    const anchor = target.closest('a');
    if (!anchor || !el?.contains(anchor)) return;
    event.preventDefault();
    event.stopPropagation();
    const path = anchor.dataset.attPath;
    if (path && anchor.classList.contains('att-image')) {
      const images = [...el.querySelectorAll<HTMLAnchorElement>('a.att-image')].map((node) => ({
        path: node.dataset.attPath ?? '',
        name: node.querySelector('img')?.alt || attDisplayName(node.dataset.attPath?.split('/').pop() ?? ''),
      }));
      void openFile(path, path.split('/').pop() ?? '', images);
      return;
    }
    if (path) {
      void openFile(path, anchor.dataset.attName ?? path.split('/').pop() ?? '');
      return;
    }
    const href = anchor.getAttribute('href') ?? '';
    if (isExternalHref(href)) {
      void openLink(href).then((result) => {
        if (result !== 'opened') boardUi().toast(result === 'copied' ? 'Copied the link' : href, result === 'copied' ? 'success' : 'info');
      });
    }
  };
  const onContextMenu = (event: MouseEvent) => {
    const anchor = anchorFromEvent(event, el ?? document.body);
    if (!anchor || !isExternalHref(anchor.getAttribute('href') ?? '')) return;
    event.preventDefault();
    event.stopPropagation();
    openLinkMenu(anchor);
  };
  return <div ref={el} class={`md ${props.class ?? ''}`} aria-label={props.label} onClick={onClick} onContextMenu={onContextMenu} />;
}
