/**
 * UNI-241 (plan S3, docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md §4.3): the
 * desktop app's own first-run "Set up this PC" wizard and the Settings →
 * This computer → Components rows, on top of the core's `setup_*` commands
 * (`setup::detect`, `setup::install`, `setup::updates`).
 *
 * Two shapes, one module:
 *
 * - **`SetupReport`** — what this PC has (the wizard's checklist and the
 *   Components rows' versions);
 * - **`UpdateReport`** — what is newer than what this PC has, from four feeds
 *   (`setup_check_updates`), for Check for updates.
 *
 * The desktop app's own version is NOT in either: its check is the signed
 * updater in `lib/desktopUpdate.ts`, which the Components screen reads
 * directly (§4.3 point 6 / UNI-263).
 *
 * Everything goes through an injectable transport so the wizard's state
 * machine, the row labels and the entry gate are all testable with no core,
 * and so the captures can answer from `window.__UNIPI_SETUP__`.
 */
import { invoke, Channel } from '@tauri-apps/api/core';
import { createSignal } from 'solid-js';
import { z } from 'zod';

// ------------------------------------------------------------------- shapes

export const ComponentStateSchema = z.enum(['ok', 'missing', 'outdated', 'broken', 'unsupported']);
export type ComponentState = z.infer<typeof ComponentStateSchema>;
export const SourceSchema = z.enum(['found', 'unipi-managed']);
export type Source = z.infer<typeof SourceSchema>;

export const ComponentSchema = z.object({
  name: z.string(),
  state: ComponentStateSchema,
  version: z.string().optional(),
  path: z.string().optional(),
  source: SourceSchema,
  fix: z.string().optional(),
  note: z.string().optional(),
});
export type Component = z.infer<typeof ComponentSchema>;

export const SetupReportSchema = z.object({
  components: z.array(ComponentSchema),
  path: z.object({
    value: z.string(),
    fromShell: z.array(z.string()),
    localBinOnLoginPath: z.boolean(),
    probed: z.boolean(),
  }),
  ready: z.boolean(),
  updateAvailable: z.boolean(),
  at: z.number(),
});
export type SetupReport = z.infer<typeof SetupReportSchema>;

/** What a step would do (decision 4: an older version is an update, after a prompt). */
export const ActionSchema = z.enum(['already-present', 'install', 'update', 'manual', 'install-for-me']);
export type Action = z.infer<typeof ActionSchema>;

export const PlanStepSchema = z.object({
  id: z.string(),
  label: z.string(),
  action: ActionSchema,
  sizeHint: z.string().optional(),
  needs: z.array(z.string()).default([]),
});
export type PlanStep = z.infer<typeof PlanStepSchema>;

export const PlanSchema = z.object({
  steps: z.array(PlanStepSchema),
  needsInternet: z.array(z.string()),
  offline: z.boolean(),
  consent: z.string(),
});
export type Plan = z.infer<typeof PlanSchema>;

/** Where a step is, as it streams. */
export const StepStateSchema = z.discriminatedUnion('state', [
  z.object({ state: z.literal('waiting') }),
  z.object({ state: z.literal('running') }),
  z.object({ state: z.literal('done') }),
  z.object({ state: z.literal('failed'), message: z.string() }),
  z.object({ state: z.literal('skipped'), reason: z.string() }),
]);
export type StepState = z.infer<typeof StepStateSchema>;

export const ProgressSchema = z.object({
  step: z.string(),
  state: StepStateSchema,
  detail: z.string().optional(),
});
export type Progress = z.infer<typeof ProgressSchema>;

export const StepResultSchema = z.object({
  step: z.string(),
  state: StepStateSchema,
  logTail: z.array(z.string()).default([]),
  attempted: z.array(z.string()).default([]),
  fallbackCommand: z.string().optional(),
});
export type StepResult = z.infer<typeof StepResultSchema>;

export const OutcomeSchema = z.object({
  steps: z.array(StepResultSchema),
  cancelled: z.boolean(),
});
export type Outcome = z.infer<typeof OutcomeSchema>;

/** How the version the feed offers compares to what is installed. */
export const UpdateSchema = z.discriminatedUnion('state', [
  z.object({ state: z.literal('current'), installed: z.string(), offered: z.string().optional() }),
  z.object({ state: z.literal('available'), from: z.string(), to: z.string() }),
  z.object({ state: z.literal('missing'), to: z.string() }),
  z.object({ state: z.literal('unknown') }),
]);
export type Update = z.infer<typeof UpdateSchema>;

