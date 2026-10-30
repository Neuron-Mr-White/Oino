// @vitest-environment jsdom
/**
 * Model picker state machine (UNI-212): one explicit footer action, no
 * phantom "Turn Fusion off", pending → close / inline error, and the
 * current mark follows what pi really runs.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { currentSelection, ModelPickerSheet, nextSinglePick, revealPicked, singleAction } from './ControlCentre';
import type { Chat } from '../../lib/chat/store';

afterEach(cleanup);

const MODELS = [
  { provider: 'a', id: 'one', name: 'Model One', reasoning: true },
  { provider: 'a', id: 'two', name: 'Model Two', reasoning: false },
  { provider: 'b', id: 'lead', name: 'Lead Model', reasoning: true },
  { provider: 'b', id: 'side', name: 'Side Model', reasoning: false },
];
const FUSION = {
  leads: [{ key: 'b/lead', name: 'Lead Model' }],
  sidekicks: [{ key: 'b/side', name: 'Side Model' }, { key: 'a/two', name: 'Model Two' }],
  curated: [],
  default: { lead: 'b/lead', sidekick: 'b/side' },
  effort: {},
  active: undefined as unknown,
};

function fakeChat(opts: { model?: string; active?: unknown; fusion?: boolean } = {}) {
  const [provider, id] = (opts.model ?? 'a/one').split('/');
  const [state, setState] = createStore<any>({
    conn: 'live',
    run: { running: false, model: MODELS.find((m) => m.provider === provider && m.id === id), thinking: 'medium', thinkingLevels: ['off', 'low', 'medium', 'high'] },
    models: MODELS,
    fusion: opts.fusion === false ? undefined : { ...FUSION, active: opts.active },
  });
  const chat = {
    state,
    setFusionSingle: vi.fn(async () => {}),
    setFusionPair: vi.fn(async () => {}),
    selectModel: vi.fn(async () => {}),
    selectThinking: vi.fn(async () => {}),
  };
  return { chat: chat as unknown as Chat & typeof chat, setState };
}

function mount(chat: Chat) {
  const onClose = vi.fn();
  render(() => <ModelPickerSheet open chat={chat} onClose={onClose} onHaptic={() => {}} />);
  return { onClose };
}
const sheet = () => document.querySelector('[data-testid=chat-model-picker]') as HTMLElement;
const foot = () => document.querySelector('[data-testid=model-picker-foot]') as HTMLElement;
const row = (label: string) => [...sheet().querySelectorAll<HTMLButtonElement>('.model-row')].find((b) => b.textContent?.includes(label))!;
const footButtons = () => [...foot().querySelectorAll('button')].map((b) => b.textContent?.trim());

describe('picker helpers (UNI-212)', () => {
  it('currentSelection: Fusion only while pi still runs its lead', () => {
    const run = { model: { provider: 'b', id: 'lead' } } as never;
    expect(currentSelection({ run, fusion: { ...FUSION, active: { kind: 'fusion', lead: 'b/lead', sidekick: 'b/side' } } } as never).kind).toBe('fusion');
    const other = { model: { provider: 'a', id: 'one' } } as never;
    expect(currentSelection({ run: other, fusion: { ...FUSION, active: { kind: 'fusion', lead: 'b/lead', sidekick: 'b/side' } } } as never)).toEqual({ kind: 'single', key: 'a/one' });
    expect(currentSelection({ run: other, fusion: undefined } as never)).toEqual({ kind: 'single', key: 'a/one' });
  });
  it('singleAction / nextSinglePick: no action for nothing or the running model; tapping twice clears', () => {
    const single = { kind: 'single' as const, key: 'a/one' };
    expect(singleAction(single, undefined)).toBeUndefined();
    expect(singleAction(single, 'a/one')).toBeUndefined();
    expect(singleAction(single, 'a/two')).toEqual({ label: 'Switch model', offFusion: false });
    expect(singleAction({ kind: 'fusion', lead: 'b/lead', sidekick: 'b/side' }, 'a/two')?.offFusion).toBe(true);
    expect(nextSinglePick(single, undefined, 'a/two')).toBe('a/two');
    expect(nextSinglePick(single, 'a/two', 'a/two')).toBeUndefined();
    expect(nextSinglePick(single, undefined, 'a/one')).toBeUndefined();
  });
});

describe('ModelPickerSheet single mode (UNI-212)', () => {
  it('opens on Single with no action button; the running model is marked current', () => {
    const { chat } = fakeChat();
    mount(chat);
    expect(footButtons()).toEqual([]);
    expect(foot().textContent).toMatch(/Tap a model/);
    expect(row('Model One').getAttribute('aria-current')).toBe('true');
  });

  it('tapping the current model twice never shows a phantom button', () => {
    const { chat } = fakeChat();
    mount(chat);
    fireEvent.click(row('Model One'));
    fireEvent.click(row('Model One'));
    expect(footButtons()).toEqual([]);
  });

  it('tap another model → Switch model; tap it again → cleared; Cancel clears too', () => {
    const { chat } = fakeChat();
    mount(chat);
    fireEvent.click(row('Model Two'));
    expect(footButtons()).toEqual(['Cancel', 'Switch model']);
    fireEvent.click(row('Model Two'));
    expect(footButtons()).toEqual([]);
    fireEvent.click(row('Model Two'));
    fireEvent.click(foot().querySelector('.pair-secondary')!);
    expect(footButtons()).toEqual([]);
  });

  it('Switch model shows a pending state, applies via set_fusion, then closes', async () => {
    const { chat } = fakeChat();
    let resolve!: () => void;
    chat.setFusionSingle.mockImplementation(() => new Promise<void>((r) => (resolve = r)));
    const { onClose } = mount(chat);
    fireEvent.click(row('Model Two'));
    fireEvent.click(foot().querySelector('.pair-primary')!);
    expect(foot().querySelector('.pair-primary')!.textContent).toMatch(/Switching/);
    expect((foot().querySelector('.pair-primary') as HTMLButtonElement).disabled).toBe(true);
    expect(chat.setFusionSingle).toHaveBeenCalledWith('a/two', 'medium');
    resolve();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('a failed switch stays open with the error inline and the pick kept', async () => {
    const { chat } = fakeChat();
    chat.setFusionSingle.mockRejectedValue(new Error('No API key for a'));
    const { onClose } = mount(chat);
    fireEvent.click(row('Model Two'));
    fireEvent.click(foot().querySelector('.pair-primary')!);
    await waitFor(() => expect(foot().querySelector('[role=alert]')?.textContent).toBe('No API key for a'));
    expect(onClose).not.toHaveBeenCalled();
    expect(footButtons()).toEqual(['Cancel', 'Switch model']);
  });

  it('no Fusion installed: switching uses set_model', async () => {
    const { chat } = fakeChat({ fusion: false });
    const { onClose } = mount(chat);
    expect(sheet().querySelector('[aria-label="Model mode"]')).toBeNull();
    fireEvent.click(row('Model Two'));
    fireEvent.click(foot().querySelector('.pair-primary')!);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(chat.selectModel).toHaveBeenCalledWith('a', 'two');
  });
});

describe('ModelPickerSheet fusion mode (UNI-212)', () => {
  it('Fusion active: opens on the Fusion tab; the pair button says Current pair until something changes', () => {
    const { chat } = fakeChat({ model: 'b/lead', active: { kind: 'fusion', lead: 'b/lead', sidekick: 'b/side', leadEffort: 'medium', sidekickEffort: 'medium' } });
    mount(chat);
    expect(sheet().querySelector('[aria-label="Model mode"] [aria-checked=true]')!.textContent).toBe('Fusion');
    const btn = foot().querySelector('.pair-primary') as HTMLButtonElement;
    expect(btn.textContent).toBe('Current pair');
    expect(btn.disabled).toBe(true);
    fireEvent.click(row('Model Two'));
    expect(btn.textContent).toBe('Update pair');
    expect(btn.disabled).toBe(false);
  });

  it('Use this pair: pending, then closes on success', async () => {
    const { chat } = fakeChat();
    let resolve!: () => void;
    chat.setFusionPair.mockImplementation(() => new Promise<void>((r) => (resolve = r)));
    const { onClose } = mount(chat);
    fireEvent.click(sheet().querySelectorAll('[aria-label="Model mode"] button')[1]!);
    const btn = foot().querySelector('.pair-primary') as HTMLButtonElement;
    expect(btn.textContent).toBe('Use this pair');
    fireEvent.click(btn);
    expect(btn.textContent).toMatch(/Applying/);
    expect(btn.disabled).toBe(true);
    expect(chat.setFusionPair).toHaveBeenCalledWith('b/lead', 'b/side', 'medium', 'medium');
    resolve();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('Use this pair failure: inline error, sheet stays open, button usable again', async () => {
    const { chat } = fakeChat();
    chat.setFusionPair.mockRejectedValue(new Error('lead unavailable'));
    const { onClose } = mount(chat);
    fireEvent.click(sheet().querySelectorAll('[aria-label="Model mode"] button')[1]!);
    fireEvent.click(foot().querySelector('.pair-primary')!);
    await waitFor(() => expect(foot().querySelector('[role=alert]')?.textContent).toBe('lead unavailable'));
    expect(onClose).not.toHaveBeenCalled();
    expect((foot().querySelector('.pair-primary') as HTMLButtonElement).disabled).toBe(false);
  });

  it('Fusion on, Single tab: picking a model offers one explicit "Turn Fusion off & switch"', () => {
    const { chat } = fakeChat({ model: 'b/lead', active: { kind: 'fusion', lead: 'b/lead', sidekick: 'b/side' } });
    mount(chat);
    fireEvent.click(sheet().querySelectorAll('[aria-label="Model mode"] button')[0]!);
    expect(footButtons()).toEqual([]);
    expect(row('Lead Model').getAttribute('aria-current')).toBeNull();
    fireEvent.click(row('Model One'));
    expect(footButtons()).toEqual(['Cancel', 'Turn Fusion off & switch']);
  });

  it('a stale fusion preset after a model switch no longer marks Fusion current', () => {
    const { chat } = fakeChat({ model: 'a/two', active: { kind: 'fusion', lead: 'b/lead', sidekick: 'b/side' } });
    mount(chat);
    expect(sheet().querySelector('[aria-label="Model mode"] [aria-checked=true]')!.textContent).toBe('Single');
    expect(row('Model Two').getAttribute('aria-current')).toBe('true');
    expect(document.querySelector('[data-testid=model-picker-current]')!.textContent).toMatch(/Model Two/);
  });
});

describe('revealPicked (Fusion columns)', () => {
  const rect = (top: number, height: number) => ({ top, bottom: top + height, height, left: 0, right: 100, width: 100, x: 0, y: top, toJSON: () => ({}) }) as DOMRect;
  it('scrolls a column so the lead/sidekick in use is visible; leaves visible ones alone', () => {
    const root = document.createElement('div');
    root.innerHTML = '<div class="cc-fusion-list" id="a"><button></button><button aria-current="true"></button></div><div class="cc-fusion-list" id="b"><button aria-current="true"></button></div>';
    const a = root.querySelector<HTMLElement>('#a')!;
    const b = root.querySelector<HTMLElement>('#b')!;
    a.getBoundingClientRect = () => rect(100, 200);
    a.querySelector<HTMLElement>('[aria-current]')!.getBoundingClientRect = () => rect(500, 50); // below the fold
    b.getBoundingClientRect = () => rect(100, 200);
    b.querySelector<HTMLElement>('[aria-current]')!.getBoundingClientRect = () => rect(120, 50); // visible
    revealPicked(root);
    expect(a.scrollTop).toBe(500 - 100 - 75); // centred
    expect(b.scrollTop).toBe(0);
  });
});
