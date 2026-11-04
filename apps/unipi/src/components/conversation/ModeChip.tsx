/**
 * Working-mode chip (WP-A14, Z-74; §3.10): the header's one mode control.
 *
 * It shows the long-horizon mode the session runs in — icon + rolling label +
 * a live count (`3/7`, `#4`, `45%`) and a pulse while the run goes — and its
 * menu *switches* only: Goal / Ralph / Graph starters, and "Plain chat"
 * (`/regular`) while a mode is up. The live state body lives in the status
 * pane's LH section (A16); this chip never repeats it.
 *
 * The modes start with an objective (`/goal <objective>`), so a starter puts
 * that command in the composer (`lib/status/modeStarter.ts`) instead of
 * sending anything; Ctrl/Cmd+Shift+M cycles the starters without the menu.
 * Hidden while `unipi.long-horizon` is off (no dead control).
 *
 * [graph-view] While graph / swarm mode runs a graph, the chip opens the graph
 * directly (the status pane's LH section, scrolled to it) and a chevron beside
 * it keeps the mode menu one click away.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { sessions } from '../../lib/session/store';
import { composerDraft, setComposerText } from '../../lib/composer/drafts';
import { focusComposer } from '../../lib/timeline/actions';
import { lhSummary, LH_PROGRESS_TOPIC, LH_STATE_TOPIC, parseLhProgress, parseLhState } from '../../lib/status/lh';
import { STARTER_HINT, STARTER_ORDER, withStarter, type ModeStarter } from '../../lib/status/modeStarter';
import { MODE_DESCRIPTION, type LhModeChoice } from '../status/lhMode';
import { useLhEnabled } from '../status/LhSection';
import { chipOpensGraph, openGraphView } from '../status/GraphView';
import { AnchoredMenu, MenuSeparator } from '../board/AnchoredMenu';
import { ModeIcon } from '../chat/ModeChip';
import { RollingLabel } from '../RollingLabel';
import { hubs } from '../../lib/session/hub';
import { liveActivity, liveBusy, type Activity } from '../../lib/session/activity';
import './conversation.css';

const PHASE_WORD = { active: 'Running', selected: 'On — the next message runs in it', paused: 'Paused' } as const;
/** What the chip says when the PC's agent host can't report the working mode. */
export const UNSUPPORTED_TEXT = "This PC's agent host is older than the app and can't report the working mode. Update Unipi on that PC.";