export const ComponentUpdateSchema = z.object({
  name: z.string(),
  update: UpdateSchema,
  detail: z.string().optional(),
});
export type ComponentUpdate = z.infer<typeof ComponentUpdateSchema>;

export const UpdateReportSchema = z.object({
  components: z.array(ComponentUpdateSchema),
  anyAvailable: z.boolean(),
  offline: z.boolean(),
});
export type UpdateReport = z.infer<typeof UpdateReportSchema>;

// ---------------------------------------------------------------- transport

export interface SetupTransport {
  detect(): Promise<unknown>;
  plan(): Promise<unknown>;
  /** Streams `Progress` back as the steps run; resolves with the `Outcome`.
   * `only` is the wizard's ticked step ids. */
  run(onProgress: (p: Progress) => void, only?: string[]): Promise<unknown>;
  cancel(): Promise<void>;
  checkUpdates(): Promise<unknown>;
  /** Decision 8's "Install for me": one component, several methods. */
  installComponent(name: string, onProgress: (p: Progress) => void): Promise<unknown>;
}

const tauriTransport: SetupTransport = {
  detect: () => invoke('setup_detect'),
  plan: () => invoke('setup_plan'),
  run: (onProgress, only) => {
    const channel = new Channel<Progress>();
    channel.onmessage = onProgress;
    return invoke('setup_run', { onProgress: channel, only });
  },
  cancel: () => invoke('setup_cancel'),
  checkUpdates: () => invoke('setup_check_updates'),
  installComponent: (name, onProgress) => {
    const channel = new Channel<Progress>();
    channel.onmessage = onProgress;
    return invoke('install_component_here', { name, onProgress: channel });
  },
};

let transport: SetupTransport | undefined;
export function setSetupTransport(t: SetupTransport | undefined): void {
  transport = t;
}
/** The captures answer from `window.__UNIPI_SETUP__` with no core at all. */
const setupTransport = (): SetupTransport => {
  const w = window as unknown as { __UNIPI_SETUP__?: SetupTransport };
  return transport ?? w.__UNIPI_SETUP__ ?? tauriTransport;
};

// ------------------------------------------------------------------ signals

/** `undefined` until the first detection landed; the wizard and the gate read it. */
const [report, setReport] = createSignal<SetupReport | undefined>();
export const setupReport = report;
/** True while a detection, a run or an update check is in flight. */
const [busy, setBusy] = createSignal(false);
export const setupBusy = busy;
const [plan, setPlan] = createSignal<Plan | undefined>();
export const setupPlan = plan;
const [updates, setUpdates] = createSignal<UpdateReport | undefined>();
export const updateReport = updates;
/** The last error worth showing (a failed step, a failed command). */
const [error, setError] = createSignal<string | undefined>();
export const setupError = error;
/** True while the install engine is running (Cancel is offered). */
const [running, setRunning] = createSignal(false);
export const setupRunning = running;

function messageOf(e: unknown): string {
  const err = e as { message?: string } | undefined;
  return err?.message ?? String(e);
}

/** `setup_detect`, into `setupReport`. Never throws: a failed detection is `undefined`. */
export async function refreshSetup(): Promise<SetupReport | undefined> {
  setBusy(true);
  try {
    const parsed = SetupReportSchema.safeParse(await setupTransport().detect());
    if (!parsed.success) {
      setError('The core sent something this app does not understand');
      return undefined;
    }
    setReport(parsed.data);
    setError(undefined);
    return parsed.data;
  } catch (e) {
    setError(messageOf(e));
    return undefined;
  } finally {
    setBusy(false);
  }
}

/** `setup_plan`. */
export async function refreshPlan(): Promise<Plan | undefined> {
  try {
    const parsed = PlanSchema.safeParse(await setupTransport().plan());
    if (!parsed.success) return undefined;
    setPlan(parsed.data);
    return parsed.data;
  } catch (e) {
    setError(messageOf(e));
    return undefined;
  }
}

/** `setup_check_updates`, into `updateReport`. */
export async function checkForUpdates(): Promise<UpdateReport | undefined> {
  setBusy(true);
  try {
    const parsed = UpdateReportSchema.safeParse(await setupTransport().checkUpdates());
    if (!parsed.success) {
      setError('The update check sent something this app does not understand');
      return undefined;
    }
    setUpdates(parsed.data);
    setError(undefined);
    return parsed.data;
  } catch (e) {
    setError(messageOf(e));
    return undefined;
  } finally {
    setBusy(false);
  }
}

