/**
 * Toolbar key encoding and the Ctrl one-shot/lock state machine
 * (DESIGN.md §3.3 toolbar capsule). Pure: the component owns timers
 * and pointers, these functions own the bytes.
 */

/** Bracketed paste wrapper (xterm-style, understood by shells and TUIs). */
export const PASTE_START = '\x1b[200~';
export const PASTE_END = '\x1b[201~';

export function bracketedPaste(text: string): string {
  return `${PASTE_START}${text}${PASTE_END}`;
}

/** Composes a composer send: bracketed paste, then Enter. */
export function composeSend(text: string): string {
  return `${bracketedPaste(text)}\r`;
}

/** Control-code for a printable key with Ctrl held, if supported. */
export function ctrlByte(data: string): string | null {
  if (data.length !== 1) return null;
  const lower = data.toLowerCase();
  if (lower >= 'a' && lower <= 'z') {
    return String.fromCharCode(lower.charCodeAt(0) - 96);
  }
  const specials: Record<string, string> = {
    '@': '\x00',
    ' ': '\x00',
    '[': '\x1b',
    '\\': '\x1c',
    ']': '\x1d',
    '^': '\x1e',
    '_': '\x1f',
    '?': '\x7f',
  };
  return specials[data] ?? null;
}

/** Which keyboard modifiers are armed for the next key. */
export type Mods = { ctrl?: boolean; shift?: boolean; opt?: boolean };

/** CSI modifier parameter: 1 + shift(1) + alt(2) + ctrl(4). */
export function csiModifier(mods: Mods): number | undefined {
  const value = (mods.shift ? 1 : 0) + (mods.opt ? 2 : 0) + (mods.ctrl ? 4 : 0);
  return value > 0 ? value + 1 : undefined;
}

/**
 * Encodes one keystroke with the armed modifiers: Ctrl maps letters
 * to control codes, Shift uppercases letters and uses shifted CSI
 * (\x1b[1;2A, \x1b[Z for Tab), Opt is Meta (ESC prefix, \x1b[1;3A),
 * combinations combine (Ctrl+Shift = \x1b[1;6A).
 */
export function encodeKey(data: string, mods: Mods): string {
  const mod = csiModifier(mods);
  const lower = data.toLowerCase();

  // Navigation keys: CSI with the modifier parameter when armed.
  const nav: Record<string, [string, string]> = {
    up: ['A', '\x1b[A'],
    down: ['B', '\x1b[B'],
    right: ['C', '\x1b[C'],
    left: ['D', '\x1b[D'],
    home: ['H', '\x1b[H'],
    end: ['F', '\x1b[F'],
  };
  if (data in nav) {
    const [letter, plain] = nav[data];
    return mod ? `\x1b[1;${mod}${letter}` : plain;
  }

  if (data === 'tab') {
    if (mods.shift) return '\x1b[Z';
    if (mods.ctrl) return '\x1b[27;5;9~';
    if (mods.opt) return '\x1b[27;3;9~';
    return '\t';
  }
  if (data === 'pgup') return mod ? `\x1b[5;${mod}~` : '\x1b[5~';
  if (data === 'pgdn') return mod ? `\x1b[6;${mod}~` : '\x1b[6~';

  // Printable keys: Ctrl maps to control codes, Shift uppercases,
  // Opt is Meta (ESC prefix).
  if (mods.ctrl) {
    const control = ctrlByte(data);
    if (control !== null) return control;
  }
  const base = mods.shift ? data.toUpperCase() : data;
  return mods.opt ? `\x1b${base}` : base;
}

/** Keydown → bytes for hardware non-printable keys (arrows, Esc,
 * Tab, Enter, Backspace, Ctrl/Alt combos). Printable keydowns return
 * empty: the IME path (lib/ime) owns them. */
export function imeKeyBytes(key: string, ctrl: boolean, alt: boolean): string {
  const nav: Record<string, [string, string]> = {
    ArrowUp: ['A', '\x1b[A'],
    ArrowDown: ['B', '\x1b[B'],
    ArrowRight: ['C', '\x1b[C'],
    ArrowLeft: ['D', '\x1b[D'],
    Home: ['H', '\x1b[H'],
    End: ['F', '\x1b[F'],
    PageUp: ['5~', '\x1b[5~'],
    PageDown: ['6~', '\x1b[6~'],
  };
  if (key in nav) {
    const [suffix, plain] = nav[key];
    const mod = (ctrl ? 5 : 0) + (alt ? 2 : 0);
    if (mod) {
      const param = 1 + mod;
      return suffix.includes('~') ? `\x1b[${param};${mod}${suffix}` : `\x1b[1;${param}${suffix}`;
    }
    if (alt) return `\x1b${plain}`;
    return plain;
  }
  if (key === 'Escape') return '\x1b';
  if (key === 'Tab') return ctrl ? '\x1b[27;5;9~' : '\t';
  if (key === 'Enter') return '\r';
  if (key === 'Backspace') return '\x7f';
  return '';
}

