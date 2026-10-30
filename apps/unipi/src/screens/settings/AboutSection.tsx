/**
 * About section (WP-A24, G-14): one Update card (the merged UpdateCard: this
 * app's updater + the host row), version/platform/protocol rows, licences —
 * plus the two rows the pi built-ins land on (WP-A25, G-15): **What's new**
 * (`/changelog`: the update feed's release notes) and **Report a bug**
 * (`/bug`: the repository's issue form in the system browser).
 */
import { Show, createResource } from 'solid-js';
import { Bug, Info, Sparkles } from 'lucide-solid';
import { z } from 'zod';
import { call } from '../../lib/ipc';
import { platform } from '../../lib/platform';
import { updateState } from '../../lib/update';
import { openExternal } from '../../lib/board/share';
import { UpdateCard } from '../../components/UpdateCard';
import { ABOUT_ANCHORS } from './anchors';
import { Row, Section } from './rows';

const MODE_LABEL = { 'tauri-mobile': 'Phone app', 'tauri-desktop': 'Desktop app', web: 'Web' } as const;

function modeHint(): string | undefined {
  const mode = platform().mode;
  if (mode === 'web') return 'Running in a browser: only the board works here. Terminals, chat and files need the UniPi app.';
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

export function AboutSection() {
  const [info] = createResource(async () => call('app_info', {}, AppInfoSchema));
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
      <Row label="Version">
        <span class="text-sm text-text-muted" data-testid="about-version">{valueOf(info(), 'version')}</span>
      </Row>
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
        <Row label="What’s new" description="Release notes of the newest build" icon={<Sparkles size={18} />}>
          <Show
            when={releaseNotes(updateState())}
            fallback={<span class="text-sm text-text-muted" data-testid="about-whats-new">No release notes in the update feed yet</span>}
          >
            <p class="about-notes" data-testid="about-whats-new">{releaseNotes(updateState())}</p>
          </Show>
        </Row>
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
