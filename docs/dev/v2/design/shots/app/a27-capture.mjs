/**
 * WP-A27 capture script: drives the owner's CloakBrowser over raw CDP (Node's
 * built-in WebSocket) against the capture harness on port 1450 (start it with
 * `npx vite --config vite.config.capture.ts`).
 *
 * Usage: node docs/dev/v2/design/shots/app/a27-capture.mjs
 *
 * Desktop 1280x820 and phone 412x915 (§13.6), scenarios from
 * `src/dev/mock/scenarios/turn-files.ts`. Each shot is one footer state:
 * collapsed ("3 files +15 −04" under a settled turn), expanded (paths, +/−,
 * status, the lazy inline diff), the "· undone" state, and the
 * "Changed since this turn — undo anyway?" conflict prompt.
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = `http://127.0.0.1:${process.env.AGENT_BROWSER_CDP ?? '45859'}`;
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;

/** Polls for a selector inside the page (the footer arrives after the stream). */
const WAIT_FOR = `const waitFor = async (selector, ms = 12000) => {
  const started = Date.now();
  for (;;) {
    const el = document.querySelector(selector);
    if (el) return el;
    if (Date.now() - started > ms) throw new Error('timeout ' + selector);
    await new Promise((r) => setTimeout(r, 120));
  }
};`;

/** Click the first footer toggle (every settled turn has one). */
const EXPAND = `(async () => {
  ${WAIT_FOR}
  const toggle = await waitFor('[data-testid=turn-files-toggle]');
  toggle.click();
  await new Promise((r) => setTimeout(r, 250));
  return true;
})()`;

/** Expand, then open the inline diff of math.ts. */
const INLINE_DIFF = `(async () => {
  ${WAIT_FOR}
  (await waitFor('[data-testid=turn-files-toggle]')).click();
  await new Promise((r) => setTimeout(r, 250));
  (await waitFor('[data-testid="turn-file-src/math.ts"]')).click();
  return true;
})()`;

/** Expand, then Undo the file the workspace changed since (the force prompt). */
const CONFLICT = `(async () => {
  ${WAIT_FOR}
  (await waitFor('[data-testid=turn-files-toggle]')).click();
  await new Promise((r) => setTimeout(r, 250));
  (await waitFor('[data-testid="turn-file-undo-src/math.ts"]')).click();
  return true;
})()`;

/** scenario, shell, width, height, shot name, action, settle ms */
const SHOTS = [
  ['turn-files', 'desktop', 1280, 820, 'collapsed', undefined],
  ['turn-files', 'desktop', 1280, 820, 'expanded', EXPAND],
  ['turn-files', 'desktop', 1280, 820, 'diff', INLINE_DIFF],
  ['turn-files-undo', 'desktop', 1280, 820, 'undone', EXPAND],
  ['turn-files-conflict', 'desktop', 1280, 820, 'conflict', CONFLICT],
  ['turn-files', 'phone', 412, 915, 'collapsed', undefined],
  ['turn-files', 'phone', 412, 915, 'expanded', EXPAND],
];

const version = await (await fetch(`${CDP}/json/version`)).json();
const browser = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  browser.onopen = resolve;
  browser.onerror = reject;
});

let seq = 0;
const pending = new Map();
browser.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
  }
};
function send(method, params = {}, sessionId, timeoutMs = 20000) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`${method} timed out`));
    }, timeoutMs);
    pending.set(id, {
      resolve: (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      reject: (e) => {
        clearTimeout(timer);
        reject(e);
      },
    });
    browser.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Bring the tab to front and grab a frame; retries when the browser is busy. */
async function shot(sessionId) {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      await send('Page.bringToFront', {}, sessionId);
      const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId);
      return data;
    } catch (error) {
      if (attempt === 2) throw error;
      await sleep(900);
    }
  }
  return undefined;
}

/** The app is up: the splash gone and the shell on screen. */
const READY = `(() => {
  const splash = document.getElementById('splash');
  const up = splash === null || splash.classList.contains('splash-out') || getComputedStyle(splash).opacity === '0';
  // every A27 shot needs a mounted footer: it exists only once the turn is on
  // screen and the service answered.
  return up && !!document.querySelector('[data-testid=turn-files]');
})()`;

async function settled(sessionId) {
  for (let i = 0; i < 60; i++) {
    const r = await send('Runtime.evaluate', { expression: READY, returnByValue: true }, sessionId);
    if (r.result.value === true) return true;
    await sleep(250);
  }
  return false;
}

mkdirSync(OUT, { recursive: true });
// Leftovers from an earlier aborted run would keep the browser busy.
for (const target of (await send('Target.getTargets')).targetInfos ?? []) {
  if (target.type === 'page' && target.url.includes('127.0.0.1:1450')) await send('Target.closeTarget', { targetId: target.targetId }).catch(() => {});
}
// Warm the dev server first (Vite's first load optimizes deps and reloads, which
// would otherwise land a screenshot on the splash).
{
  const { targetId } = await send('Target.createTarget', { url: `${APP}/?scenario=turn-files&shell=desktop#/s/pc-coffee/s-files`, background: true });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  await send('Page.enable', {}, sessionId);
  await settled(sessionId);
  await sleep(1500);
  await send('Target.closeTarget', { targetId });
  await sleep(400);
}

for (const [scenario, shell, width, height, name, action, settle = 900] of SHOTS) {
  // A CDP session can be dropped right after the previous target closed: retry once.
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      const url = `${APP}/?scenario=${scenario}&shell=${shell}#/s/pc-coffee/s-files`;
      const { targetId } = await send('Target.createTarget', { url, background: true });
      const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
      await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 2, mobile: shell === 'phone' }, sessionId);
      await send('Page.enable', {}, sessionId);
      await settled(sessionId);
      await sleep(2400);
      if (action) {
        const out = await send('Runtime.evaluate', { expression: action, awaitPromise: true, returnByValue: true }, sessionId);
        if (out.exceptionDetails) console.warn(`${name}-${shell}: ${out.exceptionDetails.exception?.description ?? out.exceptionDetails.text}`);
        await sleep(settle);
      }
      const data = await shot(sessionId);
      const file = `a27-turn-files-${name}-${shell}.png`;
      writeFileSync(`${OUT}/${file}`, Buffer.from(data, 'base64'));
      console.log(file);
      await send('Target.closeTarget', { targetId });
      await sleep(500);
      break;
    } catch (error) {
      if (attempt === 1) console.warn(`skip ${name}-${shell}: ${error.message}`);
      else await sleep(1200);
    }
  }
}
browser.close();
