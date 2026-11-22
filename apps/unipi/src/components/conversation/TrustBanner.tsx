/**
 * The project-trust banner (OIN-101 WI-3, SPEC §1): a folder with project settings (MCP servers, skills…) that
 * has no saved decision opens untrusted, and says so here — above the conversation, never blocking the composer:
 *
 *   This folder has project settings: MCP servers (.oino/mcp.json), skills.   [Trust folder] [Trust parent ▾] [Don’t trust]
 *
 * The choice is saved (`_oino/trust_set {path, trusted}`) and never asked again; "Don’t trust" hides the banner
 * for good (Settings › MCP › Project says "Not trusted" with [Trust this project]). The state comes from the
 * session snapshot's `project_trust`; an older host sends none and nothing shows.
 */
import { createSignal, For, Show } from 'solid-js';
import { ChevronDown, ShieldQuestion } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { describeProtected, needsTrustDecision, noteTrustDecision, parentChoices, trustDecisions, trustFolder } from '../../lib/session/trust';
import { toasts } from '../../lib/toasts';
import { AnchoredMenu, MenuRow } from '../board/AnchoredMenu';
import { shortCwd } from '../../lib/session/selectors';
import { desktopShell } from '../../lib/layout';
import './trust-banner.css';

/** Saves one decision on the chat's own stream and reports it. */
export async function decideTrust(session: Pick<SessionHandle, 'cmd'>, path: string, trusted: boolean, say = toasts()): Promise<boolean> {
  try {
    await session.cmd({ type: '_oino/trust_set', path, trusted });
    noteTrustDecision(path, trusted);
    say.push(trusted ? `Trusted ${shortCwd(path)} — loading its project settings` : `Not trusting ${shortCwd(path)} — its project settings stay off`, trusted ? 'success' : 'info');
    return true;
  } catch (e) {
    say.push(`Couldn’t save the trust decision: ${String((e as { message?: unknown })?.message ?? e)}`, 'error');
    return false;
  }
}

export function TrustBanner(props: { session: SessionHandle; cwd?: string }) {
  const trust = () => props.session.state.projectTrust;
  const show = () => needsTrustDecision(trust(), props.cwd, trustDecisions());
  const folder = () => (trust() ? trustFolder(trust()!, props.cwd) : props.cwd ?? '');
  const [busy, setBusy] = createSignal(false);
  const [menuAt, setMenuAt] = createSignal<DOMRect | undefined>();
  const decide = async (path: string, trusted: boolean) => {
    if (busy() || !path) return;
    setBusy(true);
    try {
      await decideTrust(props.session, path, trusted);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Show when={show()}>
      <div class="chat-banner tl-banner trust-banner" data-kind="trust" role="region" aria-label="Project trust" data-testid="trust-banner">
        <ShieldQuestion size={18} class="shrink-0" aria-hidden="true" />
        <span class="trust-banner-text min-w-0 flex-1">
          This folder has project settings: {describeProtected(trust()!.protected)}. They are off until you trust it.
        </span>
        <span class="trust-banner-actions">
          <button type="button" class="trust-banner-btn" data-primary="" disabled={busy()} onClick={() => void decide(folder(), true)} data-testid="trust-folder">
            Trust folder
          </button>
          <Show when={parentChoices(folder()).length > 0}>
            <button
              type="button"
              class="trust-banner-btn"
              disabled={busy()}
              aria-haspopup="menu"
              aria-expanded={!!menuAt()}
              data-testid="trust-parent"
              onClick={(e) => setMenuAt(e.currentTarget.getBoundingClientRect())}
            >
              Trust parent <ChevronDown size={13} aria-hidden="true" />
            </button>
          </Show>
          <button type="button" class="trust-banner-btn" disabled={busy()} onClick={() => void decide(folder(), false)} data-testid="trust-never">
            Don’t trust
          </button>
        </span>
        <Show when={menuAt()}>
          {(anchor) => (
            <AnchoredMenu anchor={anchor()} width={desktopShell() ? 300 : 280} label="Trust a parent folder" title="Trust a parent folder" onDismiss={() => setMenuAt(undefined)}>
              <For each={parentChoices(folder())}>
                {(p) => (
                  <MenuRow
                    label={shortCwd(p)}
                    testId={`trust-parent-${p}`}
                    onSelect={() => {
                      setMenuAt(undefined);
                      void decide(p, true);
                    }}
                  />
                )}
              </For>
            </AnchoredMenu>
          )}
        </Show>
      </div>
    </Show>
  );
}
