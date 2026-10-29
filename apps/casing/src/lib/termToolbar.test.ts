import { describe, expect, it } from 'vitest';
import {
  ARROW_BYTES,
  ARROW_DRAG_THRESHOLD_PX,
  KEY_BYTES,
  ModifierState,
  Modifiers,
  arrowDirection,
  bracketedPaste,
  composeSend,
  csiModifier,
  ctrlByte,
  encodeKey,
  ArrowRepeat,
  ARROW_PAD_INITIAL_DELAY_MS,
  ARROW_PAD_REPEAT_MIN_MS,
  ARROW_PAD_REPEAT_MS,
  MODIFIER_DOUBLE_TAP_MS,
  kittyKeyBytes,
  kittyModifier,
  type DesktopKeyEvent,
} from './termToolbar';

function keyEvent(key: string, mods: Partial<DesktopKeyEvent> = {}): DesktopKeyEvent {
  return { key, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods };
}

describe('ctrl encoding', () => {
  it('maps letters to control codes', () => {
    expect(ctrlByte('c')).toBe('\x03');
    expect(ctrlByte('C')).toBe('\x03');
    expect(ctrlByte('a')).toBe('\x01');
    expect(ctrlByte('z')).toBe('\x1a');
  });

  it('maps the special keys', () => {
    expect(ctrlByte('@')).toBe('\x00');
    expect(ctrlByte(' ')).toBe('\x00');
    expect(ctrlByte('[')).toBe('\x1b');
    expect(ctrlByte('?')).toBe('\x7f');
    expect(ctrlByte('5')).toBeNull();
    expect(ctrlByte('ab')).toBeNull();
  });
});

describe('Modifiers (ctrl)', () => {
  it('one-shot applies to the next input only', () => {
    const mods = new Modifiers();
    expect(mods.tap('ctrl', 1000)).toBe('one-shot');
    expect(mods.isActive('ctrl')).toBe(true);
    expect(mods.consume('c')).toBe('\x03');
    expect(mods.isActive('ctrl')).toBe(false);
    expect(mods.consume('c')).toBe('c');
  });

  it('one-shot that hits an unsupported key passes through and disarms', () => {
    const mods = new Modifiers();
    mods.tap('ctrl', 0);
    expect(mods.consume('5')).toBe('5');
    expect(mods.isActive('ctrl')).toBe(false);
  });

  it('double-tap locks until re-tap', () => {
    const mods = new Modifiers();
    expect(mods.tap('ctrl', 0)).toBe('one-shot');
    expect(mods.tap('ctrl', 200)).toBe('locked');
    expect(mods.isLocked('ctrl')).toBe(true);
    // Locked persists across keys.
    expect(mods.consume('c')).toBe('\x03');
    expect(mods.consume('z')).toBe('\x1a');
    expect(mods.isLocked('ctrl')).toBe(true);
    // Re-tap unlocks.
    expect(mods.tap('ctrl', 400)).toBe('unlocked');
    expect(mods.isActive('ctrl')).toBe(false);
  });

  it('a fresh tap after an unlock starts a new one-shot that can lock', () => {
    const mods = new Modifiers();
    mods.tap('ctrl', 0);
    mods.tap('ctrl', 200);
    expect(mods.tap('ctrl', 600)).toBe('unlocked');
    expect(mods.tap('ctrl', 700)).toBe('one-shot');
    expect(mods.tap('ctrl', 900)).toBe('locked');
  });

  it('shift works the same way', () => {
    const mods = new Modifiers();
    mods.tap('shift', 0);
    expect(mods.isActive('shift')).toBe(true);
    expect(mods.consume('a')).toBe('A');
  });
});

describe('key bytes', () => {
  it('covers the static keys', () => {
    expect(KEY_BYTES.esc).toBe('\x1b');
    expect(KEY_BYTES.tab).toBe('\t');
    expect(KEY_BYTES.ctrl_c).toBe('\x03');
    expect(KEY_BYTES.enter).toBe('\r');
    expect(KEY_BYTES.pgup).toBe('\x1b[5~');
  });

  it('encodes arrows', () => {
    expect(ARROW_BYTES.up).toBe('\x1b[A');
    expect(ARROW_BYTES.down).toBe('\x1b[B');
    expect(ARROW_BYTES.right).toBe('\x1b[C');
    expect(ARROW_BYTES.left).toBe('\x1b[D');
  });
});