/**
 * Desktop hardware keys xterm.js cannot represent, or silently eats
 * (DESIGN §5.8, UNI-311). xterm's own `evaluateKeyboardEvent` sends
 * plain `\r` for Enter regardless of Shift/Ctrl (so Shift+Enter and
 * Ctrl+Enter are indistinguishable from a bare Enter), and its
 * default branch for Ctrl+Shift+<letter> computes no key at all (the
 * keystroke vanishes). Both cases are encoded here as CSI-u — the
 * format kitty, VS Code, Alacritty, WezTerm and foot all emit for
 * exactly these otherwise-unrepresentable combinations — and nothing
 * else: a plain Enter/Tab, Alt+Enter (xterm already sends ESC CR),
 * Alt+letter (xterm already sends ESC+letter), Shift+Tab (xterm
 * already sends CSI Z, understood by shells as back-tab) and
 * ordinary Ctrl+letter all keep xterm's own, already-correct
 * handling by returning null here — changing a working legacy
 * encoding (that bash/readline rely on, e.g. back-tab) to CSI-u would
 * break completion in shells that were never asked to understand it.
 *
 * `kittyProtocol` is honoured when the host asked for the real kitty
 * keyboard protocol (CSI > 1 u / `decRequestKittyKeyboardProtocol`):
 * it widens the mapping to ordinary Ctrl+letter and Alt+letter too,
 * which xterm's legacy path already encodes reasonably, so there is
 * nothing extra to do — the flag exists for callers to skip this
 * function's narrower legacy fallback entirely once they track that
 * mode themselves.
 */
export type DesktopKeyEvent = {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
};

/** CSI-u modifier parameter (always present, 1 = no modifier). */
export function kittyModifier(mods: Mods): number {
  return 1 + (mods.shift ? 1 : 0) + (mods.opt ? 2 : 0) + (mods.ctrl ? 4 : 0);
}

export function kittyKeyBytes(e: DesktopKeyEvent): string | null {
  if (e.metaKey) return null; // Cmd combos are OS/app shortcuts, not PTY input.
  const mods: Mods = { shift: e.shiftKey, opt: e.altKey, ctrl: e.ctrlKey };
  const any = mods.shift || mods.opt || mods.ctrl;
  if (e.key === 'Enter') {
    if (!any) return null; // plain Enter: xterm's own \r.
    if (mods.opt && !mods.shift && !mods.ctrl) return null; // xterm: ESC CR already.
    return `\x1b[13;${kittyModifier(mods)}u`;
  }
  // Ctrl+Shift+<letter>: xterm computes no key at all for this combo.
  // kitty convention: the unicode-key-code is the base (lowercase)
  // letter; Shift lives in the modifier parameter, not the letter case.
  if (mods.ctrl && mods.shift && !mods.opt && e.key.length === 1 && /[a-zA-Z]/.test(e.key)) {
    const code = e.key.toLowerCase().charCodeAt(0);
    return `\x1b[${code};${kittyModifier(mods)}u`;
  }
  return null;
}

/** Arrow escapes for the drag-arrows key. */
export const ARROW_BYTES = {
  up: '\x1b[A',
  down: '\x1b[B',
  right: '\x1b[C',
  left: '\x1b[D',
} as const;
export type ArrowDirection = keyof typeof ARROW_BYTES;

/** Static key bytes for the non-Ctrl toolbar keys. */
export const KEY_BYTES: Record<string, string> = {
  esc: '\x1b',
  tab: '\t',
  ctrl_c: '\x03',
  enter: '\r',
  backspace: '\x7f',
  home: '\x1b[H',
  end: '\x1b[F',
  pgup: '\x1b[5~',
  pgdn: '\x1b[6~',
};

/** Drag-arrows tuning (DESIGN.md §3.3). */
export const ARROW_DRAG_THRESHOLD_PX = 12;
export const ARROW_REPEAT_MS = 120;

/** Direction for a drag delta past the threshold, else undefined. */
export function arrowDirection(deltaX: number, deltaY: number): ArrowDirection | undefined {
  if (Math.abs(deltaX) < ARROW_DRAG_THRESHOLD_PX && Math.abs(deltaY) < ARROW_DRAG_THRESHOLD_PX) {
    return undefined;
  }
  return Math.abs(deltaY) > Math.abs(deltaX)
    ? deltaY < 0
      ? 'up'
      : 'down'
    : deltaX > 0
      ? 'right'
      : 'left';
}

