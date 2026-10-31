/**
 * Shortcut registry (app spec §3.3 "shortcuts", WP-A18; Z-94 + G-7):
 *
 * - **Chords** are normalized strings — `Mod+K`, `Mod+Shift+F`, `Alt+M`,
 *   `Ctrl+Alt+C`, `Shift+Down`, `Mod+,` — where `Mod` is the platform
 *   primary modifier (⌘ on macOS, Ctrl elsewhere). `Ctrl` in a chord is the
 *   literal Ctrl key (macOS keeps it distinct from ⌘).
 * - **App bindings** (`defaults.ts`) are the rebindable shell map; the user's
 *   chords persist in app settings (`shortcuts.chords`, id → chord).
 * - **Extension shortcuts** (`_oino/get_shortcuts` / the `setShortcuts` ui
 *   request) are resolved by `conflicts.ts`: none may shadow an app chord —
 *   collisions are rebound to an app-chosen alternate (§4) and the result is
 *   what Ctrl+/ lists and Settings shows.
 *
 * Key events are matched through `bindingForEvent` (user chord → default →
 * resolved extension chord), so rebinding works everywhere at once.
 */
import { batch, createSignal } from 'solid-js';
import { APP_BINDINGS, SCREEN_KEYS, type AppBinding, type ScreenKeyGroup } from './defaults';
import { resolveExtShortcuts, type ExtShortcutSpec, type ResolvedExt } from './conflicts';

export type Keyish = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>;

/** `code` → chord key token (letters/digits pass through as their letter). */
const CODE_KEYS: Record<string, string> = {
  Slash: '/',
  Comma: ',',
  Period: '.',
  BracketLeft: '[',
  BracketRight: ']',
  Semicolon: ';',
  Quote: "'",
  Minus: '-',
  Equal: '=',
  Backslash: '\\',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  Escape: 'Esc',
  Enter: 'Enter',
  Backspace: 'Backspace',
  Delete: 'Delete',
  Tab: 'Tab',
  Home: 'Home',
  End: 'End',
  PageUp: 'PageUp',
  PageDown: 'PageDown',
  Space: 'Space',
};

/** The key token of an event (`undefined` when it is not bindable). */
const NAMED_KEYS = new Set(['Enter', 'Tab', 'Backspace', 'Delete', 'Home', 'End', 'PageUp', 'PageDown', 'Space', 'Insert']);
const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Ctrl', 'Alt', 'Meta', 'CapsLock', 'NumLock', 'ScrollLock', 'Dead', 'ContextMenu']);

export function keyToken(e: Keyish): string | undefined {
  if (e.code) {
    if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
    if (/^Digit\d$/.test(e.code)) return e.code.slice(5);
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(e.code)) return e.code;
    if (e.code === 'NumpadEnter') return 'Enter';
    if (e.code in CODE_KEYS) return CODE_KEYS[e.code]!;
  }
  const key = e.key;
  if (MODIFIER_KEYS.has(key)) return undefined;
  if (key === 'Escape') return 'Esc';
  if (key.startsWith('Arrow')) return key.slice(5);
  if (key.length === 1) return /^[a-z]$/.test(key) ? key.toUpperCase() : key;
  if (NAMED_KEYS.has(key) || /^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key === 'Space' ? 'Space' : key;
  return undefined;
}

/** The chord an event spells, in canonical form (`undefined` = not bindable). */
export function chordOf(e: Keyish): string | undefined {
  const token = keyToken(e);
  if (!token) return undefined;
  const parts: string[] = [];
  // Ctrl+⌘ together read as one primary press (Windows key chord leftovers).
  if (e.ctrlKey || e.metaKey) parts.push('Mod');
  if (e.altKey) parts.push('Alt');
  if (e.shiftKey) parts.push('Shift');
  parts.push(token);
  return parts.join('+');
}

