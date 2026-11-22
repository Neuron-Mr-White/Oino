/**
 * Long-press menu for markdown links (UNI-170): Open / Copy link /
 * Share, anchored to the link. Signal-store driven like the previewer,
 * so every markdown surface (chat MdBlock, board Markdown, previewer)
 * can open it without prop drilling; mounted once in the Shell.
 */
import { createSignal, For, Show } from 'solid-js';
import { Copy, ExternalLink, Share2 } from 'lucide-solid';
import { AnchoredMenu, MenuRow } from './board/AnchoredMenu';
import { copyText } from '../lib/board/share';
import { openLink } from '../lib/markdown/links';
import { desktopShell, menuAnchor } from '../lib/layout';
import { showActionNotice } from '../lib/timeline/actions';

type LinkMenuState = { href: string; rect: DOMRect };

const [menu, setMenu] = createSignal<LinkMenuState | undefined>(undefined);

/** Opens the link menu for a rendered markdown anchor. */
export function openLinkMenu(anchor: HTMLAnchorElement): void {
  const href = anchor.getAttribute('href') ?? '';
  if (!href) return;
  setMenu({ href, rect: anchor.getBoundingClientRect() });
}

type CodeMenuState = { text: string; rect: DOMRect };

const [codeMenu, setCodeMenu] = createSignal<CodeMenuState | undefined>(undefined);

/** OIN-97: opens the one-item "Copy" menu for a rendered inline `<code>` (its inner text, exactly). */
export function openCodeMenu(text: string, rect: DOMRect): void {
  setCodeMenu({ text, rect });
}

/**
 * OIN-97: a desktop right-click on rendered inline code opens the one-item "Copy" menu (the code's
 * inner text, exactly) and returns true — the caller stops there. Fenced blocks, links, verified
 * file refs (their own menu), task refs and a live text selection keep their behaviour (false).
 *
 * Register it with a NATIVE `contextmenu` listener on the markdown root, not Solid's `onContextMenu`:
 * Solid delegates that event to the document, so the `stopPropagation` here would come too late to
 * keep the message menu of an ancestor (`attachMessageMenu`) from opening on top of this one.
 */
export function openInlineCodeMenu(event: MouseEvent, root: HTMLElement): boolean {
  const code = (event.target as HTMLElement | null)?.closest?.<HTMLElement>('code');
  if (!desktopShell() || !code || !root.contains(code)) return false;
  if (code.closest('pre, a, [data-file-ref]') || code.classList.contains('task-ref')) return false;
  if (!(window.getSelection()?.isCollapsed ?? true)) return false;
  event.preventDefault();
  event.stopPropagation();
  openCodeMenu(code.textContent ?? '', menuAnchor(event, code));
  return true;
}

/** Closes the menus (also used between tests). */
export function closeLinkMenu(): void {
  setMenu(undefined);
  setCodeMenu(undefined);
}

function CodeCopyMenu() {
  const close = () => setCodeMenu(undefined);
  return (
    <Show when={codeMenu()}>
      <AnchoredMenu anchor={codeMenu()!.rect} label="Code actions" onDismiss={close} width={180}>
        <MenuRow
          icon={<Copy size={18} />}
          label="Copy"
          onSelect={() => {
            void copyText(codeMenu()!.text).then((ok) => showActionNotice(ok ? 'Copied' : "Couldn't copy the code."));
            close();
          }}
        />
      </AnchoredMenu>
    </Show>
  );
}

export function LinkAnchorMenu() {
  const [copied, setCopied] = createSignal(false);
  const close = () => (setMenu(undefined), setCopied(false));
  const rows = [
    { icon: <ExternalLink size={18} />, label: 'Open', run: () => void openLink(menu()!.href) },
    {
      icon: <Copy size={18} />,
      label: 'Copy link',
      run: () =>
        void copyText(menu()!.href).then((ok) => {
          if (ok) {
            setCopied(true);
            setTimeout(() => close(), 600);
          }
        }),
    },
    ...(typeof navigator !== 'undefined' && 'share' in navigator
      ? [
          {
            icon: <Share2 size={18} />,
            label: 'Share',
            run: () => void navigator.share({ url: menu()!.href }).catch(() => {}),
          },
        ]
      : []),
  ];
  return (
    <>
      <CodeCopyMenu />
      <Show when={menu()}>
        <AnchoredMenu anchor={menu()!.rect} label="Link actions" onDismiss={close} width={220}>
          <For each={rows}>
            {(row) => (
              <MenuRow
                icon={row.icon}
                label={row.label === 'Copy link' && copied() ? 'Copied' : row.label}
                onSelect={() => {
                  if (row.label === 'Copy link' && copied()) return close();
                  row.run();
                  if (row.label !== 'Copy link') close();
                }}
              />
            )}
          </For>
        </AnchoredMenu>
      </Show>
    </>
  );
}
