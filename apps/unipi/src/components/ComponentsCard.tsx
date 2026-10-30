/**
 * Settings → This computer → Components (UNI-241 / plan S3, §4.3 point 6).
 *
 * Rows come from two core calls — `setup_detect` (what this PC has) and
 * `setup_check_updates` (what is newer, from four feeds) — plus the desktop
 * app's own row, which is in neither report: its check is the signed updater
 * in `lib/desktopUpdate.ts` (§4.3 / UNI-263).
 *
 * The row actions:
 * - **Check for updates** — reads the feeds again; never installs anything;
 * - **Update all / Repair** — the same `setup_run` engine as a first install;
 * - **Install for me** (manual components, e.g. xvfb) — decision 8's method chain, with the
 *   command to do it yourself as the last resort (Copy).
 */
import { For, Show, createSignal, onMount, type JSX } from 'solid-js';
import { AlertTriangle, Check, Copy, Download, ExternalLink, Puzzle, RefreshCw, Server, Sparkles, Terminal, Wrench, Columns3, Globe } from 'lucide-solid';
import { checkDesktopUpdate, desktopUpdateState, installDesktopUpdate } from '../lib/desktopUpdate';
import { openExternal } from '../lib/board/share';
import {
  badgeText,
  checkForUpdates,
  componentLabel,
  installComponent,
  refreshSetup,
  setupRows,
  setupReport,
  updateReport,
  type SetupRow,
} from '../lib/setup';
import { can } from '../lib/platform';
import './setup.css';

