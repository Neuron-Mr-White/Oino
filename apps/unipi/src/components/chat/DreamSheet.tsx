/**
 * UNI-118: the chat's Dream sheet (header ⋯ → Dream, palette "Show Dream").
 * Dream is pi's background memory consolidation: a child pi reviews past
 * sessions, applies memory lessons and proposes skills/checks for approval.
 *
 *   list    status card (off / on · due · last run, "Run now", turn-on hint
 *           when off), then the runs (running first), each a row with state,
 *           start, sessions, waiting proposals and duration.
 *   detail  one run: live trajectory while it runs (pulled every 2 s), the
 *           report as markdown, proposals with Approve / Reject (confirm),
 *           Stop (confirm) and Dismiss.
 *
 * Shared pieces: BoardSheet, EmptyState, .section-label, .btn-*.
 * docs/m5/PROTOCOL.md §9.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ChevronLeft, ChevronRight, Moon, Play, Square } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { EmptyState } from '../EmptyState';
import { MdBlock } from './MdBlock';
import { renderBlocks } from '../../lib/markdown';
import type { Chat } from '../../lib/chat/store';
import { DECISION_LABEL, durationText, pendingCount, RUN_LABEL, runFacts, startedText, statusLine, type DreamDetail, type DreamProposal, type DreamRun } from '../../lib/chat/dream';

/** Is there something worth a hint on the menu row? (a running dream or proposals waiting) */
export function dreamHint(chat: Chat | undefined): string | undefined {
  const d = chat?.state.dream;
  if (!d) return undefined;
  if (d.running) return 'running';
  const n = pendingCount(d);
  return n > 0 ? String(n) : undefined;
}

type Confirm = { title: string; body: string; action: string; danger?: boolean; run: () => Promise<void> };

export function DreamSheet(props: { open: boolean; onClose: () => void; chat: Chat }) {
  const s = () => props.chat.state;
  const [viewId, setViewId] = createSignal<string | null>(null);
  const [confirm, setConfirm] = createSignal<Confirm | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [flash, setFlash] = createSignal<{ text: string; tone?: 'error' } | null>(null);
  const [now, setNow] = createSignal(Date.now());
  // Pull once on open (an older hello may predate a run); pushes keep it live.
  createEffect(on(() => props.open && s().conn === 'live', (go) => go && props.chat.refreshDream?.()));
  createEffect(() => {
    if (!props.open) {
      setViewId(null);
      setConfirm(null);
      setFlash(null);
      return;
    }
    const t = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(t));
  });
  const status = () => s().dream;
  const viewed = createMemo(() => status()?.runs.find((r) => r.id === viewId()));

  const act = async (label: string, fn: () => Promise<void>, ok: string) => {
    setBusy(true);
    setFlash(null);
    try {
      await fn();
      setFlash({ text: ok });
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      // pi's reason often already says what failed ("Approval failed (not recorded): …").
      setFlash({ text: msg.toLowerCase().startsWith(label.toLowerCase()) ? msg : `${label}: ${msg}`, tone: 'error' });
    } finally {
      setBusy(false);
    }
  };
  const runNow = () => void act("Couldn't start a dream", () => props.chat.dreamAction('run'), 'Dream started in the background.');

  const title = () => (viewed() ? `Dream · ${startedText(viewed()!.startedAt, now())}` : 'Dream');
  return (
    <>
      <BoardSheet
        open={props.open && confirm() === null}
        onClose={props.onClose}
        label="Dream"
        title={
          <span class="dream-title">
            <Show when={viewed()}>
              <button type="button" class="dream-back" aria-label="Back to all dreams" onClick={() => (setViewId(null), setFlash(null))}>
                <ChevronLeft size={20} />
              </button>
            </Show>
            {title()}
          </span>
        }
        class="dream-sheet"
        testId="dream-sheet"
      >
        <Show when={flash()}>
          {(f) => (
            <p class="dream-flash" data-tone={f().tone} role={f().tone === 'error' ? 'alert' : 'status'} data-testid="dream-flash">
              {f().text}
            </p>
          )}
        </Show>
        <Show
          when={status()}
          fallback={
            <EmptyState
              compact
              testId="dream-empty"
              icon={<Moon size={22} />}
              title={s().dreamSupported === false ? 'Dream needs a newer unipi' : 'Dream isn\u2019t available'}
              body={
                s().dreamSupported === false
                  ? 'Update unipi on the PC and restart pi to watch and approve dreams here.'
                  : 'This pi has no Dream module loaded. Update UniPi on this PC (pi install npm:@pi-unipi/unipi).'
              }
            />
          }
        >
          {(st) => (
            <Show
              when={viewed()}
              fallback={<RunList status={st()} now={now()} busy={busy()} onRun={runNow} onOpen={(id) => (setViewId(id), setFlash(null))} />}
            >
              {(run) => (
                <RunDetail
                  run={run()}
                  chat={props.chat}
                  now={now()}
                  busy={busy()}
                  onConfirm={setConfirm}
                  onDismissed={() => setViewId(null)}
                  act={act}
                />
              )}
            </Show>
          )}
        </Show>
      </BoardSheet>
      <BoardSheet
        open={props.open && confirm() !== null}
        onClose={() => !busy() && setConfirm(null)}
        label={confirm()?.title ?? 'Are you sure?'}
        testId="dream-confirm"
        footer={
          <div class="dialog-actions">
            <button type="button" class="btn btn-secondary" disabled={busy()} onClick={() => setConfirm(null)}>
              Cancel
            </button>
            <button
              type="button"
              class={`btn ${confirm()?.danger ? 'btn-danger' : 'btn-primary'}`}
              disabled={busy()}
              data-testid="dream-confirm-go"
              onClick={async () => {
                const c = confirm();
                if (!c) return;
                await c.run();
                setConfirm(null);
              }}
            >
              {busy() ? 'Working…' : confirm()?.action}
            </button>
          </div>
        }
      >
        <p class="move-why mt-0">{confirm()?.body}</p>
      </BoardSheet>
    </>
  );
}

