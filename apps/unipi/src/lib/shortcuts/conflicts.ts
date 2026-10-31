/**
 * Extension-shortcut conflict resolution (WP-A18, G-7 + §4 / Z-94):
 *
 * Extension shortcuts arrive from `_oino/get_shortcuts` / the session's
 * `setShortcuts` ui request (`{id, defaultKeys, description, extensionId}`).
 * **No extension chord may shadow an app chord** — a collision is rebound:
 *
 * - the ruled interceptions (§4, P0-10): `unipi.subagents.foreground` is
 *   handled by the app's sidebar chord (Cmd/Ctrl+B backgrounds a foreground
 *   subagent, else toggles the sidebar — `subagents.ts`), and
 *   `unipi.tray.open` by Session modal → Running (Shift+↓, never forwarded);
 * - any other collision gets the first free app-chosen alternate
 *   (`Alt+<key>`, then `Alt+Shift+<key>`);
 * - a user-rebound extension chord wins over the automatic alternate (and is
 *   itself dropped if it lands on an app chord).
 *
 * Pure — tests drive it directly; `registry.extEntries()` is its caller.
 */
import { chordsCollide, normalizeChord } from './registry';

/** The ruled rebindings (§4). `null` = the app intercepts it natively. */
export const EXT_ALTERNATES: Record<string, string | null> = {
  'unipi.subagents.foreground': null,
  'unipi.tray.open': null,
};

/** Who intercepts an id (the app binding whose chord it shares) and the note Ctrl+/ shows. */
export const EXT_INTERCEPTS: Record<string, { by: string; note: string; userMayBind?: boolean }> = {
  // A user chord of its own (Settings) binds it directly as well.
  'unipi.subagents.foreground': { by: 'shell.sidebar', note: 'while a subagent runs in the foreground; else the sidebar', userMayBind: true },
  'unipi.tray.open': { by: 'chat.running', note: 'the app opens Session · Running instead' },
};

export type ExtShortcutSpec = {
  id: string;
  /** pi key syntax, e.g. `"alt+m"`, `"ctrl+b"`, `"shift+down"`. */
  chord: string;
  title?: string;
  extId?: string;
};

export type ResolvedExt = {
  id: string;
  /** Canonical chord (`Alt+M`); empty = no free chord left (shown unbound in Ctrl+/). */
  chord: string;
  title: string;
  extId: string;
  /** The extension's own chord when it was rebound. */
  reboundFrom?: string;
  /** The app handles this id natively (shown in Ctrl+/ with its note). */
  interceptedBy?: string;
  /** Why/when (Ctrl+/ and Settings show it next to the chord). */
  note?: string;
};

/** The first free alternate chord for a colliding extension chord. */
function alternate(chord: string, busy: (candidate: string) => boolean): string | undefined {
  const token = chord.split('+').at(-1)!;
  for (const candidate of [`Alt+${token}`, `Alt+Shift+${token}`]) if (!busy(candidate)) return candidate;
  return undefined;
}

/**
 * Resolves every extension shortcut against the app's effective chords.
 * Deterministic: first-come for duplicates, spec order otherwise.
 */
export function resolveExtShortcuts(
  appChords: readonly string[],
  specs: readonly ExtShortcutSpec[],
  userChords: Readonly<Record<string, string>> = {},
  /** The effective chord of an app binding (an intercepted id shows its interceptor's chord). */
  appChordOf: (id: string) => string | undefined = () => undefined,
): ResolvedExt[] {
  const taken = new Set<string>(appChords);
  const out: ResolvedExt[] = [];
  for (const spec of specs) {
    const own = normalizeChord(spec.chord);
    if (!own) continue;
    const extId = spec.extId ?? 'extension';
    const title = spec.title?.trim() || spec.id;
    const user = userChords[`ext:${spec.id}`] !== undefined ? normalizeChord(userChords[`ext:${spec.id}`]!) : undefined;

    // Ruled: the app intercepts this id (never forwarded to the extension).
    const userOwn = !!user && !!EXT_INTERCEPTS[spec.id]?.userMayBind && ![...taken].some((t) => chordsCollide(t, user));
    if (spec.id in EXT_ALTERNATES && EXT_ALTERNATES[spec.id] === null && !userOwn) {
      const by = EXT_INTERCEPTS[spec.id] ?? { by: 'chat.running', note: '' };
      out.push({ id: spec.id, chord: appChordOf(by.by) ?? own, title, extId, interceptedBy: by.by, ...(by.note ? { note: by.note } : {}) });
      continue;
    }
    const ruled = EXT_ALTERNATES[spec.id];
    const collide = (chord: string) => [...taken].some((t) => chordsCollide(t, chord)) || out.some((e) => !e.interceptedBy && chordsCollide(e.chord, chord));
    const busy = (chord: string) => collide(chord) || chordsCollide(chord, own);

    let chord: string;
    let reboundFrom: string | undefined;
    if (user && !collide(user)) {
      chord = user;
    } else if (typeof ruled === 'string' && !collide(ruled)) {
      chord = ruled;
      if (chord !== own) reboundFrom = own;
    } else if (!collide(own)) {
      chord = own;
    } else {
      const alt = alternate(own, busy) ?? alternate(own, collide);
      // No free chord: the shortcut stays unbound (never shadow an app chord).
      chord = alt ?? '';
      reboundFrom = own;
    }
    if (chord) taken.add(chord);
    out.push({ id: spec.id, chord, title, extId, ...(reboundFrom ? { reboundFrom } : {}) });
  }
  return out;
}
