/**
 * UNI-241 (plan S3, §4.3 steps 1–5): the first-run "Set up this PC" takeover.
 *
 * A full-screen route (`#/setup`), reached only from `setupGateOpen` — the
 * desktop app opens it on launch when detection says this PC isn't ready and
 * "Not now" hasn't been pressed recently. It never appears on the phone.
 *
 * The four faces of §4.3 live here: the consent list (a checkbox per
 * component, what it is, where it goes, its size, whether it needs internet),
 * the progress rows while the engine runs, the error sheet when a step fails
 * (Retry / Copy command / Skip), and the done screen.
 */
import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import { AlertTriangle, Check, ChevronRight, Copy, Download, RefreshCw, RotateCcw, Smartphone, Terminal, Wand2, X, Zap } from 'lucide-solid';
import {
  badgeText,
  checkForUpdates,
  consentLine,
  dismissSetup,
  installComponent,
  methodsForStep,
  refreshPlan,
  refreshSetup,
  runSetup,
  setupBusy,
  setupError,
  setupPlan,
  setupReport,
  updateReport,
  setupRows,
  type PlanStep,
  type SetupRow,
  type StepResult,
} from '../lib/setup';
import { can } from '../lib/platform';
import { openPairPhone } from '../lib/pairPhone';
import crab from '../assets/unicrab-pixel.png';
import { useNavigate } from '@solidjs/router';
import '../components/setup.css';

type Phase = 'list' | 'running' | 'done' | 'error';
/** Where a step reads as, in the progress list. */
type Tone = 'waiting' | 'running' | 'done' | 'failed' | 'skipped';

