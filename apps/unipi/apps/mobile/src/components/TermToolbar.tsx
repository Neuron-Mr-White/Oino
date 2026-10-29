/**
 * Toolbar capsule (DESIGN.md §3.3, Moshi pattern): one horizontally
 * scrolling row of compact keys with the composer/keyboard keys pinned
 * on the right outside the scroller. Keys come from
 * settings.input.toolbar in order.
 *
 * Mapping (Moshi icon mode): ^ Ctrl · ⇧ Shift · ↺ Esc · ⇥ Tab · ⌥ Opt ·
 * ✥ arrow drag pad · shortcuts · history │ composer · keyboard. With
 * `modifierIcons` off (the default) Ctrl/Shift/Esc/Tab/Opt show as mono
 * text labels.
 *
 * Focus: no toolbar key may take focus from the hidden IME textarea
 * (`#term-ime`), or every tap would close the soft keyboard. Every key
 * cancels its `mousedown` (the compat event whose default action moves
 * focus) — never `pointerdown`/`touchstart`, which would also stop the
 * row from scrolling horizontally.
 */
import { createSignal, For, onCleanup } from 'solid-js';
import type { JSX } from 'solid-js';
import {
  ArrowBigUp,
  ArrowRightToLine,
  ChevronUp,
  ClipboardPaste,
  History,
  Keyboard,
  ListTree,
  MessageSquare,
  Option,
  RotateCcw,
} from 'lucide-solid';
import type { ToolbarKey } from '../settings/schema';
import {
  KEY_BYTES,
  MODIFIABLE_KEYS,
  type ArrowDirection,
  type Modifiers,
  type ModifierKind,
} from '../lib/termToolbar';
import { hapticTick, type HapticKind } from '../lib/haptics';
import ArrowPad from './ArrowPad';

export { hapticTick };

/** Accessible names: say what each key does, not what it looks like. */
export const KEY_ARIA: Record<ToolbarKey, string> = {
  ctrl: 'Ctrl modifier',
  shift: 'Shift modifier',
  esc: 'Escape',
  tab: 'Tab',
  opt: 'Option (Meta) modifier',
  arrows: 'Arrow keys: drag and hold up, down, left or right',
  ctrl_c: 'Ctrl+C (interrupt)',
  paste: 'Paste clipboard',
  shortcuts: 'Shortcuts',
  history: 'Command history',
  composer: 'Composer',
  keyboard: 'Keyboard',
  enter: 'Enter',
  backspace: 'Backspace',
  home: 'Home',
  end: 'End',
  pgup: 'Page up',
  pgdn: 'Page down',
};

/** Text labels for text-label mode (Moshi default). */
export const KEY_TEXT: Partial<Record<ToolbarKey, string>> = {
  ctrl: 'Ctrl',
  shift: 'Shift',
  esc: 'Esc',
  tab: 'Tab',
  opt: 'Opt',
  ctrl_c: '^C',
  enter: 'Enter',
  backspace: 'Bksp',
  home: 'Home',
  end: 'End',
  pgup: 'PgUp',
  pgdn: 'PgDn',
};

const LONG_PRESS_MS = 450;
const ICON = 18;

export type TermToolbarProps = {
  keys: ToolbarKey[];
  haptics: boolean;
  /** Icons (^ ⇧ ↺ ⇥ ⌥) instead of text labels for the modifier row. */
  modifierIcons: boolean;
  /** Whether the soft keyboard is up (lights the keyboard key). */
  keyboardOpen: boolean;
  modifiers: Modifiers;
  /** Emits raw bytes for every key (already modifier-encoded). */
  onBytes: (bytes: string) => void;
  onPaste: () => void;
  onShortcuts: () => void;
  onHistory: () => void;
  onComposer: () => void;
  onKeyboardToggle: () => void;
};

/** Cancels the focus-moving default of a key press (see file doc). */
const keepFocus = (event: MouseEvent) => event.preventDefault();