describe('arrow drag direction', () => {
  it('needs the threshold', () => {
    expect(arrowDirection(4, 0)).toBeUndefined();
    expect(arrowDirection(0, -(ARROW_DRAG_THRESHOLD_PX - 1))).toBeUndefined();
  });

  it('picks the dominant axis past the threshold', () => {
    expect(arrowDirection(ARROW_DRAG_THRESHOLD_PX + 1, 0)).toBe('right');
    expect(arrowDirection(-(ARROW_DRAG_THRESHOLD_PX + 5), 0)).toBe('left');
    expect(arrowDirection(0, ARROW_DRAG_THRESHOLD_PX + 1)).toBe('down');
    expect(arrowDirection(0, -(ARROW_DRAG_THRESHOLD_PX + 1))).toBe('up');
  });
});

describe('bracketed paste', () => {
  it('wraps text and appends Enter for sends', () => {
    expect(bracketedPaste('hi')).toBe('\x1b[200~hi\x1b[201~');
    expect(composeSend('hi')).toBe('\x1b[200~hi\x1b[201~\r');
    expect(composeSend('line1\nline2')).toBe('\x1b[200~line1\nline2\x1b[201~\r');
  });
});

describe('modifier encoding', () => {
  it('csi modifier parameter follows the shift/alt/ctrl bits', () => {
    expect(csiModifier({})).toBeUndefined();
    expect(csiModifier({ shift: true })).toBe(2);
    expect(csiModifier({ opt: true })).toBe(3);
    expect(csiModifier({ ctrl: true })).toBe(5);
    expect(csiModifier({ ctrl: true, shift: true })).toBe(6);
  });

  it('encodes navigation with modifiers', () => {
    expect(encodeKey('up', {})).toBe('\x1b[A');
    expect(encodeKey('up', { shift: true })).toBe('\x1b[1;2A');
    expect(encodeKey('down', { opt: true })).toBe('\x1b[1;3B');
    expect(encodeKey('right', { ctrl: true })).toBe('\x1b[1;5C');
    expect(encodeKey('left', { ctrl: true, shift: true })).toBe('\x1b[1;6D');
    expect(encodeKey('end', { shift: true })).toBe('\x1b[1;2F');
  });

  it('encodes tab variants and the opt meta prefix', () => {
    expect(encodeKey('tab', { shift: true })).toBe('\x1b[Z');
    expect(encodeKey('tab', { ctrl: true })).toBe('\x1b[27;5;9~');
    expect(encodeKey('tab', { opt: true })).toBe('\x1b[27;3;9~');
    expect(encodeKey('c', { opt: true })).toBe('\x1bc');
    expect(encodeKey('x', {})).toBe('x');
  });
});

describe('Modifiers.consume', () => {
  it('applies ctrl to letters and disarms one-shots', () => {
    const mods = new Modifiers();
    mods.tap('ctrl', 0);
    expect(mods.consume('c')).toBe('\x03');
    expect(mods.isActive('ctrl')).toBe(false);
  });

  it('locks persist across keys', () => {
    const mods = new Modifiers();
    mods.tap('ctrl', 0);
    mods.tap('ctrl', 100);
    expect(mods.consume('c')).toBe('\x03');
    expect(mods.consume('z')).toBe('\x1a');
    expect(mods.isLocked('ctrl')).toBe(true);
  });

  it('shift uppercases and opt adds the meta prefix', () => {
    const mods = new Modifiers();
    mods.tap('shift', 0);
    expect(mods.consume('a')).toBe('A');
    mods.tap('opt', 100);
    expect(mods.consume('x')).toBe('\x1bx');
  });

  it('ctrl+shift combines (modifier 6) on arrows typed from the keyboard', () => {
    const mods = new Modifiers();
    mods.tap('ctrl', 0);
    mods.tap('shift', 10);
    expect(mods.consume('up')).toBe('\x1b[1;6A');
  });
});

