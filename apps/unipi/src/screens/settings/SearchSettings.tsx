import { createResource, createSignal, onCleanup, Show } from 'solid-js';
import { Search } from 'lucide-solid';
import { z } from 'zod';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
import { settingsHostId, settingsProjectCwd } from '../../settings/target';
import { hostClient, requestParsed } from '../../lib/host';
import { Row, Section, Segmented } from './rows';
import './searchSettings.css';

/** The Settings small secondary button; disabled reads as disabled (no global rule exists). */
const BTN = 'btn-secondary settings-btn inline-flex items-center justify-center shrink-0 disabled:opacity-50 disabled:cursor-not-allowed';

/** Uses the existing Settings primitives. Build is intentionally a host-only action. */
export default function SearchSettings() {
  const [settings, { refetch }] = createResource(settingsHostId, piSettings);
  const [capabilities, { refetch: refetchCapabilities }] = createResource(settingsHostId, (hostId) =>
    requestParsed(hostClient(hostId), 'host_cmd', { cmd: { kind: 'search_capabilities' } },
      z.object({ semanticSearch: z.boolean(), installed: z.boolean().optional(), available: z.boolean().optional(), size: z.number().nullable().optional(), downloaded: z.number().optional(), total: z.number().optional() })),
  );
  const supported = () => {
    const result = capabilities();
    return result?.ok === true && result.value.semanticSearch;
  };
  const [saving, setSaving] = createSignal(false);
  const [installing, setInstalling] = createSignal(false);
  const [installConsent, setInstallConsent] = createSignal(false);
  const available = () => { const result = capabilities(); return result?.ok === true && result.value.available === true; };
  const size = () => { const result = capabilities(); return result?.ok ? result.value.size : undefined; };
  const [progress, setProgress] = createSignal({ downloaded: 0, total: 0 });
  let timer: ReturnType<typeof setInterval> | undefined;
  onCleanup(() => { if (timer) clearInterval(timer); });
  const action = async (kind: 'install_search' | 'uninstall_search' | 'cancel_search_install') => {
    if (kind === 'install_search' && !installConsent()) return;
    if (kind === 'install_search') {
      setInstalling(true);
      timer = setInterval(() => { void requestParsed(hostClient(settingsHostId()), 'host_cmd', { cmd: { kind: 'search_capabilities' } }, z.object({ downloaded: z.number(), total: z.number() })).then((r) => { if (r.ok) setProgress(r.value); }); }, 500);
    }
    setError(undefined);
    try {
      const reply = await requestParsed(hostClient(settingsHostId()), 'host_cmd', {
        cmd: kind === 'install_search' ? { kind, consent: true } : { kind },
      }, z.object({ success: z.boolean() }));
      if (!reply.ok) throw new Error(reply.error.message);
      if (kind !== 'cancel_search_install') await refetchCapabilities();
    } catch (e) { setError(e instanceof Error ? e.message : 'Search action failed'); }
    finally { if (kind === 'install_search') { if (timer) clearInterval(timer); timer = undefined; setInstalling(false); } }
  };
  const [consent, setConsent] = createSignal(false);
  const [error, setError] = createSignal<string>();
  const [status, setStatus] = createSignal<string>();
  const [indexStatus, setIndexStatus] = createSignal<{ state: string; files?: number | null; updated?: number | null }>();
  const refreshIndex = async () => {
    const cwd = settingsProjectCwd();
    if (!cwd || !supported()) return;
    const result = await requestParsed(hostClient(settingsHostId()), 'host_cmd', { cmd: { kind: 'search_index_status', cwd } }, z.object({ state: z.string(), files: z.number().nullable().optional(), updated: z.number().nullable().optional() }));
    if (result.ok) setIndexStatus(result.value);
  };
  const indexTimer = setInterval(() => void refreshIndex(), 3000);
  onCleanup(() => clearInterval(indexTimer));
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
      await refreshIndex();
    } catch (e) { setStatus(undefined); setError(e instanceof Error ? e.message : 'Could not build index'); }
    finally { setSaving(false); }
  };
  return <Section title="Search" id="sec-search" icon={<Search size={14} />}>
    <Show when={!settings.loading && !capabilities.loading} fallback={<p class="p-4 text-text-muted" role="status">Loading search settings…</p>}>
      <Show when={supported()} fallback={<><Row label="Semantic search" description={available() ? "Download the local semantic search helper." : "Semantic search is not available for this build."} /><Show when={available()}><div class="search-settings-details">
        <p>Download size: {size() ? `${(size()! / 1_000_000).toFixed(1)} MB` : 'unknown'}</p>
        <label class="search-settings-consent"><input type="checkbox" checked={installConsent()} disabled={installing()} onChange={(e) => setInstallConsent(e.currentTarget.checked)} /><span>I consent to download and install semantic search.</span></label>
        <Show when={installing()} fallback={<button type="button" class={BTN} disabled={!installConsent()} onClick={() => void action('install_search')}>Install semantic search</button>}>
          <p role="status">Installing semantic search… {progress().total ? `${Math.min(100, Math.floor(progress().downloaded * 100 / progress().total))}% (${(progress().downloaded / 1_000_000).toFixed(1)} / ${(progress().total / 1_000_000).toFixed(1)} MB)` : ''}</p><button type="button" class={BTN} onClick={() => void action('cancel_search_install')}>Cancel</button>
        </Show>
      </div></Show></>}>
        <Row label="Semantic search (zvec-grep)" description="Default off. Changes apply to new sessions. Exact search always works without an index.">
          <fieldset disabled={saving() || settings()?.ok === false}>
            <Segmented label="Semantic search" options={['off', 'on'] as const} value={enabled() ? 'on' : 'off'} labels={{ off: 'Off', on: 'On' }} onChange={(v) => void save(v)} />
          </fieldset>
        </Row>
        <div class="search-settings-details">
          <Show when={indexStatus()}>{(index) => <p class="text-sm text-text-muted">Index: {index().state}{index().files != null ? ` · ${index().files} files` : ''}{index().updated ? ` · updated ${new Date(index().updated!).toLocaleString()}` : ''}</p>}</Show>
          <p class="text-sm text-text-dim whitespace-normal">Open Project settings for the folder to index. Building downloads the local CPU model (local/potion-code-16m-v2, about 31 MB) and creates .zvec-grep/ in that project.</p>
          <Show when={settingsProjectCwd()} fallback={<p class="text-sm text-text-muted whitespace-normal">To index a project, right-click it in the sidebar (or use its … menu) and choose <strong>Build search index…</strong>. Indexed projects show an orange folder.</p>}>
            <label class="search-settings-consent">
              <input type="checkbox" checked={consent()} disabled={saving()} onChange={(e) => setConsent(e.currentTarget.checked)} />
              <span>I consent to download the model and index this project locally.</span>
            </label>
          </Show>
          <div class="flex flex-wrap items-center gap-2" data-testid="search-actions">
            <button type="button" class={BTN} disabled={!consent() || !settingsProjectCwd() || saving()} onClick={() => void build()}>Build index</button>
            <button type="button" class={BTN} disabled={saving()} onClick={() => void action('uninstall_search')}>Uninstall</button>
          </div>
          <Show when={status()}><p role="status" class="text-sm text-text-muted whitespace-normal">{status()}</p></Show>
        </div>
      </Show>
    </Show>
    <Show when={error() || settings()?.ok === false}><p role="alert" class="p-4 text-text-muted whitespace-normal">{error() ?? 'Could not load search settings. Reopen Settings to retry.'}</p></Show>
  </Section>;
}