function RunList(props: { status: NonNullable<Chat['state']['dream']>; now: number; busy: boolean; onRun: () => void; onOpen: (id: string) => void }) {
  const st = () => props.status;
  return (
    <div class="dream-body" data-testid="dream-list">
      <div class="dream-status" data-on={st().enabled ? '' : undefined} data-running={st().running ? '' : undefined}>
        <div class="dream-status-row">
          <span class="dream-status-dot" aria-hidden="true" />
          <span class="dream-status-label">{st().enabled ? 'On' : 'Off'}</span>
          <span class="dream-status-line" data-testid="dream-status-line">{statusLine(st(), props.now)}</span>
        </div>
        <Show when={!st().enabled}>
          <p class="dream-hint" data-testid="dream-off-hint">
            Turn it on in pi with <code>/unipi:settings</code> → Dream → “Dream in the background”. Run now works either way.
          </p>
        </Show>
        <Show when={st().enabled}>
          <p class="dream-hint">
            Every pi open checks for one: after {st().minSessions} new sessions, at most every {st().minGapHours} h.
          </p>
        </Show>
        <div class="dream-status-actions">
          <button type="button" class="btn btn-primary" disabled={props.busy || st().running} onClick={props.onRun} data-testid="dream-run">
            <Play size={16} /> {st().running ? 'Running…' : 'Run now'}
          </button>
        </div>
      </div>

      <div class="section-row">
        <div class="section-label">Dreams</div>
        <Show when={st().runs.length > 0}>
          <span class="section-row-aside">{st().runs.length}</span>
        </Show>
      </div>
      <Show
        when={st().runs.length > 0}
        fallback={
          <EmptyState
            compact
            testId="dream-no-runs"
            icon={<Moon size={22} />}
            title="No dreams yet"
            body="A dream reviews past sessions, keeps the lessons in memory and proposes skills for you to approve."
          />
        }
      >
        <ul class="dream-runs" data-testid="dream-runs">
          <For each={[...st().runs.filter((r) => r.status === 'running'), ...st().runs.filter((r) => r.status !== 'running')]}>
            {(run) => (
              <li>
                <button type="button" class="dream-run-row" data-status={run.status} onClick={() => props.onOpen(run.id)}>
                  <span class="dream-run-state" aria-hidden="true">
                    <Show when={run.status === 'running'} fallback={<span class="dream-run-dot" />}>
                      <span class="progress-spinner" />
                    </Show>
                  </span>
                  <span class="min-w-0 flex-1 text-left overflow-hidden">
                    <span class="dream-run-title">
                      <span class="shrink-0">{startedText(run.startedAt, props.now)}</span>
                      <span class="dream-run-kind">{run.manual ? 'manual' : 'scheduled'}</span>
                    </span>
                    <span class="dream-run-sub">{runFacts(run, props.now)}</span>
                  </span>
                  <span class="dream-run-status">{RUN_LABEL[run.status]}</span>
                  <Show when={run.pending > 0}>
                    <span class="dream-badge" aria-label={`${run.pending} waiting for approval`}>
                      {run.pending}
                    </span>
                  </Show>
                  <ChevronRight size={16} class="shrink-0 dream-chev" />
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

function RunDetail(props: {
  run: DreamRun;
  chat: Chat;
  now: number;
  busy: boolean;
  onConfirm: (c: Confirm) => void;
  onDismissed: () => void;
  act: (label: string, fn: () => Promise<void>, ok: string) => Promise<void>;
}) {
  const r = () => props.run;
  const [detail, setDetail] = createSignal<DreamDetail | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  const load = async () => {
    const d = await props.chat.dreamDetail(r().id);
    if ('error' in d) setError(d.error);
    else {
      setDetail(d);
      setError(undefined);
    }
  };
  // Load on open / when the run changes state (report lands); poll while running.
  createEffect(
    on(
      () => `${r().id}:${r().status}:${r().hasReport}`,
      () => {
        void load();
        if (r().status !== 'running') return;
        const t = setInterval(() => void load(), 2000);
        onCleanup(() => clearInterval(t));
      },
    ),
  );
  const blocks = createMemo(() => ((detail()?.report ?? '').trim() ? renderBlocks(detail()!.report, false) : []));
  const decide = (p: DreamProposal, approve: boolean) =>
    props.onConfirm({
      title: approve ? `Approve ${p.kind} “${p.name}”?` : `Reject ${p.kind} “${p.name}”?`,
      body: approve
        ? p.kind === 'skill'
          ? 'The skill is copied into the project’s skills folder and must pass the craft-skill check (no hardcoded secrets). If the check fails, nothing is recorded.'
          : 'The check is marked approved. Wiring it into pi as a hook is still up to you.'
        : 'The proposal stays in the dream’s folder but won’t be offered again.',
      action: approve ? 'Approve' : 'Reject',
      danger: !approve,
      run: () => props.act(approve ? 'Approval failed' : 'Couldn’t reject', () => props.chat.dreamAction(approve ? 'approve' : 'reject', r().id, p.id), approve ? `Approved ${p.name}.` : `Rejected ${p.name}.`),
    });
  return (
    <div class="dream-body" data-testid="dream-detail" data-status={r().status}>
      <div class="progress-head">
        <span class="progress-status" data-status={r().status === 'finished' ? 'done' : r().status === 'running' ? 'running' : r().status === 'failed' ? 'failed' : 'paused'}>
          <Show when={r().status === 'running'}>
            <span class="progress-pulse" aria-hidden="true" />
          </Show>
          {RUN_LABEL[r().status]} · {durationText((r().endedAt ?? props.now) - r().startedAt)}
        </span>
        <span class="progress-updated">{r().manual ? 'manual' : 'scheduled'}</span>
      </div>
      <p class="dream-facts">
        {r().sessions} session{r().sessions === 1 ? '' : 's'} digested · {r().events} struggle event{r().events === 1 ? '' : 's'}
        <Show when={r().error}> · <span class="dream-error">{r().error}</span></Show>
      </p>
      <div class="dream-actions">
        <Show when={r().status === 'running'}>
          <button
            type="button"
            class="btn btn-danger"
            disabled={props.busy}
            data-testid="dream-stop"
            onClick={() =>
              props.onConfirm({
                title: 'Stop this dream?',
                body: 'The background pi is stopped now. Memory edits it already made stay; no report is written.',
                action: 'Stop',
                danger: true,
                run: () => props.act('Couldn’t stop', () => props.chat.dreamAction('stop', r().id), 'Dream stopped.'),
              })
            }
          >
            <Square size={14} /> Stop
          </button>
        </Show>
        <Show when={r().status !== 'running'}>
          <button
            type="button"
            class="btn btn-ghost"
            disabled={props.busy}
            data-testid="dream-dismiss"
            onClick={() => void props.act('Couldn’t dismiss', () => props.chat.dreamAction('dismiss', r().id), 'Dismissed.').then(props.onDismissed)}
          >
            Dismiss
          </button>
        </Show>
      </div>

      <div class="section-row">
        <div class="section-label">Proposals</div>
        <Show when={r().pending > 0}>
          <span class="section-row-aside">{r().pending} waiting</span>
        </Show>
      </div>
      <Show
        when={r().proposals.length > 0}
        fallback={<p class="progress-note">{r().status === 'running' ? 'None yet — skills and checks show up here for approval.' : 'None. Memory edits apply on their own.'}</p>}
      >
        <ul class="dream-proposals" data-testid="dream-proposals">
          <For each={r().proposals}>
            {(p) => (
              <li class="dream-proposal" data-decision={p.decision}>
                <div class="min-w-0 flex-1">
                  <span class="dream-proposal-name">{p.name}</span>
                  <span class="dream-proposal-sub">
                    {p.kind === 'skill' ? 'Skill' : 'Check'} · <span class="dream-decision">{DECISION_LABEL[p.decision]}</span>
                  </span>
                </div>
                <Show when={p.decision === 'pending'}>
                  <div class="dream-proposal-actions">
                    <button type="button" class="btn btn-secondary" disabled={props.busy} onClick={() => decide(p, false)}>
                      Reject
                    </button>
                    <button type="button" class="btn btn-primary" disabled={props.busy} onClick={() => decide(p, true)}>
                      Approve
                    </button>
                  </div>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <Show when={r().status === 'running' || (detail()?.steps.length ?? 0) > 0}>
        <div class="section-label">{r().status === 'running' ? 'Live' : 'Trajectory'}</div>
        <Show when={(detail()?.steps.length ?? 0) > 0} fallback={<p class="progress-note">{error() ? `Couldn’t read it: ${error()}` : 'Starting…'}</p>}>
          <ol class="dream-steps" data-testid="dream-steps">
            <For each={detail()!.steps.slice(-30)}>
              {(step, i) => (
                <li data-kind={step.kind} data-newest={i() === Math.min(30, detail()!.steps.length) - 1 && r().status === 'running' ? '' : undefined}>
                  <span class="dream-step-glyph" aria-hidden="true">
                    {step.kind === 'tool' ? '▸' : step.kind === 'error' ? '✗' : step.kind === 'text' ? '·' : '›'}
                  </span>
                  <span class="dream-step-text">{step.text}</span>
                </li>
              )}
            </For>
          </ol>
        </Show>
      </Show>

      <div class="section-label">Report</div>
      <Show
        when={blocks().length > 0}
        fallback={<p class="progress-note">{r().status === 'running' ? 'Written at the end of the dream.' : error() ? `Couldn’t read it: ${error()}` : detail() ? 'This dream left no report.' : 'Loading…'}</p>}
      >
        <Show when={r().summary?.length}>
          <p class="dream-summary">{r().summary!.join(' · ')}</p>
        </Show>
        <div class="dream-report chat-prose md" data-testid="dream-report">
          <For each={blocks()}>{(html) => <MdBlock html={html} class="chat-block" chat={props.chat} />}</For>
        </div>
      </Show>
    </div>
  );
}
