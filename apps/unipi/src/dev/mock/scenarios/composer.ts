/**
 * Composer scenarios (WP-A12):
 *
 *   composer-states  three sessions side by side in the sidebar: an empty
 *                    one (capability hint Z-57), one with history (follow-up
 *                    hint), one with a live run ("Keep typing to queue…")
 *   composer-paste   a session whose draft carries a long-paste chip (Z-61)
 *                    plus typed text; the chip tooltip holds the paste head
 *   composer-slash   a session with command/prompt/skill sources (Z-62); the
 *                    capture types `/` and `@` to open the menus
 *
 * Drafts and prompt history are seeded through the real stores in `setup()`
 * (the capture harness clears storage first), so the captures show exactly
 * what a returning user sees.
 */
import type { MockScenario, MockSession } from '../host';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';
const cards = genCards(4, { seed: 9 });

const COMMANDS = [
  { name: 'model', description: 'Pick the model for this session', source: 'builtin' },
  { name: 'compact', description: 'Compact the context', source: 'builtin' },
  { name: 'unipi:goal', description: 'Set a goal for this session', source: 'extension' },
  { name: 'unipi:btw', description: 'Ask a side question', source: 'extension' },
  { name: 'fix-typo', description: 'Fix the typo under the caret', source: 'prompt' },
  { name: 'write-changelog', description: 'Append a changelog entry', source: 'prompt' },
  { name: 'skill:review', description: 'Review the diff like a senior engineer', source: 'skill' },
  { name: 'skill:release', description: 'Walk the release checklist', source: 'skill' },
];

function withCommands(session: MockSession): MockSession {
  session.snapshot.commands = COMMANDS as never;
  return session;
}

/** The long paste (Z-61) the setup seeds as a chip: 128 lines, ≥ 5120 chars. */
const PASTED = `${'lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod\n'.repeat(127)}last line of the paste`;

function base(sessions: Record<string, MockSession>): MockScenario {
  return {
    name: 'composer',
    hosts: [{ id: HOST, name: 'coffee', conn: { state: 'connected' }, cards, fs: genFsTree(CWD), sessions }],
  };
}

export const composerStates: MockScenario = {
  ...base({
    empty: withCommands(genSession('empty', { seed: 1, turns: 0, toolsPerTurn: 0, cwd: CWD })),
    history: withCommands(genSession('history', { seed: 2, turns: 3, toolsPerTurn: 1, thinking: true, cwd: CWD })),
    running: {
      ...withCommands(genSession('running', { seed: 3, turns: 3, toolsPerTurn: 1, cwd: CWD })),
      events: genLiveRun('running', { seq: 1, running: true, at: 0 }),
    },
  }),
  name: 'composer-states',
};

export const composerPaste: MockScenario = {
  ...base({ history: withCommands(genSession('history', { seed: 2, turns: 3, toolsPerTurn: 1, cwd: CWD })) }),
  name: 'composer-paste',
  setup() {
    // Runs after the harness cleared storage: seed the draft the way the
    // composer itself would (chip in memory, text persisted).
    void import('../../../lib/composer/drafts').then(({ composerChips, setComposerText }) => {
      composerChips.add(`${HOST}/history`, {
        kind: 'paste',
        label: 'Pasted text · 128 lines',
        detail: PASTED.replace(/\s+/g, ' ').slice(0, 200),
        markdown: PASTED,
      });
      setComposerText(`${HOST}/history`, 'summarize this and keep the API notes');
    });
  },
};

export const composerSlash: MockScenario = {
  ...base({ history: withCommands(genSession('history', { seed: 2, turns: 3, toolsPerTurn: 1, cwd: CWD })) }),
  name: 'composer-slash',
  setup() {
    void import('../../../lib/composer/history').then(({ promptHistory }) => {
      promptHistory(`${HOST}:${CWD}`).add('Add a --json flag to the CLI');
      promptHistory(`${HOST}:${CWD}`).add('Why is the build slow? Profile it');
    });
  },
};
