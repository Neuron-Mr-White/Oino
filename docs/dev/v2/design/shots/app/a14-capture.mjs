/**
 * WP-A14 capture script: drives the owner's CloakBrowser over raw CDP (Node's
 * built-in WebSocket) against the capture harness on port 1450.
 * Usage: npx vite --config vite.config.capture.ts  # then
 *        node docs/dev/v2/design/shots/app/a14-capture.mjs
 *
 * Each shot is [name, scenario, shell, width, height, route, ready, action?,
 * readyAfter?]. `action` runs in the page after `ready` (a click, a typed
 * search, a Ctrl+M key event), then `readyAfter` is awaited before the shot —
 * that is how the menus, the shortcut and the Context tab are proven, not
 * just described.
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = process.env.UNIPI_CDP ?? 'http://127.0.0.1:45859';
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;
const READY_TIMEOUT_MS = 30000;
const AFTER_READY_MS = 700;
const AFTER_ACTION_MS = 600;
/** Menus paint in a compositor layer: give the capture a beat after its action
 *  (a background tab composites late, and a shot taken too early shows the
 *  panel faded into the conversation underneath). */
const SETTLE_MS = 1400;

const typeSearch = (value) => {
  const input = document.querySelector('[data-testid=model-search]');
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
};
const click = (selector) => document.querySelector(selector).click();

const SHOTS = [
  ['model-picker', 'model-picker', 'desktop', 1280, 820, '', '[data-testid=model-picker-list]'],
  ['model-picker', 'model-picker', 'phone', 412, 915, '', '[data-testid=model-picker-list]'],
  [
    'model-picker-search',
    'model-picker',
    'desktop',
    1280,
    820,
    '',
    '[data-testid=model-picker-list]',
    `(${typeSearch})('gem')`,
  ],
  [
    'model-picker-pick',
    'model-picker',
    'desktop',
    1280,
    820,
    '',
    '[data-testid=model-picker-list]',
    "[...document.querySelectorAll('.mp-row')].find((r) => r.textContent.includes('GPT-5') && !r.textContent.includes('mini')).click()",
    '.mp-apply',
  ],
  ['model-fusion', 'model-fusion', 'desktop', 1280, 820, '', '[data-testid=model-picker-fusion]'],
  ['model-fusion', 'model-fusion', 'phone', 412, 915, '', '[data-testid=model-picker-fusion]'],
  [
    'model-picker-shortcut',
    'thinking-chip',
    'desktop',
    1280,
    820,
    '',
    '[data-testid=model-chip]',
    "window.dispatchEvent(new KeyboardEvent('keydown',{key:'m',code:'KeyM',ctrlKey:true,bubbles:true}))",
    '[data-testid=model-picker-list]',
  ],
  ['thinking-chip', 'thinking-chip', 'desktop', 1280, 820, '', '[data-testid=thinking-chip]'],
  ['thinking-chip', 'thinking-chip', 'phone', 412, 915, '', '[data-testid=thinking-chip]'],
  [
    'thinking-menu',
    'thinking-chip',
    'desktop',
    1280,
    820,
    '',
    '[data-testid=thinking-chip]',
    `(${click})('[data-testid=thinking-chip]')`,
    '[role=menu]',
  ],
  [
    'context-ring',
    'thinking-chip',
    'desktop',
    1280,
    820,
    '',
    '[data-testid=context-ring]',
    `(${click})('[data-testid=context-ring]')`,
    '[data-testid=sm-context]',
  ],
  ['mode-chip', 'mode-chip', 'desktop', 1280, 820, '', '[data-testid=mode-chip]'],
  ['mode-chip', 'mode-chip', 'phone', 412, 915, '', '[data-testid=mode-chip]'],
  [
    'mode-menu',
    'mode-chip',
    'desktop',
    1280,
    820,
    '',
    '[data-testid=mode-chip]',
    `(${click})('[data-testid=mode-chip]')`,
    '[data-testid=mode-start-goal]',
  ],
];

const version = await (await fetch(`${CDP}/json/version`)).json();
const browser = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { browser.onopen = resolve; browser.onerror = reject; });

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
function send(method, params = {}, sessionId) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`CDP timeout: ${method}`));
    }, 20000);
    pending.set(id, {
      resolve: (value) => { clearTimeout(timer); resolve(value); },
      reject: (error) => { clearTimeout(timer); reject(error); },
    });
    browser.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
}
browser.onclose = () => {
  for (const { reject } of pending.values()) reject(new Error('CDP socket closed'));
  pending.clear();
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

mkdirSync(OUT, { recursive: true });

async function waitForSelector(sessionId, selector, timeout = READY_TIMEOUT_MS) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    await sleep(500);
    const { result } = await send('Runtime.evaluate', {
      expression: `!!document.querySelector(${JSON.stringify(selector)}) && !document.getElementById('splash') && !document.body.innerText.includes('UniPi hit a problem')`,
      returnByValue: true,
    }, sessionId);
    if (result.value === true) return true;
  }
  return false;
}

async function shoot(name, scenario, shell, width, height, route, ready, action, readyAfter) {
  const url = `${APP}/?scenario=${scenario}&shell=${shell}${route}`;
  const { targetId } = await send('Target.createTarget', { url, background: true });
  try {
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 2, mobile: shell === 'phone' }, sessionId);
    await send('Page.enable', {}, sessionId);
    if (!(await waitForSelector(sessionId, ready))) return false;
    await sleep(AFTER_READY_MS);
    if (action) {
      await send('Runtime.evaluate', { expression: action, awaitPromise: false }, sessionId);
      await sleep(AFTER_ACTION_MS);
      if (readyAfter && !(await waitForSelector(sessionId, readyAfter, 8000))) return false;
      await sleep(SETTLE_MS);
    }
    const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId);
    const file = `a14-${name}-${shell}.png`;
    writeFileSync(`${OUT}/${file}`, Buffer.from(data, 'base64'));
    console.log(file);
    return true;
  } catch (error) {
    // A page killed by a concurrent HMR update loses its target mid-call.
    console.log(`attempt failed: ${String(error)}`);
    return false;
  } finally {
    await send('Target.closeTarget', { targetId }).catch(() => {});
    await sleep(300);
  }
}

// `A14_ONLY=mode-menu` re-shoots one entry (iterating on a single pane).
const only = process.env.A14_ONLY;
for (const shot of only ? SHOTS.filter((s) => s[0] === only) : SHOTS) {
  // A concurrent HMR update from another worker can kill a page mid-boot;
  // one retry with a fresh target settles it.
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    if (await shoot(...shot)) break;
    if (attempt === 3) throw new Error(`page never became ready: ${shot.join(' ')}`);
    console.log(`retrying ${shot[0]} ${shot[2]} (attempt ${attempt + 1})`);
  }
}
browser.close();