export interface RunHandle {
  outcome: Promise<Outcome | undefined>;
  cancel(): void;
}

/** `setup_run`: streams `Progress`, resolves with the `Outcome`.
 * `only` narrows the plan to the ticked steps (§4.3's consent list). */
export function runSetup(onProgress: (p: Progress) => void, only?: string[]): RunHandle {
  setRunning(true);
  setError(undefined);
  let cancel = (): void => {};
  const outcome = (async () => {
    try {
      const parsed = OutcomeSchema.safeParse(await setupTransport().run(onProgress, only));
      if (!parsed.success) {
        setError('The install sent something this app does not understand');
        return undefined;
      }
      return parsed.data;
    } catch (e) {
      setError(messageOf(e));
      return undefined;
    } finally {
      setRunning(false);
    }
  })();
  cancel = () => {
    void setupTransport().cancel();
  };
  return { outcome, cancel };
}

/** Decision 8's "Install for me" for one component (a manual-install one, e.g. xvfb). */
export async function installComponent(name: string, onProgress: (p: Progress) => void): Promise<Outcome | undefined> {
  setRunning(true);
  setError(undefined);
  try {
    const parsed = OutcomeSchema.safeParse(await setupTransport().installComponent(name, onProgress));
    return parsed.success ? parsed.data : undefined;
  } catch (e) {
    setError(messageOf(e));
    return undefined;
  } finally {
    setRunning(false);
  }
}

/** Test/capture reset. */
export function resetSetup(): void {
  setReport(undefined);
  setPlan(undefined);
  setUpdates(undefined);
  setError(undefined);
  setRunning(false);
  setBusy(false);
  setSetupTransport(undefined);
}

// --------------------------------------------------------------- row labels

/** One Components / wizard row: what to do about a component, and why. */
export interface SetupRow {
  /** The component name (`node`, `pi`, …, `xvfb`). */
  id: string;
  /** The row's title (the component's own label). */
  label: string;
  /** The one-line status, as a chip: "Install", "Update", "Newer than tested"… */
  badge: Badge;
  /** What it is, where it goes, its size and whether it needs internet (§4.3). */
  note: string;
  /** What pressing the row's action does. */
  action: Action;
  /** Pre-ticked in the wizard: it needs the install engine's attention. */
  ticked: boolean;
  /** True when nothing can be done here (the command is the answer). */
  manual: boolean;
  state: ComponentState;
}

const LABEL: Record<string, string> = {
  node: 'Node (JavaScript runtime)',
  npm: 'npm',
  pi: 'pi (coding agent)',
  unipi: 'UniPi extensions',
  'agent-browser': 'agent-browser (browser skill)',
  kanboard: 'Board binary',
  xvfb: 'Hidden browser display (Xvfb)',
  service: 'systemd',
  host: 'Background host',
};

export function componentLabel(name: string): string {
  return LABEL[name] ?? name;
}

/**
 * The state of one component, from the detection and (when it has landed) the
 * update check. The two rules the brief asks for:
 *
 * - the offered version is newer → **Update <from> → <to>**, pre-ticked (§4.3,
 *   decision 4: an older one is updated after a prompt);
 * - this PC's version is newer than the offered one → **ok (newer than
 *   tested)**, left alone — the app never downgrades a user's own version.
 */
export function componentUpdate(name: string, report?: SetupReport, updates?: UpdateReport): Update | undefined {
  const from = updates?.components.find((c) => c.name === name);
  return from?.update;
}

/** "Update 22.18.0 → 22.19.0" for a row whose feed version is newer. */
function updateText(u: Update): string {
  return u.state === 'available' ? `Update ${u.from} → ${u.to}` : u.state === 'missing' ? `Install ${u.to}` : '';
}

/** The chip a row wears. Red is reserved for `broken` (see setup.css). */
function badgeOf(action: Action, state: ComponentState, newer: boolean): Badge {
  if (state === 'broken') return 'broken';
  if (action === 'manual') return 'manual';
  if (action === 'update') return 'update';
  if (action === 'install') return 'install';
  if (newer) return 'newer';
  return 'ok';
}

/** The chip's words, for a row that is simply fine. */
function okChip(row: SetupRow): string {
  // "Installed — 24.16.0, couldn't check" and friends already say what is true;
  // the chip just repeats the leading claim.
  return row.note.split(' — ')[0] ?? 'Ready';
}

