/**
 * Skills manager (owner 2026-11-05): the one native surface for per-skill Enabled /
 * Must show, replacing the redundant "Skills hub" (a copy of the settings form) and the
 * cramped "Skill editor" overlay. A wide modal on the desktop, a full sheet on the phone:
 * scope tabs (Global · This project), search, source filter, one row per skill with both
 * switches, and — in the project scope — "inherit" to fall back to Global.
 *
 * Data: `unipi.skill-registry`'s `skills.catalog` / `skills.save` services, called in
 * Settings' own command session rooted at the project folder (so the project scope is
 * that project's `.oino` layer).
 */
import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import { Search, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { SwitchField } from '../Switch';
import { settingsCommandChat } from '../../lib/settings/run';
import { toasts } from '../../lib/toasts';
import './skills.css';

export type SkillRow = { name: string; description: string; source: string; defaultEnabled: boolean; defaultMustShow: boolean; path?: string };
export type SkillState = { enabled?: boolean; must_show?: boolean };
export type SkillCatalog = { skills: SkillRow[]; global: Record<string, SkillState>; project: Record<string, SkillState>; proxy: boolean; cwd?: string };
export type SkillScope = 'global' | 'project';

const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

/** The service reply, leniently. Pure. */
export function parseCatalog(raw: unknown): SkillCatalog {
  const o = obj(raw);
  const states = (v: unknown): Record<string, SkillState> => {
    const out: Record<string, SkillState> = {};
    for (const [k, s] of Object.entries(obj(v))) {
      const r = obj(s);
      out[k] = { ...(typeof r.enabled === 'boolean' ? { enabled: r.enabled } : {}), ...(typeof r.must_show === 'boolean' ? { must_show: r.must_show } : {}) };
    }
    return out;
  };
  const skills = (Array.isArray(o.skills) ? o.skills : []).map(obj).filter((s) => typeof s.name === 'string' && s.name).map((s) => ({
    name: String(s.name),
    description: typeof s.description === 'string' ? s.description : '',
    source: typeof s.source === 'string' ? s.source : 'package',
    defaultEnabled: s.defaultEnabled !== false,
    defaultMustShow: s.defaultMustShow === true,
    ...(typeof s.path === 'string' ? { path: s.path } : {}),
  }));
  return { skills, global: states(o.global), project: states(o.project), proxy: o.proxy !== false, ...(typeof o.cwd === 'string' ? { cwd: o.cwd } : {}) };
}

/** The effective state of a skill in `scope` (project over global over the defaults). Pure. */
export function effective(row: SkillRow, scope: SkillScope, global: Record<string, SkillState>, project: Record<string, SkillState>): { enabled: boolean; mustShow: boolean; from: 'project' | 'global' | 'default' } {
  const g = global[row.name] ?? {};
  const p = scope === 'project' ? project[row.name] ?? {} : {};
  const mustShow = p.must_show ?? g.must_show ?? row.defaultMustShow;
  const enabled = mustShow || (p.enabled ?? g.enabled ?? row.defaultEnabled);
  const from = scope === 'project' && (p.enabled !== undefined || p.must_show !== undefined) ? 'project' : g.enabled !== undefined || g.must_show !== undefined ? 'global' : 'default';
  return { enabled, mustShow, from };
}

/** Writes one field into a scope's states, dropping a value equal to what it would inherit. Pure. */
export function setState(states: Record<string, SkillState>, name: string, key: 'enabled' | 'must_show', value: boolean | undefined): Record<string, SkillState> {
  const next = { ...states };
  const cur = { ...(next[name] ?? {}) };
  if (value === undefined) delete cur[key];
  else cur[key] = value;
  if (cur.enabled === undefined && cur.must_show === undefined) delete next[name];
  else next[name] = cur;
  return next;
}

async function service<T>(hostId: string, cwd: string | undefined, name: string, payload?: Record<string, unknown>): Promise<T> {
  const handle = await settingsCommandChat(hostId, undefined, cwd);
  if (!handle) throw new Error('Couldn’t reach the agent on this PC — try again in a moment.');
  return handle.cmd<T>({ type: '_oino/service_call', name, ...(payload ? { payload } : {}) });
}

export function SkillsManager(props: { hostId: string; cwd?: string; projectName?: string; initialScope?: SkillScope; onClose: () => void }) {
  const [scope, setScope] = createSignal<SkillScope>(props.initialScope ?? (props.cwd ? 'project' : 'global'));
  const [query, setQuery] = createSignal('');
  const [source, setSource] = createSignal<string>('all');
  const [catalog, { refetch }] = createResource(async () => parseCatalog(await service<unknown>(props.hostId, props.cwd, 'skills.catalog')));
  const [global, setGlobal] = createSignal<Record<string, SkillState> | undefined>();
  const [project, setProject] = createSignal<Record<string, SkillState> | undefined>();
  const [busy, setBusy] = createSignal(false);
  const g = () => global() ?? catalog.latest?.global ?? {};
  const p = () => project() ?? catalog.latest?.project ?? {};
  const dirty = () => global() !== undefined || project() !== undefined;
  const sources = createMemo(() => [...new Set((catalog.latest?.skills ?? []).map((s) => s.source))].sort());
  const rows = createMemo(() => {
    const q = query().trim().toLowerCase();
    return (catalog.latest?.skills ?? []).filter((s) => (source() === 'all' || s.source === source()) && (!q || `${s.name} ${s.description}`.toLowerCase().includes(q)));
  });
  const counts = createMemo(() => {
    let on = 0;
    let pinned = 0;
    for (const s of catalog.latest?.skills ?? []) {
      const e = effective(s, scope(), g(), p());
      if (e.enabled) on++;
      if (e.mustShow) pinned++;
    }
    return { on, pinned, total: catalog.latest?.skills.length ?? 0 };
  });
  const write = (name: string, key: 'enabled' | 'must_show', value: boolean | undefined) => {
    if (scope() === 'project') setProject(setState(p(), name, key, value));
    else setGlobal(setState(g(), name, key, value));
  };
  const save = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      if (global() !== undefined) await service(props.hostId, props.cwd, 'skills.save', { scope: 'global', states: global() });
      if (project() !== undefined) await service(props.hostId, props.cwd, 'skills.save', { scope: 'project', states: project() });
      setGlobal(undefined);
      setProject(undefined);
      await refetch();
      toasts().push('Skill settings saved — they apply from the next prompt', 'success');
    } catch (e) {
      toasts().push(String((e as { message?: unknown })?.message ?? e), 'error');
    } finally {
      setBusy(false);
    }
  };
  return (
    <BoardSheet
      open
      onClose={props.onClose}
      label="Manage skills"
      class="skills-manager-sheet"
      testId="skills-manager"
      footer={
        <div class="skm-foot">
          <span class="skm-count">{counts().on} of {counts().total} on · {counts().pinned} must show</span>
          <span class="flex-1" />
          <button type="button" class="btn-secondary settings-btn" onClick={props.onClose}>{dirty() ? 'Discard' : 'Close'}</button>
          <button type="button" class="btn-primary settings-btn" disabled={!dirty() || busy()} onClick={() => void save()} data-testid="skills-manager-save">
            {busy() ? 'Saving…' : 'Save'}
          </button>
        </div>
      }
    >
      <div class="skm" data-testid="skills-manager-body">
        <div class="skm-bar">
          <div class="settings-scope" role="radiogroup" aria-label="Scope">
            <button type="button" role="radio" class="settings-scope-option" aria-checked={scope() === 'global'} onClick={() => setScope('global')} data-testid="skm-scope-global">Global</button>
            <Show when={props.cwd}>
              <button type="button" role="radio" class="settings-scope-option" aria-checked={scope() === 'project'} onClick={() => setScope('project')} data-testid="skm-scope-project" title={props.cwd}>
                {props.projectName ?? 'This project'}
              </button>
            </Show>
          </div>
          <label class="skm-search">
            <Search size={14} />
            <input type="search" placeholder="Search skills" value={query()} onInput={(e) => setQuery(e.currentTarget.value)} data-testid="skm-search" />
            <Show when={query()}>
              <button type="button" aria-label="Clear" onClick={() => setQuery('')}><X size={13} /></button>
            </Show>
          </label>
          <select class="skm-source" aria-label="Source" value={source()} onChange={(e) => setSource(e.currentTarget.value)} data-testid="skm-source">
            <option value="all">All sources</option>
            <For each={sources()}>{(s) => <option value={s}>{s}</option>}</For>
          </select>
        </div>
        <Show when={!catalog.error} fallback={<p class="skm-note" role="alert">{String((catalog.error as { message?: unknown })?.message ?? 'Couldn’t load the skills.')}</p>}>
          <Show when={catalog.latest} fallback={<div class="skm-skel" aria-busy="true"><For each={[1, 2, 3, 4, 5, 6]}>{() => <span />}</For></div>}>
            <div class="skm-head" aria-hidden="true">
              <span>Skill</span>
              <span>Enabled</span>
              <span>Must show</span>
              <span />
            </div>
            <ul class="skm-list" data-testid="skm-list">
              <For each={rows()} fallback={<li class="skm-note">No skill matches.</li>}>
                {(row) => {
                  const e = () => effective(row, scope(), g(), p());
                  const own = () => (scope() === 'project' ? p()[row.name] : g()[row.name]);
                  return (
                    <li class="skm-row" data-testid={`skm-row-${row.name}`} data-from={e().from}>
                      <div class="skm-main" title={row.path}>
                        <span class="skm-name">{row.name}</span>
                        <span class="skm-src">{row.source}</span>
                        <Show when={row.description}>
                          <span class="skm-desc">{row.description}</span>
                        </Show>
                      </div>
                      <SwitchField label={`${row.name} enabled`} checked={e().enabled} disabled={e().mustShow} onChange={(v) => write(row.name, 'enabled', v)} testId={`skm-en-${row.name}`} />
                      <SwitchField label={`${row.name} must show`} checked={e().mustShow} onChange={(v) => write(row.name, 'must_show', v)} testId={`skm-ms-${row.name}`} />
                      <span class="skm-inherit">
                        <Show when={own()} fallback={<span class="skm-from">{e().from === 'default' ? 'default' : scope() === 'project' ? 'global' : ''}</span>}>
                          <button type="button" class="settings-link" onClick={() => (write(row.name, 'enabled', undefined), write(row.name, 'must_show', undefined))} title={scope() === 'project' ? 'Use the global setting' : 'Back to the default'}>
                            {scope() === 'project' ? 'inherit' : 'reset'}
                          </button>
                        </Show>
                      </span>
                    </li>
                  );
                }}
              </For>
            </ul>
          </Show>
        </Show>
      </div>
    </BoardSheet>
  );
}
