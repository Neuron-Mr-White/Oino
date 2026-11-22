/**
 * Filter and Display sheets (UNI-130) — the web UI's FilterMenu and
 * DisplayMenu popovers as bottom sheets: every option the web has
 * (priorities, Running now, Waiting on dependencies, labels; columns,
 * Group linked tasks, Show description excerpt, Compact cards), as big
 * toggle chips / switch rows, with the live result count on the button.
 */
import { For, Show, type JSX } from 'solid-js';
import { Switch as KSwitch } from '@kobalte/core/switch';
import { Columns3, Link2, List, Lock, Rows3, Sparkles, TextQuote } from 'lucide-solid';
import { LANES, PRIORITIES, PRIORITY_LABEL } from '../../lib/board/api';
import { allLabels, filterCount, hue, matches, visibleLanes } from '../../lib/board/view';
import { boardStore } from '../../lib/board/store';
import { boardUi } from '../../lib/board/ui';
import { useSettings } from '../../settings/context';
import { BoardSheet } from './BoardSheet';
import { PriorityGlyph, StatusGlyph } from './Glyphs';

export function BoardSwitch(props: { checked: boolean; onChange: (value: boolean) => void; label: string }) {
  return (
    <KSwitch checked={props.checked} onChange={props.onChange} class="inline-flex items-center" aria-label={props.label}>
      <KSwitch.Input />
      <KSwitch.Control class="relative flex h-[26px] w-11 shrink-0 items-center rounded-pill bg-surface-3 transition-colors data-[checked]:bg-shell">
        <KSwitch.Thumb class="block h-5 w-5 shrink-0 rounded-pill bg-text translate-x-[3px] transition-transform data-[checked]:translate-x-[21px]" />
      </KSwitch.Control>
    </KSwitch>
  );
}

function SwitchRow(props: { icon: JSX.Element; label: string; description?: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <div class="settings-row board-switch-row">
      <span class="settings-icon">{props.icon}</span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">{props.label}</p>
        <Show when={props.description}>
          <p class="text-xs text-text-dim mt-0.5">{props.description}</p>
        </Show>
      </div>
      <div class="shrink-0">
        <BoardSwitch label={props.label} checked={props.checked} onChange={props.onChange} />
      </div>
    </div>
  );
}

function SectionLabel(props: { children: JSX.Element }) {
  return <h3 class="board-section-label">{props.children}</h3>;
}

export function FilterSheet(props: { open: boolean; onClose: () => void }) {
  const store = boardStore();
  const ui = boardUi();
  const toggle = (key: 'priorities' | 'labels', value: string) =>
    ui.setFilters(key, (current) => (current.includes(value) ? current.filter((item) => item !== value) : [...current, value]));
  const shown = () => store.state.tasks.filter((task) => matches(task, ui.query(), ui.filters)).length;
  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Filter"
      testId="filter-sheet"
      headerAction={
        <button
          type="button"
          class="board-text-button"
          disabled={filterCount(ui.filters) === 0}
          onClick={() => ui.setFilters({ priorities: [], labels: [], running: false, waiting: false })}
        >
          Clear
        </button>
      }
      footer={
        <button type="button" class="pair-primary" onClick={props.onClose}>
          Show {shown()} {shown() === 1 ? 'task' : 'tasks'}
        </button>
      }
    >
      <SectionLabel>Priority</SectionLabel>
      <div class="chip-grid">
        <For each={[...PRIORITIES].reverse()}>
          {(priority) => (
            <button
              type="button"
              class="toggle-chip"
              aria-pressed={ui.filters.priorities.includes(priority)}
              onClick={() => toggle('priorities', priority)}
            >
              <PriorityGlyph priority={priority} />
              {PRIORITY_LABEL[priority]}
            </button>
          )}
        </For>
      </div>
      <SectionLabel>Agent</SectionLabel>
      <div class="settings-group rounded-card bg-surface-2/60">
        <SwitchRow icon={<Sparkles size={18} />} label="Running now" checked={ui.filters.running} onChange={(v) => ui.setFilters('running', v)} />
        <SwitchRow
          icon={<Lock size={18} />}
          label="Waiting on dependencies"
          checked={ui.filters.waiting}
          onChange={(v) => ui.setFilters('waiting', v)}
        />
      </div>
      <Show when={allLabels(store.state.tasks).length > 0}>
        <SectionLabel>Labels</SectionLabel>
        <div class="chip-grid">
          <For each={allLabels(store.state.tasks)}>
            {(label) => (
              <button type="button" class="toggle-chip" aria-pressed={ui.filters.labels.includes(label)} onClick={() => toggle('labels', label)}>
                <span class="label-dot" style={{ background: hue(label) }} />
                {label}
              </button>
            )}
          </For>
        </div>
      </Show>
    </BoardSheet>
  );
}

export function DisplaySheet(props: { open: boolean; onClose: () => void }) {
  const ui = boardUi();
  const [settings, update] = useSettings();
  const laneOn = (lane: string) => (lane === 'backlog' ? settings.board.showBacklog : !ui.display.hidden.includes(lane));
  const toggleLaneVisible = (lane: string) => {
    if (lane === 'backlog') update('board.showBacklog', !settings.board.showBacklog);
    else ui.toggleLane(lane);
  };
  const noneVisible = () => visibleLanes(ui.display, settings.board.showBacklog).length === 0;
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Display" testId="display-sheet">
      <SectionLabel>View</SectionLabel>
      <div class="view-switch" role="radiogroup" aria-label="View" data-active={ui.view()}>
        <span class="mux-switch-thumb" aria-hidden="true" />
        <button type="button" role="radio" aria-checked={ui.view() === 'board'} class="mux-switch-option" onClick={() => ui.setView('board')}>
          <Columns3 size={16} /> Board
        </button>
        <button type="button" role="radio" aria-checked={ui.view() === 'list'} class="mux-switch-option" onClick={() => ui.setView('list')}>
          <List size={16} /> List
        </button>
      </div>
      <SectionLabel>Columns</SectionLabel>
      <div class="chip-grid">
        <For each={LANES}>
          {(lane) => (
            <button type="button" class="toggle-chip" aria-pressed={laneOn(lane.id)} onClick={() => toggleLaneVisible(lane.id)}>
              <StatusGlyph status={lane.id} size={14} />
              {lane.label}
            </button>
          )}
        </For>
      </div>
      <Show when={noneVisible()}>
        <p class="text-xs text-amber mt-2">Every column is hidden — turn one back on to see tasks.</p>
      </Show>
      <SectionLabel>Cards</SectionLabel>
      <div class="settings-group rounded-card bg-surface-2/60">
        <SwitchRow
          icon={<Link2 size={18} />}
          label="Group linked tasks"
          description="Same-lane dependency chains stay together"
          checked={ui.display.chains}
          onChange={(v) => ui.setDisplay('chains', v)}
        />
        <SwitchRow icon={<TextQuote size={18} />} label="Show description excerpt" checked={ui.display.excerpt} onChange={(v) => ui.setDisplay('excerpt', v)} />
        <SwitchRow
          icon={<Rows3 size={18} />}
          label="Compact cards"
          checked={ui.display.density === 'compact'}
          onChange={(v) => ui.setDisplay('density', v ? 'compact' : 'comfortable')}
        />
      </div>
    </BoardSheet>
  );
}
