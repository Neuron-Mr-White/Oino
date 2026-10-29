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

type LinkMenuState = { href: string; rect: DOMRect };

const [menu, setMenu] = createSignal<LinkMenuState | undefined>(undefined);

/** Opens the link menu for a rendered markdown anchor. */
export function openLinkMenu(anchor: HTMLAnchorElement): void {
  const href = anchor.getAttribute('href') ?? '';
  if (!href) return;
  setMenu({ href, rect: anchor.getBoundingClientRect() });
}

/** Closes the menu (also used between tests). */
export function closeLinkMenu(): void {
  setMenu(undefined);
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
  );
}
