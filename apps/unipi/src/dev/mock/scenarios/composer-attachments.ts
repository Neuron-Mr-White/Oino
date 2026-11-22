/**
 * Attachment / toolbar scenarios (WP-A13):
 *
 *   composer-attachments    one chat with a ready screenshot (annotatable),
 *                           a ready file, a file stuck at 42 % and one whose
 *                           upload failed (Z-60 states side by side)
 *   image-editor-ellipse    the same ready screenshot; the capture opens its
 *                           thumbnail, picks the ellipse tool and drags (D-5)
 *   composer-narrow         a chat whose composer shows the permission chip
 *                           (Auto) + Plan toggle (on) from the workflow
 *                           topic; captured at a narrow width to show Z-64's
 *                           seven-step compression folding them into icons
 *
 * The pending attachments are seeded through the real store in `setup()`, so
 * the captures show exactly what the composer would show. The blob transport
 * is swapped for one that reports progress and never settles (the uploading
 * chip) or fails (the failed chip, after its one automatic retry).
 */
import type { MockScenario, MockSession } from '../host';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';
const SESSION = 'attachments';
const TOPIC = 'unipi:workflow:status';

const cards = genCards(3, { seed: 11 });

const COMMANDS = [
  { name: 'permission', description: 'Permission mode (ask/auto/full)', source: 'extension' },
  { name: 'plan', description: 'Plan mode on/off/view/approve', source: 'extension' },
  { name: 'compact', description: 'Compact the context', source: 'builtin' },
  { name: 'skill:review', description: 'Review the diff like a senior engineer', source: 'skill' },
];

/** The screenshot the chips and the image editor show (a terminal capture). */
const SHOT = `<svg xmlns="http://www.w3.org/2000/svg" width="480" height="300" viewBox="0 0 480 300">
  <rect width="480" height="300" rx="12" fill="#141211"/>
  <rect x="0" y="0" width="480" height="34" rx="12" fill="#1C1917"/>
  <rect x="0" y="22" width="480" height="12" fill="#1C1917"/>
  <circle cx="20" cy="17" r="5" fill="#F03C32"/>
  <circle cx="38" cy="17" r="5" fill="#FAC828"/>
  <circle cx="56" cy="17" r="5" fill="#5AD25A"/>
  <rect x="86" y="9" width="150" height="16" rx="8" fill="#26211F"/>
  <text x="96" y="21" font-family="monospace" font-size="10" fill="#A39A90">coffee · unipi/src</text>
  <rect x="14" y="48" width="120" height="238" rx="10" fill="#1C1917"/>
  <rect x="26" y="62" width="96" height="9" rx="4" fill="#26211F"/>
  <rect x="26" y="80" width="72" height="9" rx="4" fill="#26211F"/>
  <rect x="26" y="98" width="84" height="9" rx="4" fill="#302924"/>
  <rect x="150" y="48" width="316" height="238" rx="10" fill="#181615"/>
  <text x="168" y="78" font-family="monospace" font-size="12" fill="#F5EFE6">cargo test -p unipi-host</text>
  <text x="168" y="100" font-family="monospace" font-size="12" fill="#5AD25A">test result: ok. 214 passed</text>
  <text x="168" y="122" font-family="monospace" font-size="12" fill="#A39A90">Finished test [unoptimized] target(s)</text>
  <rect x="168" y="142" width="220" height="1" fill="#2E2825"/>
  <text x="168" y="166" font-family="monospace" font-size="12" fill="#F07818">warning: unused import</text>
  <text x="186" y="186" font-family="monospace" font-size="12" fill="#A39A90">--> src/sessions/hub.rs:41</text>
  <rect x="168" y="212" width="280" height="26" rx="8" fill="#26211F"/>
  <text x="180" y="229" font-family="monospace" font-size="11" fill="#6E655D">pi@coffee:~/projects/unipi$</text>
</svg>`;

/** A one-chat world with the workflow extension's commands + live status. */
function base(name: string, opts: { running?: boolean; planMode?: boolean; permissionMode?: string } = {}): MockScenario {
  const session: MockSession = genSession(SESSION, { seed: 12, turns: 3, toolsPerTurn: 1, thinking: true, cwd: CWD });
  session.snapshot.commands = COMMANDS as never;
  session.snapshot.topics = [
    {
      topic: TOPIC,
      data: { planMode: opts.planMode ?? true, permissionMode: opts.permissionMode ?? 'auto' },
      sticky: true,
      publisher: 'unipi.workflow',
    },
  ] as never;
  if (opts.running) session.events = genLiveRun(SESSION, { seq: 100, running: true, at: 0 });
  return {
    name,
    hosts: [{ id: HOST, name: 'coffee', conn: { state: 'connected' }, cards, fs: genFsTree(CWD), sessions: { [SESSION]: session } }],
  };
}

const key = () => `${HOST}/${SESSION}`;

/** Base64 for a `data:` URL — UTF-8 safe (`btoa` alone mangles `·`, and the
 *  SVG would then fail to parse: a broken thumbnail and a stuck image editor). */
function base64Utf8(text: string): string {
  let binary = '';
  for (const byte of new TextEncoder().encode(text)) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** Seeds the chips the composer shows: a ready screenshot, a ready file, one
 *  uploading at 42 % and one that failed after its automatic retry (Z-60's
 *  four states side by side). Seeded through the store — the state machine
 *  itself is what the unit tests drive. */
function seedAttachments(withFiles = true): void {
  void import('../../../lib/composer/attachments').then((store) => {
    const image = base64Utf8(SHOT);
    store.seedAttachment(key(), {
      kind: 'image',
      name: 'host-tests.png',
      mime: 'image/svg+xml',
      size: Math.round((image.length * 3) / 4),
      status: 'ready',
      progress: 1,
      data: image,
    });
    if (!withFiles) return;
    store.seedAttachment(key(), {
      kind: 'file',
      name: 'session-hub.diff',
      mime: 'text/x-diff',
      size: 2048,
      status: 'ready',
      progress: 1,
      path: '/home/you/.oino/blobs/session-hub.diff',
    });
    store.seedAttachment(key(), {
      kind: 'file',
      name: 'relay-capture.log',
      mime: 'text/plain',
      size: 86_000,
      status: 'uploading',
      progress: 0.42,
    });
    store.seedAttachment(key(), {
      kind: 'file',
      name: 'iroh-trace.bin',
      mime: 'application/octet-stream',
      size: 1_340_000,
      status: 'failed',
      progress: 0.71,
      error: 'relay dropped the transfer',
    });
  });
}

export const composerAttachments: MockScenario = {
  ...base('composer-attachments', { running: false }),
  setup() {
    seedAttachments();
    void import('../../../lib/composer/drafts').then(({ setComposerText }) => {
      setComposerText(key(), 'can you check the host tests and this trace?');
    });
  },
};

export const imageEditorEllipse: MockScenario = {
  ...base('image-editor-ellipse', { planMode: true }),
  setup() {
    seedAttachments(false);
  },
};

export const composerNarrow: MockScenario = {
  ...base('composer-narrow', { running: true, planMode: true, permissionMode: 'full' }),
  setup() {
    void import('../../../lib/composer/drafts').then(({ setComposerText }) => {
      setComposerText(key(), 'queue a follow-up that tightens the relay timeouts');
    });
  },
};
