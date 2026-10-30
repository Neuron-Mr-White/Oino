/**
 * Extension-shortcut conflict resolution (WP-A18, G-7 + §4 / Z-94):
 *
 * Extension shortcuts arrive from `_oino/get_shortcuts` / the session's
 * `setShortcuts` ui request (`{id, defaultKeys, description, extensionId}`).
 * **No extension chord may shadow an app chord** — a collision is rebound:
 *
 * - the ruled alternates (§4): `unipi.subagents.foreground` → Alt+B
 *   (shell wins Cmd/Ctrl+B = sidebar), and `unipi.tray.open` is intercepted
 *   by the app (Shift+↓ opens Session modal → Running, never forwarded);
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
  'unipi.subagents.foreground': 'Alt+B',
  'unipi.tray.open': null,
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
export function resolveExtShortcuts(appChords: readonly string[], specs: readonly ExtShortcutSpec[], userChords: Readonly<Record<string, string>> = {}): ResolvedExt[] {
  const taken = new Set<string>(appChords);
  const out: ResolvedExt[] = [];
  for (const spec of specs) {
    const own = normalizeChord(spec.chord);
    if (!own) continue;
    const extId = spec.extId ?? 'extension';
    const title = spec.title?.trim() || spec.id;
    const user = userChords[`ext:${spec.id}`] !== undefined ? normalizeChord(userChords[`ext:${spec.id}`]!) : undefined;

    // Ruled: the app intercepts this id (never forwarded to the extension).
    if (spec.id in EXT_ALTERNATES && EXT_ALTERNATES[spec.id] === null) {
      out.push({ id: spec.id, chord: own, title, extId, interceptedBy: 'chat.running' });
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