/**
 * Arrow drag pad tuning (UNI-128: key-repeat semantics, replacing the
 * UNI-127 distance stepper that fired an arrow every 18 px and flooded
 * the PTY on a long drag).
 */
/** Finger offset from the press point before a direction engages. */
export const ARROW_PAD_DEADZONE_PX = 12;
/** Offset under which an engaged direction releases (hysteresis). */
export const ARROW_PAD_RELEASE_PX = 8;
/** Hold time after the first arrow before auto-repeat starts. */
export const ARROW_PAD_INITIAL_DELAY_MS = 400;
/** First auto-repeat interval, then slightly faster down to the floor. */
export const ARROW_PAD_REPEAT_MS = 100;
export const ARROW_PAD_REPEAT_MIN_MS = 70;
export const ARROW_PAD_REPEAT_ACCEL_MS = 3;
/** A new axis must beat the engaged one by this factor to take over,
 * so a diagonal wobble doesn't flip-flop between two directions. */
export const ARROW_PAD_TURN_RATIO = 1.25;

/**
 * Arrow drag pad as a hold-to-repeat key (pure; the component owns the
 * timer). A press alone sends nothing. Moving the finger past the
 * dead zone engages the dominant direction and sends ONE arrow at
 * once; holding it there auto-repeats after
 * `ARROW_PAD_INITIAL_DELAY_MS`, every `ARROW_PAD_REPEAT_MS` easing to
 * `ARROW_PAD_REPEAT_MIN_MS`. The direction always follows the
 * finger's offset from the press point: turning sends one arrow in the
 * new direction and restarts the delay; coming back to the centre
 * stops; lifting stops.
 */
export class ArrowRepeat {
  private origin: { x: number; y: number } | undefined;
  private repeats = 0;
  direction: ArrowDirection | undefined;

  constructor(
    private readonly deadzonePx = ARROW_PAD_DEADZONE_PX,
    private readonly releasePx = ARROW_PAD_RELEASE_PX,
  ) {}

  down(x: number, y: number): void {
    this.origin = { x, y };
    this.direction = undefined;
    this.repeats = 0;
  }

  /**
   * Feeds a pointer position. Returns the arrow to send *now* when a
   * direction engages or changes (the caller then (re)starts its timer
   * with `nextDelay()`), else undefined. Check `direction` afterwards:
   * undefined means the finger is back in the centre (stop the timer).
   */
  move(x: number, y: number): ArrowDirection | undefined {
    if (!this.origin) return undefined;
    const dx = x - this.origin.x;
    const dy = y - this.origin.y;
    const ax = Math.abs(dx);
    const ay = Math.abs(dy);
    const reach = Math.max(ax, ay);
    const current = this.direction;
    if (current) {
      if (reach < this.releasePx) {
        this.direction = undefined;
        this.repeats = 0;
        return undefined;
      }
    } else if (reach < this.deadzonePx) {
      return undefined;
    }
    let vertical = ay > ax;
    if (current) {
      const currentVertical = current === 'up' || current === 'down';
      const along = currentVertical ? ay : ax;
      const cross = currentVertical ? ax : ay;
      // Stay on the engaged axis unless the other one clearly wins.
      vertical = cross > along * ARROW_PAD_TURN_RATIO ? !currentVertical : currentVertical;
    }
    const next: ArrowDirection = vertical ? (dy < 0 ? 'up' : 'down') : dx > 0 ? 'right' : 'left';
    if (next === current) return undefined;
    this.direction = next;
    this.repeats = 0;
    return next;
  }

  /** Delay before the next auto-repeat, or undefined when idle. */
  nextDelay(): number | undefined {
    if (!this.direction) return undefined;
    if (this.repeats === 0) return ARROW_PAD_INITIAL_DELAY_MS;
    return Math.max(
      ARROW_PAD_REPEAT_MIN_MS,
      ARROW_PAD_REPEAT_MS - (this.repeats - 1) * ARROW_PAD_REPEAT_ACCEL_MS,
    );
  }

  /** Timer fired: the arrow to send (and counts it), or undefined. */
  repeat(): ArrowDirection | undefined {
    if (!this.direction) return undefined;
    this.repeats += 1;
    return this.direction;
  }

  up(): void {
    this.origin = undefined;
    this.direction = undefined;
    this.repeats = 0;
  }
}

export const MODIFIABLE_KEYS: Record<string, string> = {
  esc: '\x1b',
  tab: 'tab',
  home: 'home',
  end: 'end',
  pgup: 'pgup',
  pgdn: 'pgdn',
};