describe('modifier toggling (UNI-127)', () => {
  it('a second (slow) tap turns a one-shot off', () => {
    const mods = new Modifiers();
    expect(mods.tap('shift', 0)).toBe('one-shot');
    expect(mods.tap('shift', 2000)).toBe('unlocked');
    expect(mods.isActive('shift')).toBe(false);
    expect(mods.consume('a')).toBe('a');
  });

  it('double-tap locks, a tap while locked turns it off, the next tap is a fresh one-shot', () => {
    const mods = new Modifiers();
    mods.tap('opt', 0);
    expect(mods.tap('opt', MODIFIER_DOUBLE_TAP_MS - 1)).toBe('locked');
    expect(mods.tap('opt', 1000)).toBe('unlocked');
    expect(mods.isActive('opt')).toBe(false);
    expect(mods.tap('opt', 1100)).toBe('one-shot');
  });

  it('long-press locks', () => {
    const mods = new Modifiers();
    mods.lock('ctrl');
    expect(mods.isLocked('ctrl')).toBe(true);
    expect(mods.consume('c')).toBe('\x03');
    expect(mods.consume('d')).toBe('\x04');
    expect(mods.tap('ctrl', 5000)).toBe('unlocked');
  });

  it('applies to soft-keyboard text: first char consumes the one-shot', () => {
    const mods = new Modifiers();
    expect(mods.applyToText('ls')).toEqual({ bytes: 'ls', applied: false });
    mods.tap('ctrl', 0);
    expect(mods.applyToText('c')).toEqual({ bytes: '\x03', applied: true });
    expect(mods.isActive('ctrl')).toBe(false);
    mods.tap('opt', 10);
    expect(mods.applyToText('bx').bytes).toBe('\x1bbx');
    mods.tap('shift', 20);
    expect(mods.applyToText('\x7fa').bytes).toBe('\x7fA');
  });

  it('locked modifiers apply to every char', () => {
    const mods = new Modifiers();
    mods.lock('shift');
    expect(mods.applyToText('abc').bytes).toBe('ABC');
  });

  it('encodes Shift+Tab, Opt+b and Ctrl+arrow through consume', () => {
    const mods = new Modifiers();
    mods.tap('shift', 0);
    expect(mods.consume('tab')).toBe('\x1b[Z');
    mods.tap('opt', 0);
    expect(mods.consume('b')).toBe('\x1bb');
    mods.tap('ctrl', 0);
    expect(mods.consume('left')).toBe('\x1b[1;5D');
    expect(mods.anyActive()).toBe(false);
  });
});

describe('ArrowRepeat (hold-to-repeat arrow pad)', () => {
  it('a press alone, or a wobble inside the dead zone, sends nothing', () => {
    const pad = new ArrowRepeat();
    pad.down(100, 100);
    expect(pad.move(103, 104)).toBeUndefined();
    expect(pad.move(110, 92)).toBeUndefined();
    expect(pad.direction).toBeUndefined();
    expect(pad.nextDelay()).toBeUndefined();
  });

  it('engaging sends one arrow, then waits the initial delay, then repeats at a steady capped pace', () => {
    const pad = new ArrowRepeat(12, 8);
    pad.down(0, 0);
    expect(pad.move(0, -12)).toBe('up');
    // Further travel in the same direction does not send more.
    expect(pad.move(0, -60)).toBeUndefined();
    expect(pad.move(0, -200)).toBeUndefined();
    expect(pad.nextDelay()).toBe(ARROW_PAD_INITIAL_DELAY_MS);
    expect(pad.repeat()).toBe('up');
    expect(pad.nextDelay()).toBe(ARROW_PAD_REPEAT_MS);
    const delays: number[] = [];
    for (let i = 0; i < 40; i += 1) {
      pad.repeat();
      delays.push(pad.nextDelay()!);
    }
    // Slight acceleration, never faster than the floor.
    expect(delays[0]).toBeLessThan(ARROW_PAD_REPEAT_MS);
    expect(Math.min(...delays)).toBe(ARROW_PAD_REPEAT_MIN_MS);
    expect(ARROW_PAD_INITIAL_DELAY_MS).toBeGreaterThanOrEqual(300);
    expect(ARROW_PAD_REPEAT_MIN_MS).toBeGreaterThanOrEqual(60);
  });

  it('turning sends one arrow in the new direction and restarts the delay', () => {
    const pad = new ArrowRepeat(12, 8);
    pad.down(0, 0);
    expect(pad.move(20, 0)).toBe('right');
    pad.repeat();
    pad.repeat();
    // A small diagonal drift keeps the engaged axis.
    expect(pad.move(20, -22)).toBeUndefined();
    expect(pad.direction).toBe('right');
    // The other axis clearly wins → turn.
    expect(pad.move(10, -30)).toBe('up');
    expect(pad.nextDelay()).toBe(ARROW_PAD_INITIAL_DELAY_MS);
  });

  it('returning to the centre stops; leaving it again re-engages', () => {
    const pad = new ArrowRepeat(12, 8);
    pad.down(0, 0);
    expect(pad.move(0, 15)).toBe('down');
    expect(pad.move(0, 9)).toBeUndefined();
    expect(pad.direction).toBe('down');
    expect(pad.move(0, 4)).toBeUndefined();
    expect(pad.direction).toBeUndefined();
    expect(pad.repeat()).toBeUndefined();
    expect(pad.move(0, -14)).toBe('up');
  });

  it('up() resets', () => {
    const pad = new ArrowRepeat();
    pad.down(0, 0);
    pad.move(30, 0);
    pad.up();
    expect(pad.move(100, 0)).toBeUndefined();
    expect(pad.repeat()).toBeUndefined();
  });
});

