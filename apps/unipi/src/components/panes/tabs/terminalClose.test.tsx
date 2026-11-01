import { afterEach, describe, expect, it, vi } from 'vitest';
import { confirmCloseMany, confirmTerminalClose, endTerminal, registerTermBusy, resetTerminalClose, terminalCloseAsk } from './terminalClose';
import { bindTabTerm, tabTermBinding } from '../../../lib/term';
import { setHostClientFactory, type HostClient } from '../../../lib/host';

afterEach(() => {
  resetTerminalClose();
  setHostClientFactory(undefined);
  localStorage.clear();
});

const tab = (id: string) => ({ id, kind: 'terminal', title: 'oino' });

describe('closing a terminal tab (WP-A31)', () => {
  it('closes an idle terminal straight away', () => {
    registerTermBusy('a', () => ({ running: false }), () => {});
    expect(confirmTerminalClose(tab('a'), () => {})).toBe(true);
    expect(confirmTerminalClose(tab('unknown'), () => {})).toBe(true);
  });

  it('asks while a program runs; confirming lets the close through', () => {
    registerTermBusy('b', () => ({ running: true, hint: 'vim notes.md', shell: '/bin/zsh' }), () => {});
    const proceed = vi.fn();
    expect(confirmTerminalClose(tab('b'), proceed)).toBe(false);
    expect(terminalCloseAsk()).toMatchObject({ tabId: 'b', hint: 'vim notes.md', title: 'oino' });
    // A second ✕ while the ask is open does not stack another one.
    expect(confirmTerminalClose(tab('b'), proceed)).toBe(false);
    const confirm = document.querySelector<HTMLButtonElement>('[data-testid=term-close-confirm]');
    expect(document.body.textContent).toContain('vim notes.md');
    confirm!.click();
    expect(proceed).toHaveBeenCalledTimes(1);
    expect(confirmTerminalClose(tab('b'), proceed)).toBe(true);
  });

  it('ends a live terminal through its runtime, a never-shown one through the host', () => {
    const end = vi.fn();
    registerTermBusy('c', () => ({ running: false }), end);
    endTerminal('c');
    expect(end).toHaveBeenCalled();

    const request = vi.fn(async () => null);
    setHostClientFactory(() => ({ request }) as unknown as HostClient);
    bindTabTerm('d', { termId: 't9', hostId: 'h1', cwd: '/p' });
    endTerminal('d');
    expect(request).toHaveBeenCalledWith('term_kill', { termId: 't9' });
    expect(tabTermBinding('d')).toBeUndefined();
  });
});

describe('host-reported foreground (term_list) decides', () => {
  const hostWith = (rows: unknown[]) => {
    const request = vi.fn(async (op: string) => (op === 'term_list' ? rows : null));
    setHostClientFactory(() => ({ request }) as unknown as HostClient);
    return request;
  };

  it('a bound terminal closes by itself when the host reports no foreground program', async () => {
    hostWith([{ term_id: 't1', foreground: null }]);
    bindTabTerm('e', { termId: 't1', hostId: 'h1', cwd: '/p' });
    // The app's guess says busy; the host says idle — the host wins.
    registerTermBusy('e', () => ({ running: true, hint: 'sleep 9' }), () => {});
    const proceed = vi.fn();
    expect(confirmTerminalClose(tab('e'), proceed)).toBe(false);
    await vi.waitFor(() => expect(proceed).toHaveBeenCalledTimes(1));
    expect(terminalCloseAsk()).toBeUndefined();
  });

  it('asks with the host-reported program name', async () => {
    hostWith([{ term_id: 't2', foreground: { name: 'htop' } }]);
    bindTabTerm('f', { termId: 't2', hostId: 'h1', cwd: '/p' });
    const proceed = vi.fn();
    expect(confirmTerminalClose(tab('f'), proceed)).toBe(false);
    await vi.waitFor(() => expect(terminalCloseAsk()).toMatchObject({ tabId: 'f', hint: 'htop' }));
    expect(proceed).not.toHaveBeenCalled();
  });

  it('Close all asks once for every busy terminal, then proceeds on confirm', async () => {
    hostWith([
      { term_id: 't3', foreground: { name: 'vim' } },
      { term_id: 't4' },
      { term_id: 't5', foreground: { name: 'cargo' } },
    ]);
    bindTabTerm('g', { termId: 't3', hostId: 'h1', cwd: '/p' });
    bindTabTerm('h', { termId: 't4', hostId: 'h1', cwd: '/p' });
    bindTabTerm('i', { termId: 't5', hostId: 'h1', cwd: '/p' });
    const proceed = vi.fn();
    await confirmCloseMany(
      [tab('g'), tab('h'), tab('i'), { id: 'x', kind: 'files', title: 'Files' }].map((t) => ({ tab: t as never, title: t.id })),
      proceed,
    );
    expect(proceed).not.toHaveBeenCalled();
    expect(terminalCloseAsk()?.items?.map((i) => i.hint)).toEqual(['vim', 'cargo']);
    expect(document.querySelector('[data-testid=term-close-list]')?.textContent).toContain('cargo');
    document.querySelector<HTMLButtonElement>('[data-testid=term-close-confirm]')!.click();
    expect(proceed).toHaveBeenCalledTimes(1);
  });

  it('Close others with no busy terminal proceeds at once', async () => {
    hostWith([{ term_id: 't6' }]);
    bindTabTerm('j', { termId: 't6', hostId: 'h1', cwd: '/p' });
    const proceed = vi.fn();
    await confirmCloseMany([{ tab: tab('j') as never, title: 'j' }], proceed);
    expect(proceed).toHaveBeenCalledTimes(1);
  });
});
