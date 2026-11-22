/**
 * `w5-mode-command` (w5-ui D3/D4): `/swarm <text>` the way the host plays it — the command echo lands at once, the command
 * answers `handled`, and the extension's own user message (author `extension`) only lands after its hooks ran. `?delay=<ms>`
 * (default 6000, the QA's 6.25 s) sets that gap. Capture: type `/swarm refactor the parser` in the composer, Enter.
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree, genSession } from '../gen';

const SESSION = 's001';
const HOST = 'pc-coffee';
const base = genSession(SESSION, { seed: 5, turns: 2, toolsPerTurn: 1, cwd: '/home/you/projects/unipi' });
base.snapshot.commands = [{ name: 'swarm', description: 'Run independent items in parallel', source: 'extension' }] as never;

type Emitter = { emitSession(key: string, session: unknown, event: Record<string, unknown>): void };

export const modeCommand: MockScenario = {
  name: 'w5-mode-command',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3),
      fs: genFsTree(),
      sessions: {
        [SESSION]: {
          ...base,
          cmd: {
            // `/swarm ` completes its sub-commands (D5): the host's extension completer.
            '_oino/get_completions': (cmd) => {
              const text = String(cmd.text);
              const prefix = text.slice(7);
              const items = [{ value: 'status', description: 'Show the swarm' }, { value: 'stop' }, { value: 'resume' }].filter((i) => i.value.startsWith(prefix));
              return text.startsWith('/swarm ') && items.length ? { groups: [{ source: 'command:swarm', from: 7, items }] } : { groups: [] };
            },
            prompt: (cmd, host) => {
              const text = String(cmd.message ?? '');
              const m = /^\/swarm\s+([\s\S]+)$/.exec(text);
              if (!m) return {};
              const emit = (event: Record<string, unknown>) => (host as unknown as Emitter).emitSession(`${HOST}/${SESSION}`, session, event);
              const session = base;
              const last = base.snapshot.leaf_id ?? null;
              const stamp = () => new Date().toISOString();
              emit({ type: 'entry_appended', entry: { type: 'custom', id: 'w5-echo', parentId: last, customType: 'oino.command-echo', timestamp: stamp(), data: { name: 'swarm', args: m[1] } } });
              const delay = Number(new URLSearchParams(location.hash.slice(location.hash.indexOf('?') + 1)).get('delay') ?? 6000);
              setTimeout(() => {
                emit({ type: 'entry_appended', entry: { type: 'custom', id: 'w5-mode', parentId: 'w5-echo', customType: 'long-horizon-decision', timestamp: stamp(), data: { mode: 'swarm' } } });
                emit({ type: 'entry_appended', entry: { type: 'custom_message', id: 'w5-resp', parentId: 'w5-mode', customType: 'unipi-response', content: 'Swarm started', display: true, details: { source: 'Long-horizon' }, timestamp: stamp() } });
                const message = { role: 'user', author: 'extension', timestamp: Date.now(), content: [{ type: 'text', text: m[1] }] };
                emit({ type: 'message_start', message });
                emit({ type: 'entry_appended', entry: { type: 'message', id: 'w5-user', parentId: 'w5-resp', timestamp: stamp(), message } });
              }, delay);
              return { disposition: 'handled' };
            },
          },
        },
      },
    },
  ],
};