/** The chip's words. */
export function badgeText(badge: Badge, row: SetupRow): string {
  switch (badge) {
    case 'install':
      return 'Install';
    case 'update':
      return 'Update';
    case 'broken':
      return 'Broken';
    case 'manual':
      return 'By hand';
    case 'newer':
      return 'Newer than tested';
    default:
      return okChip(row);
  }
}

/**
 * Is this PC's version newer than the one the feed offers? Decision 4 says a
 * newer version is left alone, and the user should see *why* a row reads "ok".
 * The core answers `Current` with the version it compared against precisely so
 * this can be told apart from merely being up to date.
 */
export function newerThanOffered(u: Update | undefined, installed?: string): boolean {
  if (!u || u.state !== 'current' || !u.offered) return false;
  const here = installed ?? u.installed;
  return versionTuple(here) > versionTuple(u.offered);
}

/** `1.2.3`, `v1.2.3`, `1.2.3-alpha.4` → `(1, 2, 3)` (never a caret). */
export function versionTuple(text: string): [number, number, number] {
  const digits = text
    .trim()
    .replace(/^[^\d]*/, '')
    .split('')
    .filter((c) => /\d|\./.test(c))
    .join('');
  const parts = digits.split('.').map((p) => Number.parseInt(p, 10) || 0);
  return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
}

/** Is `a` a newer version than `b`? Element-wise, so `0.4.10` beats `0.4.9`. */
export function versionNewer(a: string, b: string): boolean {
  const [a1, a2, a3] = versionTuple(a);
  const [b1, b2, b3] = versionTuple(b);
  return a1 !== b1 ? a1 > b1 : a2 !== b2 ? a2 > b2 : a3 > b3;
}

/** What a row is, in one short status word: the tick-box's chip. */
export type Badge = 'install' | 'update' | 'ok' | 'newer' | 'broken' | 'manual';

/** Components the app never installs on its own: the answer is the
 * package-manager command (decision 8's chain is user-initiated), and the
 * core plans them as `Action::Manual`. (herdr/tmux are gone, D-14.) */
const NEVER_AUTOMATIC = new Set(['xvfb']);

/** §4.3's "needs internet": every one of these is a download. */
const NEEDS_INTERNET = new Set(['node', 'pi', 'unipi', 'kanboard', 'agent-browser']);
/** §4.3's "needs admin rights": the package managers do. */
const NEEDS_ADMIN = new Set(['xvfb']);

/** Where a component lands, so a row can say where its files go (§4.3). */
const PLACE: Record<string, string> = {
  node: '~/.local/share/pi-node',
  npm: 'comes with Node',
  pi: '~/.pi/agent',
  unipi: "a pi package",
  'agent-browser': '~/.pi/agent/npm',
  kanboard: 'no download of its own',
  xvfb: 'your package manager · lets the browser show only in the app',
  host: '~/.unipi/app-host',
};

/** The wizard's checklist / the Components rows, in install-plan order.
 *
 * Each row carries two separate lines, because they answer different questions:
 * `badge` is the status word (what will happen, or what is true), and `note` is
 * §4.3's "what it is, where it goes, how big, does it need internet". They used
 * to be the same string, so a row read "Not installed / Not installed".
 *
 * A row is pre-ticked when the core's plan would do work on it — including
 * decision 4's `update` — and never for a manual one (`Action::Manual`). */
