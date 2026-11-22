/**
 * Archived sessions (owner 2026-11-05): no longer an endless footer list in the sidebar.
 * A fixed "Archived" button above the host row opens this wide modal: search, a project
 * filter, sort, paging, and Restore / Open / Delete per row (plus Restore selected and
 * Delete all with a confirm). Same host ops as before (`sessions_list{archived}`,
 * `session_restore`, `session_delete`, `trash_empty`).
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Archive, ArchiveRestore, Search, Trash2 } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { hubs } from '../../lib/session/hub';
import { deleteArchived, emptyTrash, restoreSession, sessionHref } from '../../lib/session/actions';
import { sessionTitle } from '../../lib/session/selectors';
import { isGeneralWorkspaceCwd } from '../../lib/session/generalWorkspace';
import { fuzzyRank } from '../../lib/fuzzy';
import { shortAge } from '../../lib/time';
import { BoardSheet } from '../board/BoardSheet';
import { pageButtons, pageOf } from '../../lib/paging';
import './archivedModal.css';

const [openFor, setOpenFor] = createSignal<string | undefined>();
export function openArchivedModal(hostId: string): void {
  setOpenFor(hostId);
}

const GENERAL = '__general__';
const projectOf = (c: SessionCard) => isGeneralWorkspaceCwd(c.cwd) ? GENERAL : (c.cwd ?? '').replace(/[\\/]+$/, '').split(/[\\/]/).pop() || GENERAL;
const projectLabel = (key: string) => key === GENERAL ? 'General workspace' : key;
const projectOptions = (cards: readonly SessionCard[]) => {
  const newest = new Map<string, number>();
  for (const card of cards) {
    const key = projectOf(card);
    newest.set(key, Math.max(newest.get(key) ?? 0, card.last_activity ?? card.updated_at ?? 0));
  }
  return [...newest].sort(([, ta], [, tb]) => tb - ta).map(([key]) => key);
};
const titleOf = (c: SessionCard) => sessionTitle(c, undefined, c.session_id);

/** Search over title + folder, project filter, newest / oldest / name. Pure. */
export function archivedView(cards: readonly SessionCard[], q: string, project: string, sort: 'newest' | 'oldest' | 'name'): SessionCard[] {
  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const at = (c: SessionCard) => c.last_activity ?? c.updated_at ?? 0;
  return cards
    .filter((c) => (project === 'all' || projectOf(c) === project) && words.every((w) => `${titleOf(c)} ${projectLabel(projectOf(c))} ${c.cwd ?? ''}`.toLowerCase().includes(w)))
    .sort((a, b) => (sort === 'name' ? titleOf(a).localeCompare(titleOf(b)) : sort === 'oldest' ? at(a) - at(b) : at(b) - at(a)));
}

export function ArchivedModalLayer() {
  return <Show when={openFor()} keyed>{(hostId) => <ArchivedModal hostId={hostId} onClose={() => setOpenFor(undefined)} />}</Show>;
}