export function ComponentsCard() {
  const [busyId, setBusyId] = createSignal<string | undefined>();
  const [copied, setCopied] = createSignal<string | undefined>();
  const [error, setError] = createSignal<string | undefined>();

  onMount(() => {
    void refreshSetup();
    void checkForUpdates();
    // The desktop app's own row comes from the signed updater's state, which
    // the app checks on start (§4.3 point 6); ask here too, so the row is right
    // even when the card is opened before that landed.
    void checkDesktopUpdate();
  });

  const rows = (): SetupRow[] => setupRows(setupReport(), updateReport());
  const updates = updateReport;
  const desktopState = desktopUpdateState;
  const desktop = desktopState;
  const todo = (): SetupRow[] => rows().filter((r) => r.ticked && !r.manual);
  /** The newer desktop version on offer, when the signed updater found one. */
  const desktopUpdate = (): { kind: 'in_app' | 'manual'; downloadPage: string } | undefined => {
    const s = desktopUpdateState();
    return s.kind === 'available' ? s.info : undefined;
  };

  /** Repair / Update: the install engine for that one component. */
  async function fix(id: string): Promise<void> {
    setBusyId(id);
    setError(undefined);
    const outcome = await installComponent(id, () => {});
    setBusyId(undefined);
    if (!outcome) setError(`Could not install ${componentLabel(id)}`);
    else await refreshSetup();
  }

  /** §4.3: Update all runs the same engine, one component at a time. */
  async function fixAll(): Promise<void> {
    setError(undefined);
    for (const row of todo()) {
      setBusyId(row.id);
      const outcome = await installComponent(row.id, () => {});
      if (!outcome) {
        setError(`Could not install ${row.label}`);
        setBusyId(undefined);
        return;
      }
    }
    setBusyId(undefined);
    await refreshSetup();
  }

  async function copyFix(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(text);
      setTimeout(() => setCopied(undefined), 2000);
    } catch {
      setError('Could not reach the clipboard — the command is above');
    }
  }

  return (
    <div class="setup-components" data-testid="setup-components">
      {/* A card of its own, like every other Settings block: without it the rows
          sat flush to the left edge with no icon, and the action bar was clipped
          by the section's bottom. */}
      <section class="settings-group rounded-card bg-surface-1" data-section="Versions">
        <h2 class="section-label settings-section-label">
          <Wrench size={14} /> Versions
        </h2>
        <p class="setup-components-intro">What this PC has, and what the feeds offer. Updating runs the same installer as the first run.</p>
        <Show when={updates()?.offline}>
          <p class="setup-offline" data-testid="components-offline">
            <AlertTriangle size={14} /> Offline, so versions may be stale. What is installed still works.
          </p>
        </Show>
        <Show when={error()}>
          <p class="setup-error-message" data-testid="components-error">
            {error()}
          </p>
        </Show>

        <ul class="setup-rows">
          <For each={rows()}>
            {(row) => {
              const component = () => setupReport()?.components.find((c) => c.name === row.id);
              return (
                <li class="setup-components-row" data-testid={`components-row-${row.id}`} data-state={row.state}>
                  <span class="settings-icon">{ROW_ICON[row.id] ?? <Wrench size={18} />}</span>
                  <span class="setup-components-text">
                    <span class="setup-components-label">
                      {row.label}
                      <em class="setup-badge" data-badge={row.badge}>
                        {badgeText(row.badge, row)}
                      </em>
                    </span>
                    <span class="setup-components-note">{row.note}</span>
                  </span>
                  <span class="setup-components-actions">
                    {/* §4.3 point 6: Repair / Update runs the same engine as a
                        first install. For manual components that is decision 8's
                        method chain, which is what "Install for me" invokes. */}
                    <Show when={row.ticked || (row.manual && row.state !== 'ok')}>
                      <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        data-testid={`components-repair-${row.id}`}
                        disabled={!!busyId() || !can('selfInstall')}
                        onClick={() => void fix(row.id)}
                      >
                        <Download size={14} /> {row.action === 'update' ? 'Update' : 'Install for me'}
                      </button>
                    </Show>
                    <Show when={row.manual && component()?.fix}>
                      <button
                        type="button"
                        class="btn btn-secondary btn-sm"
                        data-testid={`components-copy-${row.id}`}
                        onClick={() => void copyFix(component()!.fix!)}
                      >
                        <Copy size={14} /> {copied() === component()!.fix ? 'Copied' : 'Copy'}
                      </button>
                    </Show>
                  </span>
                </li>
              );
            }}
          </For>

          {/* The desktop app: the signed updater, not the setup engine. */}
          <Show when={can('desktopUpdater')}>
            <li class="setup-components-row" data-testid="components-row-desktop">
              <span class="setup-components-text">
                <span class="setup-components-label">
                  UniPi desktop app
                  <Show when={desktopDetail()}>
                    {' '}
                    <em class="setup-components-detail">{desktopDetail()}</em>
                  </Show>
                  <Show when={desktop().kind === 'checking'}>
                    {' '}
                    <em class="setup-components-detail">Checking…</em>
                  </Show>
                  <Show when={desktop().kind === 'error'}>
                    {' '}
                    <em class="setup-components-detail">Check failed</em>
                  </Show>
                </span>
                <span class="setup-components-note">{desktopNote()}</span>
              </span>
              <span class="setup-components-actions">
                <Show when={desktopUpdate()}>
                  {(offer) => (
                    <>
                      <Show
                        when={offer().kind === 'in_app'}
                        fallback={
                          <button
                            type="button"
                            class="btn btn-primary btn-sm"
                            data-testid="components-desktop-download"
                            onClick={() => void openExternal(offer().downloadPage)}
                          >
                            <ExternalLink size={14} /> Download
                          </button>
                        }
                      >
                        <button type="button" class="btn btn-primary btn-sm" data-testid="components-desktop-update" onClick={() => void installDesktopUpdate()}>
                          <Download size={14} /> Update
                        </button>
                      </Show>
                    </>
                  )}
                </Show>
                <button type="button" class="btn btn-secondary btn-sm" data-testid="components-check" onClick={() => void checkForUpdates()}>
                  <RefreshCw size={14} /> Check
                </button>
              </span>
            </li>
          </Show>
        </ul>

        <div class="setup-components-foot">
          <Show when={todo().length > 1}>
            <button type="button" class="btn btn-primary" data-testid="components-update-all" disabled={!!busyId() || !can('selfInstall')} onClick={() => void fixAll()}>
              <Download size={14} /> Update all
            </button>
          </Show>
          <button
            type="button"
            class="btn btn-secondary"
            data-testid="components-check-updates"
            disabled={!!busyId()}
            onClick={() => void checkForUpdates()}
          >
            <RefreshCw size={14} /> Check for updates
          </button>
          <Show when={updates()?.anyAvailable}>
            <span class="setup-components-hint" data-testid="components-hint">
              <Check size={14} /> {todo().length} to do
            </span>
          </Show>
        </div>
      </section>
    </div>
  );
}

/** A per-component icon, so the block reads like the other Settings rows. */
const ROW_ICON: Record<string, JSX.Element> = {
  node: <Terminal size={18} />,
  npm: <Terminal size={18} />,
  pi: <Sparkles size={18} />,
  unipi: <Puzzle size={18} />,
  kanboard: <Columns3 size={18} />,
  xvfb: <Globe size={18} />,
  host: <Server size={18} />,
};

/** The desktop app's one-line state, from the signed updater's own state. */
function desktopDetail(): string | undefined {
  const s = desktopUpdateState();
  if (s.kind === 'current') return `Up to date — ${s.info.current}`;
  if (s.kind === 'available') return `Update ${s.info.current} → ${s.info.version}`;
  if (s.kind === 'checking') return 'Checking…';
  if (s.kind === 'error') return 'The update check failed';
  return undefined;
}

/** Why the desktop row can or cannot update in place. */
function desktopNote(): string {
  const s = desktopUpdateState();
  if (s.kind === 'available') return s.info.kind === 'in_app' ? 'Updates in place, then restarts' : 'A .deb / .rpm can’t replace itself — download the new version';
  if (s.kind === 'current') return 'Up to date';
  if (s.kind === 'error') return 'The update check failed';
  return 'Signed updates; a .deb or .rpm downloads the new version instead';
}
