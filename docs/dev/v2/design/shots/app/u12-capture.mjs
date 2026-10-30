/**
 * WP-U12 capture script: drives the owner's CloakBrowser over raw CDP (Node's
 * built-in WebSocket) against the capture harness on port 1450 (start it with
 * `npx vite --config vite.config.capture.ts`).
 *
 * Usage: node docs/dev/v2/design/shots/app/u12-capture.mjs
 *
 * Desktop 1280x820 and phone 412x915 (§13.6), scenarios from
 * `src/dev/mock/scenarios/reserved.ts`. Each shot is one reserved-routing
 * behaviour: the native pane tab a `host/git` entry focuses, the owner fallback
 * of a row the build does not implement yet, the rail/bottom-nav items that
 * follow their owner (the phone's Chat tab shows the bottom nav), and the
 * phone's Files layer opened by `host/files`.
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = `http://127.0.0.1:${process.env.AGENT_BROWSER_CDP ?? '45859'}`;
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;

/** scenario, shell, width, height, shot name, hash, click script (after settle), settle ms */
const SHOTS = [
  ['u12-native', 'desktop', 1280, 820, 'native', '#/s/pc-coffee/s001'],
  ['u12-fallback', 'desktop', 1280, 820, 'fallback', '#/s/pc-coffee/s001'],
  ['u12-owner-off', 'desktop', 1280, 820, 'owner-off', '#/s/pc-coffee/s001'],
  ['u12-phone', 'phone', 412, 915, 'files', '#/s/pc-coffee/s001'],
  ['u12-nav', 'phone', 412, 915, 'nav', '#/chat'],
  ['u12-phone-off', 'phone', 412, 915, 'owner-off', '#/chat'],
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

mkdirSync(OUT, { recursive: true });
// Leftovers from an earlier aborted run would keep the browser busy.
for (const target of (await send('Target.getTargets')).targetInfos ?? []) {
  if (target.type === 'page' && target.url.includes('127.0.0.1:1450')) await send('Target.closeTarget', { targetId: target.targetId }).catch(() => {});
}
for (const [scenario, shell, width, height, name, hash, action, settle] of SHOTS) {
  try {
    const url = `${APP}/?scenario=${scenario}&shell=${shell}${hash}`;
    const { targetId } = await send('Target.createTarget', { url, background: true });
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 2, mobile: shell === 'phone' }, sessionId);
    await send('Page.enable', {}, sessionId);
    // Wait for the shell to be on screen (the splash is up until the mock host
    // answers), then let the <= 1 s registry patch and the gate fetch settle.
    const ready = `!!document.querySelector('[data-testid=workspace],[data-testid=files-layer],[data-testid=chat-tab]')`;
    for (let i = 0; i < 48; i++) {
      const r = await send('Runtime.evaluate', { expression: ready, returnByValue: true }, sessionId);
      if (r.result.value === true) break;
      await sleep(250);
    }
    await sleep(3200);
    if (action) {
      const expression = `(() => { (${action.toString()})(); return true; })()`;
      await send('Runtime.evaluate', { expression, awaitPromise: true }, sessionId);
      await sleep(settle ?? 900);
    }
    const data = await shot(sessionId);
    const file = `u12-${name}-${shell}.png`;
    writeFileSync(`${OUT}/${file}`, Buffer.from(data, 'base64'));
    console.log(file);
    await send('Target.closeTarget', { targetId });
    await sleep(300);
  } catch (error) {
    console.warn(`skip ${name}-${shell}: ${error.message}`);
  }
}
browser.close();
