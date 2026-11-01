import { afterEach, describe, expect, it, vi } from 'vitest';
import { confirmTerminalClose, endTerminal, registerTermBusy, resetTerminalClose, terminalCloseAsk } from './terminalClose';
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