export function ModeChip(props: { hostId: string; sessionId: string; cwd?: string }) {
  const s = createMemo(() => sessions().ensure(props.hostId, props.sessionId));
  createEffect(
    on(s, (handle) => {
      const release = handle.acquire();
      handle.subscribeTopics([LH_STATE_TOPIC, LH_PROGRESS_TOPIC]);
      onCleanup(release);
    }),
  );
  const enabled = useLhEnabled(s);
  const summary = createMemo(() =>
    enabled()
      ? lhSummary(parseLhState(s().state.topics[LH_STATE_TOPIC]?.data), parseLhProgress(s().state.topics[LH_PROGRESS_TOPIC]?.data))
      : undefined,
  );
  const [open, setOpen] = createSignal(false);
  const [anchor, setAnchor] = createSignal<DOMRect | undefined>();
  /** [status] the session's activity: the dot pulses only while it really works (a run, or
   *  graph / swarm nodes and waking bg tasks it waits on); a dialog turns it to the accent. */
  const activity = createMemo<Activity>(() => {
    const card = hubs().peek(props.hostId)?.state.cards[props.sessionId];
    if (card) return liveActivity(card, s().state).activity;
    const st = s().state;
    if (st.dialogs.length > 0) return 'needs-you';
    return liveBusy(st.run) ? 'working' : st.run.backgroundOn?.length ? 'background' : 'idle';
  });
  const dot = () => (summary()?.phase === 'active' && (activity() === 'working' || activity() === 'needs-you') ? activity() : undefined);

  /** A starter rewrites the draft's leading command and keeps the words typed. */
  const start = (mode: ModeStarter) => {
    const key = s().key;
    setComposerText(key, withStarter(composerDraft(key).text, mode));
    setOpen(false);
    focusComposer(key);
  };
  const plain = () => {
    setOpen(false);
    void s().send('/regular', { mode: 'prompt' });
  };

  /** [graph-mode] an agent host older than the app can't report the mode (no `_oino/topics`):
   *  the chip says so instead of a silent "Plain" that a /graph switch never changes. */
  const unsupported = () => !summary() && s().state.topicsUnsupported === true;
  const label = () => summary()?.label ?? (unsupported() ? 'Mode ?' : 'Plain');
  const opensGraph = () => chipOpensGraph(summary());
  const menuAt = (el: HTMLElement) => {
    setAnchor(el.getBoundingClientRect());
    setOpen(!open());
  };
  const aria = () =>
    opensGraph()
      ? `Working mode: ${label()}${summary()?.countText ? ` ${summary()!.countText}` : ''}. Open the graph`
      : unsupported() ? `Working mode unknown: ${UNSUPPORTED_TEXT} Open the mode menu` : `Working mode: ${label()}${summary()?.phase === 'paused' ? ', paused' : dot() === 'needs-you' ? ', needs you' : dot() === 'working' ? ', working' : ''}. Open the mode menu`;

  return (
    <Show when={enabled() && !opensGraph()}>
      <button
        type="button"
        class="hdr-mode"
        data-mode={summary()?.mode ?? 'none'}
        data-phase={summary()?.phase ?? (unsupported() ? 'unsupported' : undefined)}
        data-activity={summary() ? activity() : undefined}
        aria-label={aria()}
        aria-expanded={opensGraph() ? undefined : open()}
        aria-haspopup={opensGraph() ? undefined : 'menu'}
        title={unsupported() ? UNSUPPORTED_TEXT : opensGraph() ? 'Open the graph — nodes, their models and progress' : 'Working mode — Goal / Ralph / Swarm / Graph (Ctrl Shift M)'}
        onClick={(e) => {
          if (opensGraph()) {
            setOpen(false);
            openGraphView({ hostId: props.hostId, sessionId: props.sessionId, path: props.cwd });
            return;
          }
          menuAt(e.currentTarget as HTMLElement);
        }}
        data-opens={opensGraph() ? 'graph' : undefined}
        data-testid="mode-chip"
      >
        <span class="hdr-mode-icon">
          <ModeIcon mode={summary()?.mode ?? 'none'} size={13} />
        </span>
        <RollingLabel class="hdr-mode-label" text={label()} testId="mode-chip-label" />
        <Show when={summary()?.phase === 'paused'}>
          <span class="hdr-mode-count">paused</span>
        </Show>
        <Show when={summary()?.phase !== 'paused' && summary()?.countText}>
          {(count) => <span class="hdr-mode-count">{count()}</span>}
        </Show>
        <Show when={dot()}>
          {(d) => <span class="hdr-mode-pulse" data-activity={d()} aria-hidden="true" />}
        </Show>
      </button>
      <Show when={open() && anchor()}>
        <AnchoredMenu anchor={anchor()!} alignEnd width={300} label="Working mode" onDismiss={() => setOpen(false)}>
          <div class="mode-switch-head" data-mode={summary()?.mode ?? 'none'}>
            <span class="mode-switch-icon">
              <ModeIcon mode={summary()?.mode ?? 'none'} size={16} />
            </span>
            <span class="min-w-0 flex-1">
              <span class="mode-switch-title">{summary() ? `${summary()!.label} mode` : 'Plain chat'}</span>
              <span class="mode-switch-phase" data-phase={summary()?.phase}>
                {summary() ? PHASE_WORD[summary()!.phase] : unsupported() ? 'Unknown — the agent host is too old to report it' : 'No long-horizon mode'}
              </span>
            </span>
          </div>
          <p class="mode-switch-desc" data-testid="mode-switch-desc">{unsupported() ? UNSUPPORTED_TEXT : (MODE_DESCRIPTION[(summary()?.mode as LhModeChoice) ?? 'none'] ?? '')}</p>
          <MenuSeparator />
          <div class="board-menu-label">Start</div>
          <For each={STARTER_ORDER}>
            {(mode) => (
              <button type="button" role="menuitem" class="mode-pick" data-mode={mode} onClick={() => start(mode)} data-testid={`mode-start-${mode}`}>
                <span class="mode-pick-icon">
                  <ModeIcon mode={mode} size={18} />
                </span>
                <span class="min-w-0 flex-1">
                  <span class="mode-pick-label">{mode[0]!.toUpperCase() + mode.slice(1)}</span>
                  <span class="mode-pick-hint">{STARTER_HINT[mode]}</span>
                </span>
              </button>
            )}
          </For>
          <Show when={summary()}>
            <MenuSeparator />
            <button type="button" role="menuitem" class="mode-pick" data-mode="none" onClick={plain} data-testid="mode-plain">
              <span class="mode-pick-icon">
                <ModeIcon mode="none" size={18} />
              </span>
              <span class="min-w-0 flex-1">
                <span class="mode-pick-label">Plain chat</span>
                <span class="mode-pick-hint">Stop the mode and run prompts plainly (/regular)</span>
              </span>
            </button>
          </Show>
        </AnchoredMenu>
      </Show>
    </Show>
  );
}
