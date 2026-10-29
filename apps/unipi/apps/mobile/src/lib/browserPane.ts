/**
 * The browser beside the chat (UNI-296): pure rules + the small shared state
 * of the side pane (desktop) / sheet (phone).
 *
 * - `agentStrip` — what the "Agent is using the browser" strip shows.
 * - `takeover` — the Take over / Give back state machine (request → result,
 *   agent events from the PC in between).
 * - `stealthBadge` — CloakBrowser (stealth) or "Not stealthy — Chrome
 *   fallback: <reason>".
 * - `profileChoices` — the profile picker's rows.
 * - `browserPane` — open / width (desktop, remembered) and the sheet (phone).
 */
import { createRoot, createSignal } from 'solid-js';
import type { BrowserAgent, BrowserOpenData, BrowserProfilesData } from '@unipi/protocol';
import type { AppError } from '@unipi/protocol';

// ------------------------------------------------------------- agent strip

export type AgentStrip =
  | { kind: 'none' }
  /** An agent sends input right now: offer Take over. */
  | { kind: 'driving' }
  /** The user holds the browser: offer Give back. */
  | { kind: 'taken' };

export function agentStrip(agent: BrowserAgent | undefined): AgentStrip {
  if (!agent) return { kind: 'none' };
  if (agent.taken_over) return { kind: 'taken' };
  if (agent.driving) return { kind: 'driving' };
  return { kind: 'none' };
}

// ------------------------------------------------------------- take over

export type TakeoverState = {
  agent: BrowserAgent | undefined;
  /** A Take over / Give back request is on its way. */
  pending?: 'take' | 'give';
  /** The last request failed (plain words). */
  error?: string;
};

export type TakeoverAction =
  | { kind: 'agent'; agent: BrowserAgent | undefined }
  | { kind: 'request'; on: boolean }
  | { kind: 'ok'; agent: BrowserAgent }
  | { kind: 'failed'; error: AppError };

/** Plain words for a failed take-over request. */
export function takeoverError(error: AppError): string {
  if (error.code === 'E_SCHEMA' || error.code === 'E_UNSUPPORTED') return 'Update unipi-host on the PC to take over from the agent.';
  return error.message || "Couldn't reach the PC.";
}

export function takeover(state: TakeoverState, action: TakeoverAction): TakeoverState {
  switch (action.kind) {
    case 'agent':
      return { ...state, agent: action.agent };
    case 'request':
      return { ...state, pending: action.on ? 'take' : 'give', error: undefined };
    case 'ok':
      return { agent: action.agent };
    case 'failed':
      return { ...state, pending: undefined, error: takeoverError(action.error) };
  }
}

// ------------------------------------------------------------- stealth

export type StealthBadge = {
  tone: 'ok' | 'warn';
  label: string;
  /** The full sentence (a note line under the header when not stealthy). */
  note?: string;
  title: string;
};

/** The engine/stealth badge for what runs (undefined = an older PC that doesn't say). */
export function stealthBadge(open: Pick<BrowserOpenData, 'engine' | 'stealth' | 'stealth_reason' | 'stealth_note'> | undefined): StealthBadge | undefined {
  if (!open || open.stealth === undefined) return undefined;
  if (open.stealth && open.stealth_note) return { tone: 'warn', label: 'Less stealthy', note: open.stealth_note, title: open.stealth_note };
  if (open.stealth) return { tone: 'ok', label: 'Stealth', title: 'CloakBrowser: sites see an ordinary browser, not automation' };
  const reason = (open.stealth_reason ?? '').trim();
  if (/engine is set to chrome/i.test(reason)) {
    const note = 'Not stealthy — plain Chrome is set on the PC';
    return { tone: 'warn', label: 'Not stealthy', note, title: note };
  }
  const note = `Not stealthy — Chrome fallback: ${reason || 'CloakBrowser could not start'}`;
  return { tone: 'warn', label: 'Not stealthy', note, title: note };
}

// ------------------------------------------------------------- profiles

export type ProfileChoice = { name: string; current: boolean; running: boolean; hint?: string };

