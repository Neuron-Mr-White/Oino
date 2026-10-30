/**
 * WP-A13 capture script: drives the owner's CloakBrowser over raw CDP (Node's
 * built-in WebSocket) against the capture harness on port 1450.
 *
 *   npx vite --config vite.config.capture.ts
 *   node docs/dev/v2/design/shots/app/a13-capture.mjs
 *
 * Shots: the composer's attachment states (desktop + phone), the image editor
 * with an ellipse drawn (desktop + phone), and the toolbar full vs. folded at
 * a narrow width (Z-64).
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = process.env.UNIPI_CDP ?? 'http://127.0.0.1:45859';
const APP = process.env.UNIPI_CAPTURE ?? 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;

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
    // A background tab that navigates (route restore) answers pending calls
    // with this; it is not fatal for a capture run.
    if (msg.error && /navigated or closed/i.test(msg.error.message)) resolve(undefined);
    else msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
  }
};
function send(method, params = {}, sessionId) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`${method} timed out`));
    }, 20_000);
    pending.set(id, {
      resolve: (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      reject: (error) => {
        clearTimeout(timer);
        reject(error);
      },
    });
    browser.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Runs an expression in the page and returns its JSON value. */
async function evaluate(sessionId, expression) {
  const { result } = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, sessionId);
  return result?.value;
}

/**
 * Clicks an element in the page. The capture tabs are background tabs, where
 * Chrome does not deliver `Input.dispatchMouseEvent`, so the click is a DOM
 * one (`el.click()` — every target here is a `<button>`).
 */
async function click(sessionId, selector) {
  const ok = await evaluate(
    sessionId,
    `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.click(); return true; })()`,
  );
  if (!ok) throw new Error(`no element for ${selector}`);
}

/** Polls until `selector` exists (the app boots the mock host first). */
async function waitFor(sessionId, selector, timeoutMs = 20_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const present = await evaluate(sessionId, `!!document.querySelector(${JSON.stringify(selector)})`);
    if (present) return true;
    await sleep(250);
  }
  return false;
}

/** Drags on the image editor's canvas (an ellipse, D-5) with pointer events. */
async function ellipse(sessionId, from = [0.28, 0.3], to = [0.72, 0.68], shift = false) {
  const ok = await evaluate(
    sessionId,
    `(() => {
      const el = document.querySelector('.img-edit-canvas');
      if (!el) return false;
      const r = el.getBoundingClientRect();
      const at = ([fx, fy]) => ({ x: r.left + r.width * fx, y: r.top + r.height * fy, clientX: r.left + r.width * fx, clientY: r.top + r.height * fy });
      el.setPointerCapture = () => {};
      const fire = (type, p, buttons) => el.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0, buttons, shiftKey: ${shift}, ...p }));
      const a = at(${JSON.stringify(from)});
      const b = at(${JSON.stringify(to)});
      fire('pointerdown', a, 1);
      for (let i = 1; i <= 8; i++) fire('pointermove', at([${from[0]} + ((${to[0]} - ${from[0]}) * i) / 8, ${from[1]} + ((${to[1]} - ${from[1]}) * i) / 8]), 1);
      fire('pointerup', b, 0);
      return true;
    })()`,
  );
  if (!ok) throw new Error('no image editor canvas');
}

const SHOTS = [
  // Z-60 states: ready screenshot + ready file, one uploading at 42 %, one failed.
  { name: 'composer-attachments', shell: 'desktop', width: 1440, height: 900 },
  { name: 'composer-attachments', shell: 'phone', width: 412, height: 915 },
  // D-5: the editor with an ellipse.
  {
    name: 'image-editor-ellipse',
    shell: 'desktop',
    width: 1440,
    height: 900,
    actions: async (s) => {
      await click(s, '[data-testid="attachment-chip"] .att-thumb');
      await sleep(900);
      await click(s, '[data-testid="tool-ellipse"]');
      await ellipse(s);
      await sleep(400);
    },
  },
  {
    name: 'image-editor-ellipse',
    shell: 'phone',
    width: 412,
    height: 915,
    actions: async (s) => {
      await click(s, '[data-testid="attachment-chip"] .att-thumb');
      await sleep(900);
      await click(s, '[data-testid="tool-ellipse"]');
      await ellipse(s, [0.22, 0.38], [0.78, 0.52]);
      await sleep(400);
    },
  },
  // Z-64: the toolbar full (1440) and folded at a narrow width (720), plus phone.
  { name: 'composer-narrow', shell: 'desktop', width: 1440, height: 900, suffix: 'full' },
  { name: 'composer-narrow', shell: 'desktop', width: 720, height: 820, suffix: 'narrow' },
  { name: 'composer-narrow', shell: 'phone', width: 412, height: 915 },
];

mkdirSync(OUT, { recursive: true });
for (const shot of SHOTS) {
  const url = `${APP}/?scenario=${shot.name}&shell=${shot.shell}#/s/pc-coffee/attachments`;
  console.log(`… ${shot.name} ${shot.shell} ${shot.width}`);
  const { targetId } = await send('Target.createTarget', { url, background: true });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  await send('Emulation.setDeviceMetricsOverride', { width: shot.width, height: shot.height, deviceScaleFactor: 2, mobile: shot.shell === 'phone' }, sessionId);
  await send('Page.enable', {}, sessionId);
  await send('Runtime.enable', {}, sessionId);
  // The session route can lose a race with the host list loading (the shell
  // falls back to /chat); boot first, then set the hash until the composer is
  // there.
  await sleep(3500);
  let ready = false;
  for (let attempt = 0; attempt < 6 && !ready; attempt++) {
    if (attempt > 0) await sleep(800);
    await evaluate(sessionId, `location.hash = '#/s/pc-coffee/attachments'`);
    ready = await waitFor(sessionId, '[data-testid="composer-toolbar"]', 6000);
  }
  if (!ready) {
    const state = await evaluate(sessionId, `({ hash: location.hash, text: document.body.innerText.slice(0, 80), toolbar: !!document.querySelector('[data-testid="composer-toolbar"]'), compose: !!document.querySelector('[data-testid="tl-composer"]') })`);
    console.warn('  composer never rendered', JSON.stringify(state));
  }
  await sleep(1200); // settle: attachments, topics, layout
  try {
    if (shot.actions) await shot.actions(sessionId);
  } catch (error) {
    console.warn(`  actions failed: ${error.message}`);
  }
  const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId);
  const name = `a13-${shot.name}${shot.suffix ? `-${shot.suffix}` : ''}-${shot.shell}.png`;
  writeFileSync(`${OUT}/${name}`, Buffer.from(data, 'base64'));
  console.log(name);
  await send('Target.closeTarget', { targetId });
}
browser.close();