function ArchivedModal(props: { hostId: string; onClose: () => void }) {
  const navigate = useNavigate();
  const hub = () => hubs().ensure(props.hostId);
  void hub().loadArchived();
  const [q, setQ] = createSignal('');
  const [project, setProject] = createSignal('all');
  const [sort, setSort] = createSignal<'newest' | 'oldest' | 'name'>('newest');
  const [page, setPage] = createSignal(1);
  const [picked, setPicked] = createSignal<Set<string>>(new Set());
  const [confirm, setConfirm] = createSignal<string | undefined>();
  createEffect(on([q, project, sort], () => setPage(1)));
  const all = () => hub().state.archived;
  const projects = createMemo(() => projectOptions(all()));
  const [projectQuery, setProjectQuery] = createSignal('');
  const [projectOpen, setProjectOpen] = createSignal(false);
  const [projectIndex, setProjectIndex] = createSignal(0);
  let picker: HTMLDivElement | undefined;
  createEffect(() => {
    if (!projectOpen()) return;
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (picker?.contains(target)) return;
      setProjectOpen(false);
      // A tap on the scrim only dismisses the dropdown; the modal stays.
      // (the sheet dismisses on pointerup, so swallow that one too)
      if (!(target as Element).closest?.('.archived-modal-sheet')) {
        event.stopPropagation(); event.preventDefault();
        document.addEventListener('pointerup', (up) => up.stopPropagation(), { capture: true, once: true });
      }
    };
    const focusout = (event: FocusEvent) => { if (!picker?.contains(event.relatedTarget as Node | null)) setProjectOpen(false); };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('focusout', focusout, true);
    onCleanup(() => { document.removeEventListener('pointerdown', outside, true); document.removeEventListener('focusout', focusout, true); });
  });
  const projectHits = createMemo(() => {
    const query = projectQuery().trim();
    return query ? fuzzyRank(query, projects(), (key) => [projectLabel(key)]).map((hit) => hit.item) : projects();
  });
  const chooseProject = (key: string) => { setProject(key); setProjectQuery(''); setProjectOpen(false); };
  const shown = createMemo(() => archivedView(all(), q(), project(), sort()));
  const pg = () => pageOf(shown(), page(), 30);
  const toggle = (id: string) =>
    setPicked((cur) => {
      const n = new Set(cur);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const restorePicked = async () => {
    const ids = [...picked()];
    setPicked(new Set<string>());
    for (const id of ids) await restoreSession(props.hostId, id);
  };
  return (
    <BoardSheet
      open
      onClose={props.onClose}
      label="Archived sessions"
      class="archived-modal-sheet"
      testId="archived-modal"
      footer={
        <div class="arcm-foot">
          <span class="arcm-dim">{shown().length} of {all().length} archived</span>
          <span class="flex-1" />
          <Show when={picked().size > 0}>
            <button type="button" class="btn-primary settings-btn" onClick={() => void restorePicked()} data-testid="arcm-restore-picked">
              <ArchiveRestore size={13} /> Restore {picked().size}
            </button>
          </Show>
          <Show
            when={confirm() !== '*'}
            fallback={
              <button type="button" class="btn-secondary settings-btn text-wm-red" onClick={() => (setConfirm(undefined), void emptyTrash(props.hostId))} data-testid="arcm-empty-confirm">
                Delete all {all().length} for good?
              </button>
            }
          >
            <button type="button" class="btn-secondary settings-btn" disabled={!all().length} onClick={() => setConfirm('*')} data-testid="arcm-empty">
              <Trash2 size={13} /> Delete all
            </button>
          </Show>
        </div>
      }
    >
      <div class="arcm">
        <div class="arcm-bar">
          <label class="arcm-search">
            <Search size={14} />
            <input type="search" placeholder="Search archived sessions" value={q()} onInput={(e) => setQ(e.currentTarget.value)} data-testid="arcm-search" />
          </label>
          <div class="arcm-project-picker" data-testid="arcm-project-picker" ref={picker} onKeyDown={(e) => { if (e.key === 'Escape' && projectOpen()) { e.preventDefault(); e.stopPropagation(); setProjectQuery(''); setProjectOpen(false); } }}>
            <input class="arcm-project-input" role="combobox" aria-label="Project" aria-expanded={projectOpen()} aria-controls="arcm-project-options" placeholder="Search projects" value={projectQuery() || (project() === 'all' ? '' : projectLabel(project()))} onFocus={() => setProjectOpen(true)} onClick={() => setProjectOpen(true)} onInput={(e) => { setProjectQuery(e.currentTarget.value); setProjectOpen(true); setProjectIndex(0); }} onKeyDown={(e) => {
              if (e.key === 'ArrowDown') { e.preventDefault(); setProjectOpen(true); setProjectIndex((i) => Math.min(i + 1, projectHits().length)); }
              else if (e.key === 'ArrowUp') { e.preventDefault(); setProjectIndex((i) => Math.max(0, i - 1)); }
              else if (e.key === 'Enter' && projectOpen()) { e.preventDefault(); const key = projectHits()[Math.max(0, projectIndex() - 1)]; if (key) chooseProject(key); else if (projectIndex() === 0) chooseProject('all'); }
              else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setProjectQuery(''); setProjectOpen(false); }
            }} data-testid="arcm-project" />
            <Show when={projectOpen()}><div class="arcm-project-options" id="arcm-project-options" role="listbox" data-testid="arcm-project-options">
              <Show when={!projectQuery().trim() || fuzzyRank(projectQuery(), ['All projects'], (label) => [label]).length}><button type="button" role="option" aria-selected={project() === 'all'} class="arcm-project-option" classList={{ 'is-active': projectIndex() === 0 }} onMouseEnter={() => setProjectIndex(0)} onClick={() => chooseProject('all')}>All projects</button></Show>
              <For each={projectHits()}>{(key, i) => { const allMatches = !projectQuery().trim() || !!fuzzyRank(projectQuery(), ['All projects'], (label) => [label]).length; const index = i() + (allMatches ? 1 : 0); return <button type="button" role="option" aria-selected={project() === key} class="arcm-project-option" classList={{ 'is-active': projectIndex() === index }} onMouseEnter={() => setProjectIndex(index)} onClick={() => chooseProject(key)}>{projectLabel(key)}</button>; }}</For>
              <Show when={projectQuery().trim() && !projectHits().length}><div class="arcm-dim arcm-project-empty">No projects match.</div></Show>
            </div></Show>
          </div>
          <select class="arcm-select" aria-label="Sort" value={sort()} onChange={(e) => setSort(e.currentTarget.value as 'newest')}>
            <option value="newest">Newest first</option>
            <option value="oldest">Oldest first</option>
            <option value="name">Name</option>
          </select>
        </div>
        <Show when={hub().state.archivedLoaded} fallback={<div class="arcm-skel" aria-busy="true"><For each={[1, 2, 3, 4, 5]}>{() => <span />}</For></div>}>
          <Show when={shown().length} fallback={<p class="arcm-dim arcm-empty">{all().length ? 'Nothing matches.' : 'Nothing archived.'}</p>}>
            <ul class="arcm-list" data-testid="arcm-list">
              <For each={pg().rows}>
                {(c) => (
                  <li class="arcm-row" data-testid={`arcm-row-${c.session_id}`}>
                    <input type="checkbox" aria-label={`Select ${titleOf(c)}`} checked={picked().has(c.session_id)} onChange={() => toggle(c.session_id)} />
                    <button type="button" class="arcm-open" title={titleOf(c)} onClick={() => (props.onClose(), navigate(sessionHref(props.hostId, c.session_id)))}>
                      <span class="arcm-title">{titleOf(c)}</span>
                      <span class="arcm-dim">{projectLabel(projectOf(c))} · {shortAge(c.last_activity ?? c.updated_at)}</span>
                    </button>
                    <Show
                      when={confirm() !== c.session_id}
                      fallback={
                        <>
                          <button type="button" class="settings-link" onClick={() => setConfirm(undefined)}>Cancel</button>
                          <button type="button" class="settings-link text-wm-red" onClick={() => (setConfirm(undefined), void deleteArchived(props.hostId, c.session_id))}>Delete</button>
                        </>
                      }
                    >
                      <button type="button" class="btn-secondary settings-btn arcm-restore" onClick={() => void restoreSession(props.hostId, c.session_id)} data-testid={`arcm-restore-${c.session_id}`}>
                        <ArchiveRestore size={13} /> Restore
                      </button>
                      <button type="button" class="ghost-icon-button" aria-label="Delete for good" title="Delete for good" onClick={() => setConfirm(c.session_id)}>
                        <Trash2 size={14} />
                      </button>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
            <Show when={pg().pages > 1}>
              <nav class="cron-pager" aria-label="Archived pages">
                <button type="button" class="cron-page-btn" disabled={pg().page <= 1} onClick={() => setPage(pg().page - 1)}>‹</button>
                <For each={pageButtons(pg().page, pg().pages)}>
                  {(b) => (b === '…' ? <span class="cron-page-gap">…</span> : <button type="button" class="cron-page-btn" aria-current={b === pg().page ? 'page' : undefined} onClick={() => setPage(b)}>{b}</button>)}
                </For>
                <button type="button" class="cron-page-btn" disabled={pg().page >= pg().pages} onClick={() => setPage(pg().page + 1)}>›</button>
              </nav>
            </Show>
          </Show>
        </Show>
      </div>
    </BoardSheet>
  );
}

/** The fixed sidebar button above the host row. */
export function ArchivedButton(props: { hostId: string | undefined }) {
  const count = () => {
    const id = props.hostId;
    if (!id) return 0;
    const st = hubs().ensure(id).state;
    return st.archivedLoaded ? st.archived.length : undefined;
  };
  return (
    <Show when={props.hostId}>
      {(id) => (
        <button type="button" class="sb-archived-btn" onClick={() => openArchivedModal(id())} data-testid="sb-archived-open" title="Archived sessions — search and restore">
          <Archive size={15} />
          <span>Archived</span>
          <Show when={count()}>{(n) => <span class="sb-host-count">{n()}</span>}</Show>
        </button>
      )}
    </Show>
  );
}