/** Normalizes a written chord ("ctrl+shift+p", "Alt+M") to canonical form. */
export function normalizeChord(raw: string): string | undefined {
  const parts = raw.trim().toLowerCase().split('+').map((p) => p.trim()).filter(Boolean);
  if (!parts.length) return undefined;
  const key = parts.pop()!;
  const mods = new Set(parts);
  const out: string[] = [];
  // `cmd`/`meta`/`primary` read as Mod.
  const mod = mods.has('mod') || mods.has('cmd') || mods.has('meta') || mods.has('primary');
  const ctrl = mods.has('ctrl') || mods.has('control');
  if (mod || ctrl) out.push('Mod');
  if (mods.has('alt') || mods.has('opt') || mods.has('option')) out.push('Alt');
  if (mods.has('shift')) out.push('Shift');
  const token = CODE_KEYS[key] ?? (/^[a-z0-9]$/i.test(key) ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1));
  if (['Shift', 'Control', 'Ctrl', 'Alt', 'Meta', 'Mod'].includes(token)) return undefined;
  out.push(token);
  return out.join('+');
}

/** Does `chord` match the key event? (Chord parts must all be present.) */
export function chordMatches(chord: string, e: Keyish): boolean {
  const parts = chord.split('+');
  const token = parts.at(-1)!;
  if (keyToken(e) !== token) return false;
  const want = new Set(parts.slice(0, -1));
  if (want.has('Mod') !== (e.ctrlKey || e.metaKey)) return false;
  if (want.has('Alt') !== e.altKey) return false;
  if (want.has('Shift') !== e.shiftKey) return false;
  return true;
}

/** True when two chords would catch the same physical press on some platform. */
export function chordsCollide(a: string, b: string): boolean {
  const [am, aAlt, aShift, ak] = partsOf(a);
  const [bm, bAlt, bShift, bk] = partsOf(b);
  // `Ctrl+X` and `Mod+X` collide wherever Ctrl *is* the primary (Win/Linux).
  const modsCollide = am === bm || (am && bm);
  return modsCollide && aAlt === bAlt && aShift === bShift && ak === bk;
}

function partsOf(chord: string): [boolean, boolean, boolean, string] {
  const parts = chord.split('+');
  return [parts.includes('Mod'), parts.includes('Alt'), parts.includes('Shift'), parts.at(-1)!];
}

/** `⌘K` / `Ctrl+K` style tokens for `<kbd>` rendering. */
export function chordTokens(chord: string, mac = isMac()): string[] {
  return chord.split('+').map((p) => (p === 'Mod' ? (mac ? '⌘' : 'Ctrl') : p === 'Alt' ? (mac ? '⌥' : 'Alt') : p));
}

const ARROWS: Record<string, string> = { Up: '↑', Down: '↓', Left: '←', Right: '→' };

/** Tokens for an extension row (Ctrl+/, Settings): arrows as glyphs, `—` when unbound. */
export function extChordTokens(chord: string, mac = isMac()): string[] {
  return chord ? chordTokens(chord, mac).map((t) => ARROWS[t] ?? t) : ['—'];
}

export function displayChord(chord: string, mac = isMac()): string {
  return chordTokens(chord, mac).join(mac ? '' : '+');
}

export function isMac(): boolean {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}

// ── The registry ────────────────────────────────────────────────────────

export type AppEntry = AppBinding & { chord: string; chords: string[]; defaults: string[]; rebound: boolean };
export type ExtEntry = ResolvedExt & { chord: string };

/** How a key event resolved: an app binding or an extension shortcut. */
export type BindingHit =
  | { kind: 'app'; id: string }
  | { kind: 'ext'; id: string; extId: string; intercepted: boolean; interceptedBy?: string };

const [userChords, setUserChords] = createSignal<Readonly<Record<string, string>>>({});
const [extSpecs, setExtSpecs] = createSignal<readonly ExtShortcutSpec[]>([]);

/** Wired by the shell (inside `SettingsProvider`): reads/writes `shortcuts.chords`. */
export type ShortcutsPersistence = { read(): Record<string, string>; write(next: Record<string, string>): void };
let persistence: ShortcutsPersistence | undefined;

