/**
 * WP-U10 capture script: drives the owner's CloakBrowser over raw CDP (Node's
 * built-in WebSocket) against the capture harness on port 1450.
 * Usage: node docs/dev/v2/design/shots/app/u10-capture.mjs
 *
 * Desktop 1280x820 and phone 412x915 (§13.6), scenarios from
 * `src/dev/mock/scenarios/slots.ts`. Two shots open a surface with a scripted
 * click (the palette with "Open <title>", the phone ⋯ "Panes" group).
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = `http://127.0.0.1:${process.env.AGENT_BROWSER_CDP ?? '45859'}`;
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;

/** scenario, shell, width, height, shot name, hash, click script (after settle), settle ms */
const SHOTS = [
  ['slots', 'desktop', 1280, 820, 'slots', '#/s/pc-coffee/s001'],
  ['slots-overlay', 'desktop', 1280, 820, 'overlay', '#/s/pc-coffee/s001'],
  ['slots-tray', 'desktop', 1280, 820, 'tray', '#/s/pc-coffee/s001'],
  ['slots-dialogs', 'desktop', 1280, 820, 'dialogs', '#/s/pc-coffee/s001'],
  [
    'slots',
    'desktop',
    1280,
    820,
    'palette',
    '#/s/pc-coffee/s001',
    () => {
      document.querySelector('[data-testid=sb-search]')?.click();
      setTimeout(() => {
        // "All" scope, so the Panels group (U10's "Open <title>") is listed
        [...document.querySelectorAll('.cmdk-scope')].find((b) => b.textContent.trim() === 'All')?.click();
        setTimeout(() => {
          const input = document.querySelector('[data-testid=command-center] input');
          if (!input) return;
          input.value = 'Open ';
          input.dispatchEvent(new InputEvent('input', { bubbles: true }));
        }, 120);
      }, 120);
    },
    1600,
  ],
  ['slots', 'phone', 412, 915, 'slots', '#/s/pc-coffee/s001'],
  ['slots-overlay', 'phone', 412, 915, 'overlay', '#/s/pc-coffee/s001'],
  ['slots-dialogs', 'phone', 412, 915, 'dialogs', '#/s/pc-coffee/s001'],
  [
    'slots',
    'phone',
    412,
    915,
    'panes',
    '#/s/pc-coffee/s001',
    () => document.querySelector('[data-testid=conv-more]')?.click(),
    1200,
  ],
  [
    'slots',
    'phone',
    412,
    915,
    'panelayer',
    '#/s/pc-coffee/s001',
    () => {
      document.querySelector('[data-testid=conv-more]')?.click();
      setTimeout(() => {
        const row = [...document.querySelectorAll('[role=menuitem]')].find((el) => /Scratch notes|Notes/.test(el.textContent ?? ''));
        (row ?? [...document.querySelectorAll('button')].find((el) => /Scratch notes|Notes/.test(el.textContent ?? '')))?.click();
      }, 250);
    },
    1600,
  ],
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
  // answers), then let the ≤ 1 s registry patch script and turn renders settle.
  const ready = `!!document.querySelector('[data-testid=workspace],[data-testid=ui-extpage],[data-testid=ext-page-missing],[data-testid=chat-tab]')`;
  for (let i = 0; i < 48; i++) {
    const r = await send('Runtime.evaluate', { expression: ready, returnByValue: true }, sessionId);
    if (r.result.value === true) break;
    await sleep(250);
  }
  await sleep(2600);
  if (action) {
    const expression = `(() => { (${action.toString()})(); return true; })()`;
    await send('Runtime.evaluate', { expression, awaitPromise: true }, sessionId);
    await sleep(settle ?? 800);
  }
  const data = await shot(sessionId);
  const file = `u10-${name}-${shell}.png`;
  writeFileSync(`${OUT}/${file}`, Buffer.from(data, 'base64'));
  console.log(file);
  await send('Target.closeTarget', { targetId });
  await sleep(300);
  } catch (error) {
    console.warn(`skip ${name}-${shell}: ${error.message}`);
  }
}
browser.close();