describe('kittyKeyBytes (UNI-311: Shift/Ctrl+Enter, Ctrl+Shift+letter the PTY the way kitty sends them)', () => {
  it('modifier parameter is 1 + shift(1) + alt(2) + ctrl(4)', () => {
    expect(kittyModifier({})).toBe(1);
    expect(kittyModifier({ shift: true })).toBe(2);
    expect(kittyModifier({ opt: true })).toBe(3);
    expect(kittyModifier({ ctrl: true })).toBe(5);
    expect(kittyModifier({ shift: true, ctrl: true })).toBe(6);
    expect(kittyModifier({ shift: true, opt: true, ctrl: true })).toBe(8);
  });

  it('Shift+Enter -> CSI 13;2u (xterm alone cannot tell this apart from plain Enter)', () => {
    expect(kittyKeyBytes(keyEvent('Enter', { shiftKey: true }))).toBe('\x1b[13;2u');
  });

  it('Ctrl+Enter -> CSI 13;5u', () => {
    expect(kittyKeyBytes(keyEvent('Enter', { ctrlKey: true }))).toBe('\x1b[13;5u');
  });

  it('Ctrl+Shift+Enter -> CSI 13;6u', () => {
    expect(kittyKeyBytes(keyEvent('Enter', { ctrlKey: true, shiftKey: true }))).toBe('\x1b[13;6u');
  });

  it('plain Enter is left to xterm (returns null, no app encoding)', () => {
    expect(kittyKeyBytes(keyEvent('Enter'))).toBeNull();
  });

  it('Alt+Enter is left to xterm (it already sends ESC CR)', () => {
    expect(kittyKeyBytes(keyEvent('Enter', { altKey: true }))).toBeNull();
  });

  it('Ctrl+Shift+<letter> -> CSI <code>;6u, xterm computes nothing for this combo', () => {
    expect(kittyKeyBytes(keyEvent('k', { ctrlKey: true, shiftKey: true }))).toBe('\x1b[107;6u');
    expect(kittyKeyBytes(keyEvent('K', { ctrlKey: true, shiftKey: true }))).toBe('\x1b[107;6u');
    expect(kittyKeyBytes(keyEvent('A', { ctrlKey: true, shiftKey: true }))).toBe('\x1b[97;6u');
  });

  it('plain Ctrl+<letter> and Alt+<letter> are left to xterm (already correctly encoded)', () => {
    expect(kittyKeyBytes(keyEvent('c', { ctrlKey: true }))).toBeNull();
    expect(kittyKeyBytes(keyEvent('a', { altKey: true }))).toBeNull();
  });

  it('Shift+Tab is left to xterm (CSI Z, back-tab shells already understand)', () => {
    expect(kittyKeyBytes(keyEvent('Tab', { shiftKey: true }))).toBeNull();
  });

  it('Cmd combos are never sent to the PTY (OS/app shortcuts)', () => {
    expect(kittyKeyBytes(keyEvent('Enter', { metaKey: true, shiftKey: true }))).toBeNull();
  });

  it('an unrelated plain letter is left to xterm', () => {
    expect(kittyKeyBytes(keyEvent('a'))).toBeNull();
  });
});