export function setupRows(report?: SetupReport, updates?: UpdateReport, plan?: Plan): SetupRow[] {
  const order = ['node', 'pi', 'unipi', 'agent-browser', 'kanboard', 'xvfb'];
  const step = (id: string) => plan?.steps.find((s) => s.id === id);
  return order.flatMap((id) => {
    const component = report?.components.find((c) => c.name === id);
    if (!component) return [];
    const u = componentUpdate(id, report, updates);
    const stepAction = step(id)?.action;
    const sizeHint = step(id)?.sizeHint;
    // The plan is the authority on whether work is needed; the feeds only say
    // what the offered version is (and so what a row reads as).
    let action: Action = stepAction ?? (component.state === 'ok' ? 'already-present' : component.state === 'outdated' ? 'update' : 'install');
    let detail = '';
    const newer = !!u && u.state === 'current' && newerThanOffered(u, component.version);
    if (u && u.state === 'available') {
      detail = updateText(u);
      action = 'update';
    } else if (newer) {
      // Decision 4: ours is newer than the feed's, so it is left alone — and
      // the row says so instead of a bare "up to date".
      detail = `Newer than tested — ${u.offered}`;
      action = 'already-present';
    } else if (u && u.state === 'missing') {
      detail = updateText(u);
      action = 'install';
    } else if (component.state === 'ok') {
      // We know what is installed. Whether it is the *newest* is a separate
      // claim, and only the feeds can make it — so an absent or unknown answer
      // never reads as "Up to date" (§4.3 point 4: offline must not claim it).
      if (u === undefined) detail = component.version ? `Installed — ${component.version}` : 'Ready';
      else if (u.state === 'unknown') detail = component.version ? `Installed — ${component.version}, couldn’t check` : 'Couldn’t check';
      else if (u.state === 'current') detail = component.version ? `Up to date — ${component.version}` : 'Up to date';
      else detail = component.version ? `Installed — ${component.version}` : 'Ready';
    } else if (component.state === 'outdated') {
      detail = component.version ? `Older — ${component.version}` : 'Older than tested';
      action = 'update';
    } else if (component.state === 'broken') {
      detail = 'Broken — repair it';
      action = 'install';
    } else if (component.state === 'missing') {
      detail = 'Not installed';
      action = 'install';
    } else {
      detail = component.note ?? 'Not needed on this PC';
      action = 'already-present';
    }
    const manual = NEVER_AUTOMATIC.has(id) || component.state === 'unsupported' || action === 'manual';
    if (manual) {
      detail = component.fix ?? component.note ?? detail;
      action = 'manual';
    }
    const badge = badgeOf(action, component.state, newer);
    // §4.3's sub-line: what it is / where it goes / how big / what it needs.
    // Intrinsic to the component, not to the plan, so the wizard's list and the
    // Components rows read the same whether or not a plan has landed.
    const parts = [PLACE[id], sizeHint, NEEDS_ADMIN.has(id) ? 'needs admin rights' : NEEDS_INTERNET.has(id) ? 'needs internet' : undefined];
    const note = [detail, ...parts.filter(Boolean)].filter(Boolean).join(' · ');
    return [{ id, label: componentLabel(id), badge, note, action, ticked: action === 'install' || action === 'update', manual, state: component.state }];
  });
}

/**
 * The engine's method chain for a component (decision 8), in the order
 * `setup::install::methods_for` tries it. The UI needs it to offer
 * "Try another way" only when another way genuinely exists, so this mirrors the
 * core rather than guessing: on a failure the sheet shows the first method that
 * is not in the step's `attempted` list.
 *
 * PC-specific methods (which package manager) are named by the core; the
 * chain's length and shape are what matters here, and `attempted` is the
 * authority on what ran.
 */
export function methodsForStep(component: string): string[] {
  switch (component) {
    case 'pi':
      return ['pi.dev installer', 'npm'];
    default:
      return [];
  }
}

/** What the wizard's consent line says, given the ticked rows. */
export function consentLine(rows: SetupRow[]): string {
  const wanted = rows.filter((r) => r.ticked && !r.manual);
  if (wanted.length === 0) return 'Everything UniPi needs is already here';
  return `UniPi will download and install: ${wanted.map((r) => r.label).join(', ')}`;
}

// --------------------------------------------------------------- entry gate

const DISMISS_KEY = 'unipi.setup.notNow';
/** How long "Not now" lasts before the full-screen takeover comes back. */
export const DISMISS_DAYS = 14;

function storage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

/** The Unix day "Not now" was last pressed, or 0. */
export function dismissedAt(): number {
  const raw = storage()?.getItem(DISMISS_KEY);
  const value = raw ? Number.parseInt(raw, 10) : 0;
  return Number.isFinite(value) ? value : 0;
}

/** "Not now": remember it, so the takeover stops appearing on every launch. */
export function dismissSetup(now = Date.now()): void {
  storage()?.setItem(DISMISS_KEY, String(now));
}

export function forgetSetupDismissal(): void {
  storage()?.removeItem(DISMISS_KEY);
}

/**
 * Should the full-screen `#/setup` takeover appear?
 *
 * Desktop only, `report.ready` false, and "Not now" not pressed recently
 * (DISMISS_DAYS). While it is dismissed a banner in Settings → This computer
 * stands in for it (the same information, without the takeover).
 */
export function setupGateOpen(report?: SetupReport, dismissed = dismissedAt(), now = Date.now(), desktop = true): boolean {
  if (!desktop || !report || report.ready) return false;
  if (!dismissed) return true;
  return now - dismissed > DISMISS_DAYS * 86_400_000;
}
