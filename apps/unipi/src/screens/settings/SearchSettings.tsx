import { createResource, createSignal, Show } from 'solid-js';
import { Search } from 'lucide-solid';
import { z } from 'zod';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
import { settingsHostId, settingsProjectCwd } from '../../settings/target';
import { hostClient, requestParsed } from '../../lib/host';
import { Row, Section, Segmented } from './rows';

/** Uses the existing Settings primitives. Build is intentionally a host-only action. */
export default function SearchSettings() {
  const [settings, { refetch }] = createResource(settingsHostId, piSettings);
  const [capabilities] = createResource(settingsHostId, (hostId) =>
    requestParsed(hostClient(hostId), 'host_cmd', { cmd: { kind: 'search_capabilities' } },
      z.object({ semanticSearch: z.boolean() })),
  );
  const supported = () => {
    const result = capabilities();
    return result?.ok === true && result.value.semanticSearch;
  };
  const [saving, setSaving] = createSignal(false);
  const [consent, setConsent] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const [status, setStatus] = createSignal<string>();
  const enabled = () => {
    const result = settings();
    return result?.ok && (result.value.search as { semanticEnabled?: unknown } | undefined)?.semanticEnabled === true;
  };
  const save = async (value: string) => {
    setSaving(true); setError(undefined);
    try {
      const reply = await setPiSetting('search.semanticEnabled', value === 'on', 'global');
      if (!reply.ok) throw new Error(reply.error.message);
      await refetch();
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not save search settings'); }
    finally { setSaving(false); }
  };
  const build = async () => {
    const cwd = settingsProjectCwd();
    if (!supported() || !consent() || !cwd || saving()) return;
    setSaving(true); setError(undefined); setStatus('Building index…');
    try {
      const reply = await requestParsed(hostClient(settingsHostId()), 'host_cmd', {
        cmd: { kind: 'build_search_index', cwd, consent: true },
      }, z.object({ success: z.boolean() }));
      if (!reply.ok) throw new Error(reply.error.message);
      setStatus('Index ready. Semantic search is available in new enabled sessions.');
      setConsent(false);
    } catch (e) { setStatus(undefined); setError(e instanceof Error ? e.message : 'Could not build index'); }
    finally { setSaving(false); }
  };
  return <Section title="Search" id="sec-search" icon={<Search size={14} />}>
    <Show when={!settings.loading && !capabilities.loading} fallback={<p class="p-4 text-text-muted" role="status">Loading search settings…</p>}>
      <Show when={supported()} fallback={<p class="p-4 text-text-muted whitespace-normal">Exact search is available. This host does not include the optional semantic search engine.</p>}>
        <Row label="Semantic search (zvec-grep)" description="Default off. Changes apply to new sessions. Exact search always works without an index.">
          <fieldset disabled={saving() || settings()?.ok === false}>
            <Segmented label="Semantic search" options={['off', 'on'] as const} value={enabled() ? 'on' : 'off'} labels={{ off: 'Off', on: 'On' }} onChange={(v) => void save(v)} />
          </fieldset>
        </Row>
        <div class="p-4 space-y-3">
          <p class="text-sm text-text-dim whitespace-normal">Open Project settings for the folder to index. Building downloads the local CPU model (local/potion-code-16m-v2, about 31 MB) and creates .zvec-grep/ in that project.</p>
          <Show when={settingsProjectCwd()} fallback={<p class="text-sm text-text-muted">Choose a project using its Project settings first.</p>}>
            <label class="flex items-start gap-2 text-sm text-text whitespace-normal">
              <input type="checkbox" checked={consent()} disabled={saving()} onChange={(e) => setConsent(e.currentTarget.checked)} />
              I consent to download the model and index this project locally.
            </label>
          </Show>
          <button type="button" class="btn-secondary min-h-10" disabled={!consent() || !settingsProjectCwd() || saving()} onClick={() => void build()}>Build index</button>
          <Show when={status()}><p role="status" class="text-sm text-text-muted whitespace-normal">{status()}</p></Show>
        </div>
      </Show>
    </Show>
    <Show when={error() || settings()?.ok === false}><p role="alert" class="p-4 text-text-muted whitespace-normal">{error() ?? 'Could not load search settings. Reopen Settings to retry.'}</p></Show>
  </Section>;
}