export function installShortcutsPersistence(p: ShortcutsPersistence | undefined): void {
  persistence = p;
  setUserChords(p ? { ...p.read() } : {});
}

/** Every app binding with its effective chord(s) (a user chord replaces the defaults). */
export function appEntries(): AppEntry[] {
  const users = userChords();
  return APP_BINDINGS.map((b) => {
    const user = users[b.id];
    const chord = user ?? b.chords[0]!;
    return { ...b, chord, chords: user ? [user] : b.chords, defaults: b.chords, rebound: user !== undefined };
  });
}

/** Extension shortcuts as resolved against the app chords (conflicts.ts). */
export function extEntries(): ExtEntry[] {
  const entries = appEntries();
  const app = entries.filter((e) => !e.displayOnly).flatMap((e) => e.chords);
  return resolveExtShortcuts(app, extSpecs(), userChords(), (id) => entries.find((e) => e.id === id)?.chord);
}

/** A user chord override; `null` resets to the default. */
export function rebindShortcut(id: string, chord: string | null): void {
  const next = { ...userChords() };
  if (chord === null) delete next[id];
  else next[id] = chord;
  setUserChords(next);
  persistence?.write(next);
}

/** Where a binding applies (`conversation` ones need a shown session). */
export function scopeOf(id: string): 'app' | 'conversation' | undefined {
  return APP_BINDINGS.find((b) => b.id === id)?.scope;
}

/** The app bindings table Ctrl+/ and Settings render (plus display-only screen keys). */
export function screenKeyGroups(): ScreenKeyGroup[] {
  return SCREEN_KEYS;
}

/** Which binding a key event means: user chords first, then defaults, then
 *  extension chords — an extension chord never shadows an app chord. */
export function bindingForEvent(e: Keyish): BindingHit | undefined {
  for (const entry of appEntries()) {
    if (entry.displayOnly) continue;
    for (const chord of entry.chords) if (chordMatches(chord, e)) return { kind: 'app', id: entry.id };
  }
  for (const entry of extEntries()) {
    if (entry.chord && chordMatches(entry.chord, e))
      return { kind: 'ext', id: entry.id, extId: entry.extId, intercepted: !!entry.interceptedBy, ...(entry.interceptedBy ? { interceptedBy: entry.interceptedBy } : {}) };
  }
  return undefined;
}

/** Feeds the session's extension shortcuts (`setShortcuts` ui payload). */
export function setExtensionShortcuts(specs: readonly ExtShortcutSpec[] | undefined): void {
  batch(() => setExtSpecs(specs ?? []));
}

/** Parses a `setShortcuts` / `_oino/get_shortcuts` payload (`{shortcuts: [{id,
 *  defaultKeys, description, extensionId}]}`) defensively. */
export function parseExtShortcutSpecs(value: unknown): ExtShortcutSpec[] {
  const list = Array.isArray(value)
    ? value
    : !!value && typeof value === 'object' && Array.isArray((value as { shortcuts?: unknown }).shortcuts)
      ? (value as { shortcuts: unknown[] }).shortcuts
      : [];
  const out: ExtShortcutSpec[] = [];
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue;
    const s = raw as Record<string, unknown>;
    if (typeof s.id !== 'string') continue;
    const keys = Array.isArray(s.defaultKeys) ? s.defaultKeys.filter((k): k is string => typeof k === 'string') : [];
    out.push({
      id: s.id,
      chord: keys[0] ?? '',
      ...(typeof s.description === 'string' && s.description ? { title: s.description } : {}),
      ...(typeof s.extensionId === 'string' ? { extId: s.extensionId } : {}),
    });
  }
  return out;
}

/** Test hook: drop all state. */
export function resetShortcuts(): void {
  batch(() => {
    setUserChords({});
    setExtSpecs([]);
    persistence = undefined;
  });
}