/** The picker's rows: the default first, then by name; `current` marked. */
export function profileChoices(data: BrowserProfilesData | undefined, current: string): ProfileChoice[] {
  const names = new Map<string, ProfileChoice>();
  for (const p of data?.profiles ?? []) {
    const sites = p.cookie_domains.length;
    names.set(p.name, {
      name: p.name,
      current: p.name === current,
      running: p.running,
      hint: p.running ? 'running' : sites ? `${sites} site${sites === 1 ? '' : 's'} signed in` : undefined,
    });
  }
  if (!names.has(current)) names.set(current, { name: current, current: true, running: false });
  const def = data?.default ?? 'default';
  return [...names.values()].sort((a, b) => (a.name === def ? -1 : b.name === def ? 1 : a.name.localeCompare(b.name)));
}

/** Can the picker switch at all (more than one profile known)? */
export function canPickProfile(data: BrowserProfilesData | undefined): boolean {
  return (data?.profiles.length ?? 0) > 1;
}

// ------------------------------------------------------------- pane state

export const PANE_MIN = 360;
/** The chat keeps at least this much beside the pane. */
export const PANE_CHAT_MIN = 380;
export const PANE_DEFAULT = 560;

/** Clamp the pane width inside an area of `area` px (the chat + pane). */
export function clampPaneWidth(width: number, area: number): number {
  const max = Math.max(PANE_MIN, area - PANE_CHAT_MIN);
  return Math.round(Math.min(Math.max(Number.isFinite(width) ? width : PANE_DEFAULT, PANE_MIN), max));
}

/**
 * Fold the chat list away while the pane is open? Yes when the area right of
 * the list (`viewport - listEnd`) can't hold the chat (`PANE_CHAT_MIN`) plus
 * the pane at its wanted width (at least `PANE_MIN`).
 */
export function foldsList(viewport: number, listEnd: number, wanted: number): boolean {
  return viewport - listEnd < PANE_CHAT_MIN + Math.max(PANE_MIN, Number.isFinite(wanted) ? wanted : PANE_DEFAULT);
}

const KEY = 'unipi.browserPane';
type Saved = { open?: boolean; width?: number; profiles?: Record<string, string> };

function read(): Saved {
  try {
    const parsed = JSON.parse(localStorage.getItem(KEY) ?? '{}') as unknown;
    return parsed && typeof parsed === 'object' ? (parsed as Saved) : {};
  } catch {
    return {};
  }
}

function createPane() {
  const saved = read();
  const [open, setOpenSignal] = createSignal(!!saved.open);
  const [width, setWidthSignal] = createSignal(saved.width ?? PANE_DEFAULT);
  const [profiles, setProfiles] = createSignal<Record<string, string>>(saved.profiles ?? {});
  const [sheet, setSheet] = createSignal(false);
  const save = () => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ open: open(), width: width(), profiles: profiles() }));
    } catch {
      // storage blocked: just not remembered
    }
  };
  return {
    /** Desktop: the pane beside the chat is open (remembered). */
    open,
    setOpen: (next: boolean) => {
      setOpenSignal(next);
      save();
    },
    /** Desktop: the pane's width in px (remembered; clamp with `clampPaneWidth` when drawing). */
    width,
    setWidth: (next: number) => {
      setWidthSignal(Math.round(next));
      save();
    },
    resetWidth: () => {
      setWidthSignal(PANE_DEFAULT);
      save();
    },
    /** Phone (or a narrow window): the full-height sheet (not remembered). */
    sheet,
    setSheet,
    /** The profile shown for a PC (remembered; default "default"). */
    profileFor: (hostId: string) => profiles()[hostId] ?? 'default',
    setProfileFor: (hostId: string, name: string) => {
      setProfiles({ ...profiles(), [hostId]: name });
      save();
    },
  };
}

let shared: ReturnType<typeof createPane> | undefined;
export function browserPane(): ReturnType<typeof createPane> {
  if (!shared) shared = createRoot(() => createPane());
  return shared;
}
/** Test hook. */
export function resetBrowserPane(): void {
  shared = undefined;
}

/** The chat's "Browser" action: the pane on the desktop shell, the sheet otherwise. */
export function toggleBrowser(desktop: boolean): void {
  const pane = browserPane();
  if (desktop) pane.setOpen(!pane.open());
  else pane.setSheet(!pane.sheet());
}
