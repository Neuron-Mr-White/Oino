/**
 * About section (WP-A24, G-14): one Update card (the merged UpdateCard: this
 * app's updater + the host row), the Components rows on the desktop (what
 * this install ships: app, agent host, oino engine, extensions — P0-01) or the
 * Version row elsewhere, platform/protocol rows, licences — plus the two rows
 * the built-in commands land on (WP-A25, G-15): **What's new**
 * (`/changelog`: the newest build's release notes from this platform's update
 * feed — the desktop updater or the APK feed — plus "Browse changelog", the
 * updater extension's changelog browser, P1-08) and **Report a bug**
 * (`/bug`: the repository's issue form in the system browser).
 */
import { Show, createResource, createSignal, onMount } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Bug, Info, Sparkles } from 'lucide-solid';
import { z } from 'zod';
import { call } from '../../lib/ipc';
import { platform } from '../../lib/platform';
import { updateState } from '../../lib/update';
import { checkDesktopUpdate, desktopUpdateState, type DesktopUpdateState } from '../../lib/desktopUpdate';
import { CHANGELOG_ACTION, runManageActionAnywhere } from '../../lib/settings/manage';
import { LOCAL_HOST } from '../../lib/host/client';
import { useHostsOptional } from '../../hosts/context';
import { can } from '../../lib/platform';
import { openExternal } from '../../lib/board/share';
import { UpdateCard } from '../../components/UpdateCard';
import { ComponentsCard } from '../../components/ComponentsCard';
import { ABOUT_ANCHORS } from './anchors';
import { Row, Section } from './rows';

const MODE_LABEL = { 'tauri-mobile': 'Phone app', 'tauri-desktop': 'Desktop app', web: 'Web' } as const;

function modeHint(): string | undefined {
  const mode = platform().mode;
  if (mode === 'web') return 'Running in a browser: only the board works here. Terminals, chat and files need the Unipi app.';
  if (mode === 'tauri-desktop') return 'Phone-only features (QR scan, haptics, push notifications) are hidden.';
  return undefined;
}

const AppInfoSchema = z.object({ version: z.string(), platform: z.string(), proto: z.string() });

type InfoResult = Awaited<ReturnType<typeof call<{ version: string; platform: string; proto: string }>>>;

/** One field of the app_info Result ('unavailable' until it answers). */
function valueOf(result: InfoResult | undefined, field: 'version' | 'platform' | 'proto'): string {
  return result?.ok ? result.value[field] : 'unavailable';
}

/** The release notes the update feed carries for the newest build, when it has any. */
function releaseNotes(state: ReturnType<typeof updateState>): string | undefined {
  if (state.kind === 'available' || state.kind === 'downloading' || state.kind === 'ready') {
    return (state as { release: { notes?: string } }).release.notes;
  }
  return undefined;
}

/** The desktop feed's notes: the newer build's when one is out. */
export function desktopNotes(state: DesktopUpdateState): { version?: string; notes: string } | undefined {
  const info = 'info' in state ? state.info : undefined;
  const notes = info?.notes?.trim();
  return notes ? { version: info?.version, notes } : undefined;
}

/** What's new for this platform (P1-08: the desktop reads its own updater feed, not the APK one). */
function whatsNew(): { version?: string; notes: string } | undefined {
  if (platform().mode === 'tauri-desktop') return desktopNotes(desktopUpdateState());
  const notes = releaseNotes(updateState())?.trim();
  return notes ? { notes } : undefined;
}

function noNotesText(): string {
  if (platform().mode !== 'tauri-desktop') return 'No release notes in the update feed yet';
  const s = desktopUpdateState();
  if (s.kind === 'checking' || s.kind === 'idle') return 'Checking the update feed…';
  if (s.kind === 'current') return 'You have the newest build';
  if (s.kind === 'error') return 'The update feed could not be reached';
  return 'This release has no notes';
}

export function AboutSection() {
  const [info] = createResource(async () => call('app_info', {}, AppInfoSchema));
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const [browsing, setBrowsing] = createSignal(false);
  const [browseNote, setBrowseNote] = createSignal<string | undefined>();
  onMount(() => {
    // The desktop feed is first checked 8 s after start; About asks now (quietly, rate-limited).
    if (platform().mode === 'tauri-desktop' && desktopUpdateState().kind === 'idle') void checkDesktopUpdate();
  });
  const browse = async () => {
    setBrowsing(true);
    setBrowseNote(undefined);
    const hostId = hosts?.hosts().find((h) => h.transport === 'local')?.id ?? LOCAL_HOST;
    const outcome = await runManageActionAnywhere(CHANGELOG_ACTION, { navigate, hostId }).finally(() => setBrowsing(false));
    if (outcome !== 'sent') setBrowseNote('The changelog browser needs the Updater extension (Settings › Extensions)');
  };
  return (
    <Section title="About" id="sec-about" icon={<Info size={14} />}>
      <div class="p-4 pb-0">
        <UpdateCard />
      </div>
      <Show when={platform().mode !== 'tauri-mobile'}>
        <Row label="Mode" description={modeHint()}>
          <span class="text-sm text-text-muted" data-testid="platform-mode">{MODE_LABEL[platform().mode]}</span>
        </Row>
      </Show>
      <Show
        when={platform().mode === 'tauri-desktop'}
        fallback={
          <Row label="Version">
            <span class="text-sm text-text-muted" data-testid="about-version">{valueOf(info(), 'version')}</span>
          </Row>
        }
      >
        <ComponentsCard />
      </Show>
      <Row label="Platform">
        <span class="text-sm text-text-muted">{valueOf(info(), 'platform')}</span>
      </Row>
      <Row label="Protocol">
        <span class="text-sm text-text-muted">{valueOf(info(), 'proto')}</span>
      </Row>
      <Row label="Licences" description="Inter and JetBrains Mono fonts: SIL Open Font License · Lucide icons: ISC">
        <span class="text-sm text-text-muted">MIT</span>
      </Row>
      <div id={ABOUT_ANCHORS['whats-new']} class="settings-anchored">
        <Row
          label="What’s new"
          description={whatsNew() ? (whatsNew()!.version ? `Release notes of ${whatsNew()!.version}` : 'Release notes of the newest build') : noNotesText()}
          icon={<Sparkles size={18} />}
        >
          <Show when={!whatsNew() && can('hosts')}>
            <button type="button" class="btn-secondary settings-btn" data-testid="about-browse-changelog" disabled={browsing()} onClick={() => void browse()}>
              {browsing() ? 'Opening…' : 'Browse changelog'}
            </button>
          </Show>
        </Row>
        <Show when={whatsNew() || browseNote()}>
          <div class="about-whats-new" data-testid="about-whats-new">
            <Show when={whatsNew()}>{(n) => <p class="about-notes">{n().notes}</p>}</Show>
            <Show when={browseNote()}>
              <p class="settings-field-hint settings-field-hint-bad" role="status">{browseNote()}</p>
            </Show>
          </div>
        </Show>
      </div>
      <div id={ABOUT_ANCHORS['report-bug']} class="settings-anchored">
        <Row label="Report a bug" description="Opens the project’s issue form in your browser" icon={<Bug size={18} />}>
          <button type="button" class="btn-secondary settings-btn" data-testid="about-report-bug" onClick={() => openExternal('https://github.com/Neuron-Mr-White/Oino/issues/new')}>
            Open
          </button>
        </Row>
      </div>
    </Section>
  );
}
