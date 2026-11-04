/**
 * `queue` / `queue-held` (WP-A11, Z-52/Z-53): the follow-up queue in the
 * dock. `queue` is a live run with one steering and two follow-up texts
 * waiting; typing + Enter queues more (the mode matrix is visible live),
 * every row edit runs through `_oino/queue_edit`, Clear is `clear_queue`,
 * Stop leaves the queue **held** — `queue-held` is that moment: idle after
 * an interrupt, Resume delivers the first item and the run drains the rest.
 *
 * All events share one seq counter (the store rejects gaps).
 */
import type { MockHost, MockScenario, MockSession } from '../host';
import type { SessionEventDto } from '../../../lib/host/client';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

type Queues = { steering: string[]; followUp: string[] };
type Step = { at: number; dto: SessionEventDto };

const QUEUES: Queues = { steering: ['Keep the public API unchanged'], followUp: ['Run the linter afterwards', 'Update the README example'] };

/** A queue that actually answers: edits mutate the state and publish `queue_update`. */
function queueSession(id: string, opts: { running: boolean; queues: Queues }): MockSession {
  const base = genSession(id, { seed: 6, turns: 2, toolsPerTurn: 1, cwd: '/home/you/projects/unipi' });
  const q: Queues = { steering: [...opts.queues.steering], followUp: [...opts.queues.followUp] };
  let seq = 0;
  let live = opts.running;
  const ev = (event: Record<string, unknown>, at = 0): Step => ({ at, dto: { session: id, seq: ++seq, event: event as never } });
  const push = (host: MockHost, events: Array<Record<string, unknown>>) =>
    host.world.push(host.hostId, id, events.map((event) => ({ session: id, seq: ++seq, event: event as never })));

  // `genLiveRun` numbers its own steps from 1; the counter continues after them.
  const run = opts.running ? genLiveRun(id, { seq: 1, running: true, prompt: 'Run the tests and fix what fails' }) : [];
  seq = run.length;
  const steps: Step[] = [...run, ev({ type: 'queue_update', steering: q.steering, followUp: q.followUp }, 50)];
  if (!opts.running) steps.push(ev({ type: 'run_state', state: 'idle' }));

  return {
    ...base,
    events: steps,
    cmd: {
      steer: (cmd, host) => {
        q.steering.push(String(cmd.message ?? ''));
        push(host, [{ type: 'queue_update', steering: q.steering, followUp: q.followUp }]);
        return {};
      },
      follow_up: (cmd, host) => {
        q.followUp.push(String(cmd.message ?? ''));
        push(host, [{ type: 'queue_update', steering: q.steering, followUp: q.followUp }]);
        return {};
      },
      abort: (cmd, host) => {
        live = false;
        push(host, [{ type: 'agent_end', messages: [] }, { type: 'run_state', state: 'idle' }]);
        return {};
      },
      clear_queue: (_cmd, host) => {
        q.steering = [];
        q.followUp = [];
        push(host, [{ type: 'queue_update', steering: [], followUp: [] }]);
        return { steering: [], followUp: [] };
      },
      '_oino/queue_edit': (cmd, host) => {
        const queue = cmd.queue === 'steering' ? q.steering : q.followUp;
        switch (cmd.op) {
          case 'remove':
            queue.splice(Number(cmd.index), 1);
            break;
          case 'edit':
            queue[Number(cmd.index)] = String(cmd.text ?? '');
            break;
          case 'move': {
            const [moved] = queue.splice(Number(cmd.from), 1);
            queue.splice(Number(cmd.to), 0, moved!);
            break;
          }
          case 'sendNow': {
            const [taken] = queue.splice(Number(cmd.index), 1);
            // Like the host: while a run is going the item becomes the first steering message.
            if (live && taken !== undefined) {
              q.steering.unshift(taken);
              break;
            }
            push(host, [
              { type: 'agent_start' },
              { type: 'message_start', message: { role: 'user', content: taken ?? '' } },
              { type: 'queue_update', steering: q.steering, followUp: q.followUp },
            ]);
            return {};
          }
        }
        push(host, [{ type: 'queue_update', steering: q.steering, followUp: q.followUp }]);
        return {};
      },
    },
  };
}

function host(running: boolean) {
  return {
    id: 'pc-coffee',
    name: 'coffee',
    conn: { state: 'connected' as const },
    cards: genCards(3).map((c, i) =>
      i === 0 ? { ...c, session_id: 's001', run_state: (running ? 'working' : 'idle') as 'working' | 'idle', name: 'Fix the failing tests' } : c,
    ),
    fs: genFsTree(),
    sessions: { s001: queueSession('s001', { running, queues: QUEUES }) },
  };
}

export const queuePanel: MockScenario = { name: 'queue', hosts: [host(true)] };
export const queueHeld: MockScenario = { name: 'queue-held', hosts: [host(false)] };
