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
import type { BrowserAgent, BrowserElement, BrowserOpenData, BrowserProfilesData } from '@unipi/protocol';
import { fitRect } from './browser';
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

// ------------------------------------------------------------- follow the agent

/** A tab the user opened here within this long is the user's, not the agent's. */
export const USER_TAB_WINDOW_MS = 3000;

/**
 * WP-B8: which tab to show after the tab list changed. A tab that appeared while an
 * agent is connected to the browser — and that the user did not just open here — is
 * the agent's (`browser_navigate` opens one in the background): show it, so the pane
 * follows the agent's work. `undefined` = stay. The first list (nothing known yet)
 * never jumps.
 */
export function followTab(
  known: ReadonlySet<string>,
  tabs: ReadonlyArray<{ id: string }>,
  active: string,
  agent: BrowserAgent | undefined,
  sinceUserTab: number,
): string | undefined {
  if (known.size === 0 || !agent?.connected || agent.taken_over) return undefined;
  if (sinceUserTab < USER_TAB_WINDOW_MS) return undefined;
  const fresh = tabs.filter((t) => !known.has(t.id));
  const last = fresh[fresh.length - 1];
  return last && last.id !== active ? last.id : undefined;
}

// ------------------------------------------------------------- element picker

/** A picture handed to the chat (base64, no `data:` prefix). */
export type BrowserShot = { mime: string; data: string; width: number; height: number };
/** WP-B8: what the element picker hands to the chat. */
export type BrowserPick = { element: BrowserElement; url: string; title: string; shot?: BrowserShot };

type Size = { width: number; height: number };
type Rect = { x: number; y: number; width: number; height: number };

/** Where an element box (frame CSS px) sits over the letterboxed canvas (box px). */
export function overlayRect(el: Rect, box: Size, frame: Size): Rect {
  if (!frame.width || !frame.height) return { x: 0, y: 0, width: 0, height: 0 };
  const fit = fitRect(box.width, box.height, frame.width, frame.height);
  const k = fit.width / frame.width;
  const x0 = Math.max(fit.x, fit.x + el.x * k);
  const y0 = Math.max(fit.y, fit.y + el.y * k);
  const x1 = Math.min(fit.x + fit.width, fit.x + (el.x + el.width) * k);
  const y1 = Math.min(fit.y + fit.height, fit.y + (el.y + el.height) * k);
  return { x: x0, y: y0, width: Math.max(0, x1 - x0), height: Math.max(0, y1 - y0) };
}

/** Longest side of a picture handed to the chat. */
export const SHOT_MAX_SIDE = 1600;

/**
 * The source rectangle (image px) of a crop of the last frame — `rect` in frame CSS px,
 * none = the whole frame — and its output size (≤ {@link SHOT_MAX_SIDE}). `undefined`
 * when the crop is empty (element off screen).
 */
export function cropRect(rect: Rect | undefined, frame: Size, image: Size): { sx: number; sy: number; sw: number; sh: number; outWidth: number; outHeight: number } | undefined {
  if (!frame.width || !frame.height || !image.width || !image.height) return undefined;
  const kx = image.width / frame.width;
  const ky = image.height / frame.height;
  const r = rect ?? { x: 0, y: 0, width: frame.width, height: frame.height };
  const pad = rect ? 4 : 0;
  const x0 = Math.max(0, Math.floor((r.x - pad) * kx));
  const y0 = Math.max(0, Math.floor((r.y - pad) * ky));
  const x1 = Math.min(image.width, Math.ceil((r.x + r.width + pad) * kx));
  const y1 = Math.min(image.height, Math.ceil((r.y + r.height + pad) * ky));
  const sw = x1 - x0;
  const sh = y1 - y0;
  if (sw < 2 || sh < 2) return undefined;
  const down = Math.min(1, SHOT_MAX_SIDE / Math.max(sw, sh));
  return { sx: x0, sy: y0, sw, sh, outWidth: Math.max(1, Math.round(sw * down)), outHeight: Math.max(1, Math.round(sh * down)) };
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
  if (error.code === 'E_SCHEMA' || error.code === 'E_UNSUPPORTED') return 'Update Unipi on the PC to take over from the agent.';
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