export default function TermToolbar(props: TermToolbarProps) {
  // Bumped whenever modifier state may have changed (taps, consumes).
  const [modTick, setModTick] = createSignal(0);
  const refreshMods = () => setModTick(modTick() + 1);

  function tick(kind: HapticKind = 'tap') {
    void hapticTick(props.haptics, kind);
  }

  function send(bytes: string) {
    props.onBytes(bytes);
    refreshMods();
  }

  function tapKey(key: ToolbarKey) {
    tick();
    if (key === 'paste') return props.onPaste();
    if (key === 'shortcuts') return props.onShortcuts();
    if (key === 'history') return props.onHistory();
    const name = MODIFIABLE_KEYS[key];
    if (name !== undefined) return send(props.modifiers.consume(name));
    const bytes = KEY_BYTES[key];
    if (bytes) send(bytes);
  }

  // Soft-keyboard input consumes one-shots outside this component; poll
  // cheaply so the lit state follows.
  const poll = setInterval(refreshMods, 150);
  onCleanup(() => clearInterval(poll));

  function modifierKey(kind: ModifierKind): JSX.Element {
    let pressTimer: ReturnType<typeof setTimeout> | undefined;
    let longPressed = false;
    const state = () => (modTick(), props.modifiers.get(kind));
    const lit = () => state().isActive();
    const locked = () => state().isLocked();
    return (
      <button
        type="button"
        class={`term-key ${props.modifierIcons ? '' : 'term-key-text'} ${lit() ? 'term-key-lit' : ''} ${locked() ? 'term-key-locked' : ''}`}
        aria-label={`${KEY_ARIA[kind]}${lit() ? (locked() ? ', locked' : ', armed for next key') : ''}`}
        aria-pressed={lit()}
        onMouseDown={keepFocus}
        onPointerDown={() => {
          longPressed = false;
          clearTimeout(pressTimer);
          pressTimer = setTimeout(() => {
            longPressed = true;
            props.modifiers.lock(kind);
            tick('tick');
            refreshMods();
          }, LONG_PRESS_MS);
        }}
        onPointerUp={() => clearTimeout(pressTimer)}
        onPointerCancel={() => clearTimeout(pressTimer)}
        onPointerLeave={() => clearTimeout(pressTimer)}
        onContextMenu={(event) => event.preventDefault()}
        onClick={() => {
          clearTimeout(pressTimer);
          if (longPressed) {
            longPressed = false;
            return;
          }
          props.modifiers.tap(kind, Date.now());
          tick();
          refreshMods();
        }}
      >
        {keyGlyph(kind, props.modifierIcons)}
        <span class="term-key-lock-dot" aria-hidden="true" />
      </button>
    );
  }

  function staticKey(key: ToolbarKey): JSX.Element {
    if (key === 'arrows') return arrowPad();
    const text = !props.modifierIcons && (key === 'esc' || key === 'tab');
    return (
      <button
        type="button"
        class={`term-key ${text || !hasIcon(key) ? 'term-key-text' : ''}`}
        aria-label={KEY_ARIA[key] ?? key}
        onMouseDown={keepFocus}
        onClick={() => tapKey(key)}
      >
        {keyGlyph(key, props.modifierIcons)}
      </button>
    );
  }

  /** The arrows key is a drag pad, not a button (see ArrowPad). */
  function arrowPad(): JSX.Element {
    return (
      <ArrowPad
        class="term-key"
        label={KEY_ARIA.arrows}
        iconSize={ICON}
        haptics={props.haptics}
        onArrow={(direction: ArrowDirection) => send(props.modifiers.consume(direction))}
      />
    );
  }

  function pinnedKey(key: 'composer' | 'keyboard'): JSX.Element {
    const lit = () => key === 'keyboard' && props.keyboardOpen;
    return (
      <button
        type="button"
        class={`term-key ${lit() ? 'term-key-lit' : ''}`}
        aria-label={key === 'keyboard' ? (lit() ? 'Hide keyboard' : 'Show keyboard') : KEY_ARIA.composer}
        aria-pressed={key === 'keyboard' ? lit() : undefined}
        onMouseDown={keepFocus}
        onClick={() => {
          tick();
          if (key === 'composer') props.onComposer();
          else props.onKeyboardToggle();
        }}
      >
        {key === 'composer' ? <MessageSquare size={ICON} /> : <Keyboard size={ICON} />}
      </button>
    );
  }

  return (
    <div class="term-toolbar" role="toolbar" aria-label="Terminal toolbar">
      <div class="term-key-scroll">
        <For each={props.keys.filter((key) => key !== 'composer' && key !== 'keyboard')}>
          {(key) => (key === 'ctrl' || key === 'shift' || key === 'opt' ? modifierKey(key) : staticKey(key))}
        </For>
      </div>
      <div class="term-toolbar-pinned">
        {pinnedKey('composer')}
        {pinnedKey('keyboard')}
      </div>
    </div>
  );
}

function hasIcon(key: ToolbarKey): boolean {
  return ['ctrl', 'shift', 'esc', 'tab', 'opt', 'paste', 'shortcuts', 'history'].includes(key);
}

/** Glyph for a scroll-row key: Moshi icons, or mono text labels. */
export function keyGlyph(key: ToolbarKey, icons: boolean): JSX.Element {
  const textMode = !icons && (key === 'ctrl' || key === 'shift' || key === 'esc' || key === 'tab' || key === 'opt');
  if (textMode || !hasIcon(key)) {
    return <span class="term-key-label">{KEY_TEXT[key] ?? key}</span>;
  }
  switch (key) {
    case 'ctrl':
      return <ChevronUp size={ICON} />;
    case 'shift':
      return <ArrowBigUp size={ICON} />;
    case 'esc':
      return <RotateCcw size={ICON} />;
    case 'tab':
      return <ArrowRightToLine size={ICON} />;
    case 'opt':
      return <Option size={ICON} />;
    case 'paste':
      return <ClipboardPaste size={ICON} />;
    case 'shortcuts':
      return <ListTree size={ICON} />;
    case 'history':
      return <History size={ICON} />;
    default:
      return <span class="term-key-label">{key}</span>;
  }
}