export function SetupWizard() {
  const navigate = useNavigate();
  const [phase, setPhase] = createSignal<Phase>('list');
  /** Which rows are ticked. Seeded from the plan the core built (§4.3: the
   * consent list is what the install would do, minus what the user opts out of). */
  const [ticked, setTicked] = createSignal<string[]>([]);
  const [results, setResults] = createSignal<StepResult[]>([]);
  /** Live per-step labels, while the engine runs. */
  const [live, setLive] = createSignal<Record<string, string>>({});
  const [failed, setFailed] = createSignal<StepResult | undefined>();
  const [cancelled, setCancelled] = createSignal(false);
  const [logShown, setLogShown] = createSignal(false);
  const [handle, setHandle] = createSignal<ReturnType<typeof runSetup> | undefined>();
  /** Decision 8's "Install for me" running for one component (a manual component, e.g. xvfb). */
  const [installing, setInstalling] = createSignal(false);
  const [installNote, setInstallNote] = createSignal<string | undefined>();

  const report = setupReport;
  const rows = createMemo<SetupRow[]>(() => setupRows(report(), updateReport(), setupPlan()));
  const planSteps = createMemo<PlanStep[]>(() => setupPlan()?.steps ?? []);
  const consent = createMemo(() => consentLine(rows().map((r) => ({ ...r, ticked: r.ticked && ticked().includes(r.id) }))));
  const offline = () => setupPlan()?.offline ?? false;
  const needsInternet = () => setupPlan()?.needsInternet ?? [];
  const logText = () => results().flatMap((s) => s.logTail).join('\n');

  onMount(async () => {
    await refreshSetup();
    await refreshPlan();
    await checkForUpdates();
    // Pre-tick exactly what the core would do, and nothing it wouldn't: the
    // rows that already satisfy their step are not the user's to opt out of.
    setTicked((setupPlan()?.steps ?? []).filter((s) => s.action === 'install' || s.action === 'update').map((s) => s.id));
  });

  const toggle = (id: string) => setTicked((t) => (t.includes(id) ? t.filter((x) => x !== id) : [...t, id]));

  const install = async () => {
    setPhase('running');
    setResults([]);
    setLive({});
    setFailed(undefined);
    setCancelled(false);
    const run = runSetup((p) => setLive((l) => ({ ...l, [p.step]: p.detail ?? stateWord(p.state.state) })), ticked());
    setHandle(run);
    const outcome = await run.outcome;
    setHandle(undefined);
    if (!outcome) {
      setResults([]);
      setPhase('error');
      return;
    }
    setResults(outcome.steps);
    if (outcome.cancelled) {
      setCancelled(true);
      setPhase('done');
      return;
    }
    const bad = outcome.steps.find((s) => s.state.state === 'failed');
    if (bad) {
      setFailed(bad);
      setPhase('error');
      return;
    }
    // Done: re-read the PC, so the screen and the Components rows tell the truth.
    await refreshSetup();
    setPhase('done');
  };

  const cancel = () => {
    handle()?.cancel();
    setCancelled(true);
  };

  /** §4.1 / decision 8: one component, several methods, then the command.
   * Separate from the consent list because the core never plans these itself. */
  const installForMe = async (id: string) => {
    setInstalling(true);
    setInstallNote(`Installing ${id}…`);
    const outcome = await installComponent(id, (p) => setInstallNote(p.detail ?? stateWord(p.state.state)));
    setInstalling(false);
    const step = outcome?.steps.find((s) => s.step === id);
    if (!outcome || !step || step.state.state !== 'done') {
      const message = step && step.state.state === 'failed' ? step.state.message : `Could not install ${id}`;
      setInstallNote(step?.fallbackCommand ? `${message} — try: ${step.fallbackCommand}` : message);
      return;
    }
    setInstallNote(undefined);
    // Re-read the PC so the row stops offering an install.
    await refreshSetup();
    await refreshPlan();
  };

  /** A row's component, for its DIY command. */
  const component = (id: string) => report()?.components.find((c) => c.name === id);

  /**
   * §4.3 point 3's "Try another way": the next method the engine has for the
   * failed step, or nothing. Only offered when the chain really has another way
   * — `attempted` is what the engine already tried, so this can never offer a
   * rerun of the method that just failed.
   */
  const anotherWay = (): string | undefined => {
    const step = failed();
    if (!step) return undefined;
    const tried = step.attempted ?? [];
    return methodsForStep(step.step).find((m) => !tried.includes(m));
  };

  /** The event's own state word, as a label a user reads. */
  const stateWord = (state: string): string => ({ waiting: 'Waiting', running: 'Working…', done: 'Done', failed: 'Failed', skipped: 'Skipped' })[state] ?? state;

  /** One step's line in the progress list. */
  const stepTone = (id: string): { label: string; tone: Tone } => {
    const done = results().find((s) => s.step === id);
    if (done) {
      const state = done.state;
      if (state.state === 'failed') return { label: state.message, tone: 'failed' };
      if (state.state === 'skipped') return { label: state.reason, tone: 'skipped' };
      return { label: 'Done', tone: 'done' };
    }
    const liveLabel = live()[id];
    if (liveLabel) return { label: liveLabel, tone: liveLabel === 'Working…' ? 'running' : 'done' };
    return { label: 'Waiting', tone: 'waiting' };
  };

  /** How a finished step reads on the done screen. */
  const doneLabel = (s: StepResult): string => {
    const state = s.state;
    if (state.state === 'failed') return state.message;
    if (state.state === 'skipped') return state.reason;
    return 'Done';
  };

  /** The error sheet's headline: the failed step's message, else the lib's. */
  const failedMessage = (): string => {
    const f = failed();
    if (f && f.state.state === 'failed') return f.state.message;
    return setupError() ?? 'Unknown error';
  };

  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* the sheet still shows the command; the clipboard is a convenience */
    }
  }

  return (
    <div class="setup-screen" data-testid="setup-wizard">
      <div class="setup-card">
        {/* ------------------------------------------------ the consent list */}
        <Show when={phase() === 'list'}>
          <header class="setup-head">
            <img class="setup-crab" src={crab} alt="" />
            <h1>Set up this PC</h1>
            <p class="setup-sub">UniPi runs your pi sessions on this computer. It needs a few things first — you choose which.</p>
          </header>

          <Show when={!report()}>
            <p class="setup-note">
              <RefreshCw size={14} class={setupBusy() ? 'setup-spin' : ''} /> Looking at what this PC already has…
            </p>
          </Show>

          <Show when={offline()}>
            <p class="setup-offline" data-testid="setup-offline">
              <AlertTriangle size={14} /> No internet right now.{' '}
              {needsInternet().length ? `These need it: ${needsInternet().join(', ')}.` : 'Some steps need it.'} A bundled host still gives you local
              Files and Terminal.
            </p>
          </Show>

          <ul class="setup-list" data-testid="setup-list">
            <For each={rows()}>
              {(row) => (
                <li>
                  <button
                    type="button"
                    class="setup-row"
                    data-testid={`setup-row-${row.id}`}
                    data-ticked={ticked().includes(row.id) ? 'true' : 'false'}
                    // A `manual` row is an opt-in, not part of the consent list:
                    // the core never plans it, so it is a separate button.
                    disabled={row.manual || row.action === 'already-present'}
                    onClick={() => toggle(row.id)}
                  >
                    <span class="setup-check" aria-hidden="true">
                      <Show when={ticked().includes(row.id)}>
                        <Check size={14} strokeWidth={3} />
                      </Show>
                    </span>
                    <span class="setup-row-text">
                      <span class="setup-row-label">
                        {row.label}
                        {/* The status chip. `manual` and `ok` are neutral; red is
                            reserved for `broken` (setup.css). */}
                        <em class="setup-badge" data-badge={row.badge}>
                          {badgeText(row.badge, row)}
                        </em>
                      </span>
                      {/* §4.3's sub-line: what it is, where it goes, its size,
                          and whether it needs internet or admin. */}
                      <span class="setup-row-note" data-testid={`setup-row-note-${row.id}`}>
                        {row.note}
                      </span>
                    </span>
                  </button>
                  {/* §4.1 / decision 8: manual components are never automatic, so they are offered
                      as its own opt-in with the engine's method chain. */}
                  <Show when={row.manual && component(row.id)?.fix}>
                    <div class="setup-row-actions">
                      <button
                        type="button"
                        class="btn btn-secondary btn-sm"
                        data-testid={`setup-install-for-me-${row.id}`}
                        disabled={installing() || !can('selfInstall')}
                        onClick={() => void installForMe(row.id)}
                      >
                        <Download size={14} /> Install for me
                      </button>
                      <button
                        type="button"
                        class="btn btn-ghost btn-sm"
                        data-testid={`setup-copy-${row.id}`}
                        onClick={() => void copy(component(row.id)!.fix!)}
                      >
                        <Copy size={14} /> Copy
                      </button>
                    </div>
                  </Show>
                </li>
              )}
            </For>
          </ul>

          <p class="setup-consent" data-testid="setup-consent">
            {consent()}
          </p>
          <div class="setup-actions">
            <button
              type="button"
              class="btn btn-primary"
              data-testid="setup-install"
              disabled={setupBusy() || ticked().length === 0}
              onClick={() => void install()}
            >
              <Download size={16} /> Install
            </button>
            <button
              type="button"
              class="btn btn-ghost"
              data-testid="setup-not-now"
              onClick={() => {
                dismissSetup();
                navigate('/settings');
              }}
            >
              Not now
            </button>
          </div>
          <p class="setup-fine">Nothing is installed silently. "Not now" still lets the app work as a client of your other PCs.</p>
          <Show when={installNote()}>
            <p class="setup-note" data-testid="setup-install-note">
              <RefreshCw size={14} class={installing() ? 'setup-spin' : ''} /> {installNote()}
            </p>
          </Show>
        </Show>

        {/* --------------------------------------------------- the progress */}
        <Show when={phase() === 'running'}>
          <header class="setup-head">
            <h1>Setting up…</h1>
            <p class="setup-sub">Each step reports as it goes.</p>
          </header>
          <ul class="setup-steps" data-testid="setup-steps">
            <For each={planSteps()}>
              {(step) => {
                const state = () => stepTone(step.id);
                return (
                  <li data-testid={`setup-step-${step.id}`} data-tone={state().tone}>
                    <span class="setup-step-dot" aria-hidden="true" />
                    <span class="setup-step-text">
                      <span class="setup-step-label">{step.label}</span>
                      <span class="setup-step-state">{state().label}</span>
                    </span>
                    <Show when={state().tone === 'running'}>
                      <RefreshCw size={14} class="setup-spin" />
                    </Show>
                  </li>
                );
              }}
            </For>
          </ul>
          <div class="setup-actions">
            <button type="button" class="btn btn-secondary" data-testid="setup-cancel" onClick={cancel}>
              <X size={16} /> Cancel
            </button>
          </div>
        </Show>

        {/* -------------------------------------------------- the error sheet */}
        <Show when={phase() === 'error'}>
          <header class="setup-head">
            <h1>That didn't work</h1>
            <p class="setup-sub">{failed() ? `A step failed while setting up this PC` : 'The install stopped'}</p>
          </header>
          <p class="setup-error-message" data-testid="setup-error-message">
            {failedMessage()}
          </p>
          <Show when={failed() && failed()!.logTail.length > 0}>
            <pre class="setup-log" data-testid="setup-error-log">
              {failed()!.logTail.slice(-20).join('\n')}
            </pre>
          </Show>
          <div class="setup-actions">
            <button type="button" class="btn btn-primary" data-testid="setup-retry" onClick={() => void install()}>
              <RotateCcw size={16} /> Retry
            </button>
            {/* §4.3 point 3's "Try another way": the engine has a method chain
                for this component (pi's N1 → N2), so offer the next one rather than only a rerun. */}
            <Show when={anotherWay()}>{(next) => (
              <button
                type="button"
                class="btn btn-secondary"
                data-testid="setup-another-way"
                disabled={installing()}
                onClick={() => void installForMe(failed()!.step)}
              >
                <Wand2 size={16} /> Try {next()}
              </button>
            )}</Show>
            <Show when={failed()?.fallbackCommand}>
              <button type="button" class="btn btn-secondary" data-testid="setup-copy-command" onClick={() => void copy(failed()!.fallbackCommand!)}>
                <Copy size={16} /> Copy command
              </button>
            </Show>
            <button
              type="button"
              class="btn btn-ghost"
              data-testid="setup-skip"
              onClick={() => {
                setFailed(undefined);
                setPhase('list');
              }}
            >
              Skip
            </button>
          </div>
        </Show>

        {/* ----------------------------------------------------- done screen */}
        <Show when={phase() === 'done'}>
          <header class="setup-head">
            <h1>{cancelled() ? 'Stopped' : 'This PC is ready'}</h1>
            <p class="setup-sub">
              {cancelled() ? 'What finished before you cancelled is installed; you can run this again any time.' : 'Everything you picked is installed.'}
            </p>
          </header>
          <ul class="setup-steps" data-testid="setup-done-steps">
            <For each={results()}>
              {(s) => (
                <li data-tone={s.state.state === 'failed' ? 'failed' : s.state.state === 'skipped' ? 'skipped' : 'done'}>
                  <span class="setup-step-dot" aria-hidden="true" />
                  <span class="setup-step-text">
                    <span class="setup-step-label">{s.step}</span>
                    <span class="setup-step-state">{doneLabel(s)}</span>
                  </span>
                </li>
              )}
            </For>
          </ul>
          <Show when={report() && !report()!.path.localBinOnLoginPath}>
            <p class="setup-note" data-testid="setup-path-note">
              <Terminal size={14} /> Open a new terminal so <code>pi</code> is on your PATH.
            </p>
          </Show>
          <Show when={results().some((s) => s.step === 'unipi' && s.state.state === 'done')}>
            <p class="setup-note" data-testid="setup-restart-note">
              <Zap size={14} /> Restart any running pi sessions to load UniPi.
            </p>
          </Show>
          <div class="setup-actions">
            <button type="button" class="btn btn-primary" data-testid="setup-done" onClick={() => navigate('/')}>
              Done
            </button>
            <Show when={can('camera')}>
              <button
                type="button"
                class="btn btn-secondary"
                data-testid="setup-pair"
                onClick={() => {
                  navigate('/settings');
                  openPairPhone();
                }}
              >
                <Smartphone size={16} /> Pair a phone
              </button>
            </Show>
          </div>
        </Show>

        <Show when={logText().length > 0 && phase() !== 'list'}>
          <button type="button" class="setup-log-toggle" data-testid="setup-log-toggle" onClick={() => setLogShown(!logShown())}>
            {logShown() ? 'Hide the log' : 'Show the log'} <ChevronRight size={14} class={logShown() ? 'setup-flip' : ''} />
          </button>
        </Show>
        <Show when={logShown()}>
          <pre class="setup-log" data-testid="setup-log">
            {logText()}
          </pre>
        </Show>
      </div>
    </div>
  );
}