/** Hardware/IME keydown names → encoder key names. */
export const KEYDOWN_TO_KEY: Record<string, string> = {
  ArrowUp: 'up',
  ArrowDown: 'down',
  ArrowLeft: 'left',
  ArrowRight: 'right',
  Home: 'home',
  End: 'end',
  PageUp: 'pgup',
  PageDown: 'pgdn',
  Tab: 'tab',
  Escape: '\x1b',
};

export type ModifierKind = 'ctrl' | 'shift' | 'opt';

/** Two taps within this window lock the modifier. */
export const MODIFIER_DOUBLE_TAP_MS = 350;

/** Modifier key state (Ctrl/Shift/Opt):
 * - off → tap → one-shot (applies to the next key, then turns off)
 * - one-shot → tap again → off; a quick second tap (double-tap) → locked
 * - long-press → locked
 * - locked → tap → off */
export class ModifierState {
  private pending = false;
  private locked = false;
  private lastTap = Number.NEGATIVE_INFINITY;

  constructor(public readonly kind: ModifierKind) {}

  /** A tap on the key. */
  tap(now: number): 'one-shot' | 'locked' | 'unlocked' {
    const quick = now - this.lastTap <= MODIFIER_DOUBLE_TAP_MS;
    this.lastTap = now;
    if (this.locked) {
      this.locked = false;
      this.pending = false;
      this.lastTap = Number.NEGATIVE_INFINITY;
      return 'unlocked';
    }
    if (this.pending) {
      this.pending = false;
      if (quick) {
        this.locked = true;
        return 'locked';
      }
      return 'unlocked';
    }
    this.pending = true;
    return 'one-shot';
  }

  /** Long-press: lock straight away. */
  lock(): void {
    this.pending = false;
    this.locked = true;
    this.lastTap = Number.NEGATIVE_INFINITY;
  }

  /** Whether the modifier applies to the next key. */
  isActive(): boolean {
    return this.pending || this.locked;
  }

  isLocked(): boolean {
    return this.locked;
  }

  /** Disarms a one-shot (a lock stays). */
  disarm(): void {
    this.pending = false;
  }
}

/** The three modifier keys together; encodes soft-keyboard input with
 * the armed combination (Ctrl+Shift = modifier 6, Opt = Meta, ...). */
export class Modifiers {
  private readonly states: Record<ModifierKind, ModifierState> = {
    ctrl: new ModifierState('ctrl'),
    shift: new ModifierState('shift'),
    opt: new ModifierState('opt'),
  };

  get(kind: ModifierKind): ModifierState {
    return this.states[kind];
  }

  tap(kind: ModifierKind, now: number): 'one-shot' | 'locked' | 'unlocked' {
    return this.states[kind].tap(now);
  }

  isActive(kind: ModifierKind): boolean {
    return this.states[kind].isActive();
  }

  isLocked(kind: ModifierKind): boolean {
    return this.states[kind].isLocked();
  }

  anyActive(): boolean {
    return this.isActive('ctrl') || this.isActive('shift') || this.isActive('opt');
  }

  lock(kind: ModifierKind): void {
    this.states[kind].lock();
  }

  /** Armed modifier set, without consuming. */
  armed(): Mods {
    return {
      ctrl: this.states.ctrl.isActive(),
      shift: this.states.shift.isActive(),
      opt: this.states.opt.isActive(),
    };
  }

  /**
   * Applies the armed modifiers to soft-keyboard text (the IME diff).
   * Backspaces (`\x7f`) pass through untouched. The first character
   * consumes the one-shots; later characters only get the locked
   * modifiers. Returns the text unchanged when nothing is armed.
   */
  applyToText(text: string): { bytes: string; applied: boolean } {
    if (!this.anyActive()) return { bytes: text, applied: false };
    let out = '';
    let applied = false;
    for (const char of Array.from(text)) {
      if (char === '\x7f') {
        out += char;
        continue;
      }
      if (this.anyActive()) {
        out += this.consume(char);
        applied = true;
      } else {
        out += char;
      }
    }
    return { bytes: out, applied };
  }

  /** Consumes terminal input, applying the armed modifiers; one-shots
   * disarm after the next key (locks persist). */
  consume(data: string): string {
    const mods: Mods = {
      ctrl: this.states.ctrl.isActive(),
      shift: this.states.shift.isActive(),
      opt: this.states.opt.isActive(),
    };
    for (const kind of ['ctrl', 'shift', 'opt'] as ModifierKind[]) {
      if (this.states[kind].isActive() && !this.states[kind].isLocked()) {
        this.states[kind].disarm();
      }
    }
    return encodeKey(data, mods);
  }
}
