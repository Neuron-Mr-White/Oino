// @vitest-environment jsdom
/** UNI-266: the palette's "Pair a phone" (desktop app only; allCommands() drops commands whose `when` is false). */
import { cleanup, render } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, describe, expect, it } from 'vitest';
import { BuiltinPanes } from './builtins';
import { allCommands } from '../../lib/shell';
import { setPlatformOverride } from '../../lib/platform';
import { pairPhoneOpen, resetPairPhone, setPairPhoneTransport } from '../../lib/pairPhone';

afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  setPairPhoneTransport(undefined);
  resetPairPhone();
});

function mount() {
  render(() => (
    <HashRouter>
      <Route path="*" component={() => <BuiltinPanes />} />
    </HashRouter>
  ));
  return allCommands().find((c) => c.id === 'hosts.pairPhone');
}

describe('palette: Pair a phone', () => {
  it('desktop app: shown, opens the dialog', () => {
    setPlatformOverride('tauri-desktop');
    setPairPhoneTransport({ start: async () => undefined, cancel: async () => undefined, startHost: async () => undefined });
    const cmd = mount()!;
    expect(cmd.title).toBe('Pair a phone');
    expect(cmd.when?.()).toBe(true);
    cmd.run();
    expect(pairPhoneOpen()).toBe(true);
  });

  it('phone app and web: hidden', () => {
    setPlatformOverride('tauri-mobile');
    expect(mount()).toBeUndefined();
    cleanup();
    setPlatformOverride('web');
    expect(mount()).toBeUndefined();
  });
});
