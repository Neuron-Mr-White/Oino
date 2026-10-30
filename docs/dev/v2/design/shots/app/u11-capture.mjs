/**
 * WP-U11 capture script: drives the PC browser over raw CDP (Node's built-in
 * WebSocket) against the capture harness on port 1450 — the OSS-2 form engine
 * inside the one Settings screen, desktop + phone.
 *
 * Usage: npx vite --config vite.config.capture.ts   # port 1450
 *        node docs/dev/v2/design/shots/app/u11-capture.mjs
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = process.env.UNIPI_CDP ?? `http://127.0.0.1:${process.env.AGENT_BROWSER_CDP ?? '45859'}`;
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;
const READY_TIMEOUT_MS = 30000;
const AFTER_READY_MS = 900;

/** [name, scenario, shell, width, height, route, ready, focus?] — `focus` scrolls
 *  the shot to that element before capturing (the form is longer than a screen). */
const SHOTS = [
  ['extensions', 'settings-form-list', 'desktop', 1440, 900, '#/settings/extensions', '[data-testid="ns-memory"]'],
  ['extensions', 'settings-form-list', 'phone', 412, 915, '#/settings/extensions', '[data-testid="ns-memory"]'],
  ['form', 'settings-form', 'desktop', 1440, 900, '#/settings/extensions', '#field-memory-topK'],
  ['form', 'settings-form', 'phone', 412, 915, '#/settings/extensions', '#field-memory-topK'],
  ['form-lists', 'settings-form', 'desktop', 1440, 900, '#/settings/extensions', '#field-memory-weights', '#field-memory-weights'],
  ['form-lists', 'settings-form', 'phone', 412, 915, '#/settings/extensions', '#field-memory-weights', '#field-memory-weights'],
  ['form-advanced', 'settings-form-advanced', 'desktop', 1440, 900, '#/settings/extensions', '[data-testid="page-body-advanced-page"]', '#field-memory-retention'],
  ['form-advanced', 'settings-form-advanced', 'phone', 412, 915, '#/settings/extensions', '[data-testid="page-body-advanced-page"]', '#field-memory-retention'],
  ['agent', 'settings-form-agent', 'desktop', 1440, 900, '#/settings/agent', '#field-oino_model-defaultModel'],
  ['agent', 'settings-form-agent', 'phone', 412, 915, '#/settings/agent', '#field-oino_model-defaultModel'],
];


/** One CDP connection per round: the owner's browser restarts (an update, a
 *  concurrent capture) kill the socket mid-run, so a failed shot is re-tried on
 *  a fresh connection instead of ending the capture. */
async function connect() {
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
      try {
        browser.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
      } catch (error) {
        clearTimeout(timer);
        pending.delete(id);
        reject(error);
      }
    });
  }
  browser.onclose = () => {
    for (const { reject } of pending.values()) reject(new Error('CDP socket closed'));
    pending.clear();
  };
  return { browser, send };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

mkdirSync(OUT, { recursive: true });
async function shoot(send, name, scenario, shell, width, height, route, ready, focus) {
  const url = `${APP}/?scenario=${scenario}&shell=${shell}${route}`;
  let targetId;
  try {
    ({ targetId } = await send('Target.createTarget', { url, background: true }));
    const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 2, mobile: shell === 'phone' }, sessionId);
    await send('Page.enable', {}, sessionId);
    const started = Date.now();
    let ok = false;
    while (Date.now() - started < READY_TIMEOUT_MS) {
      await sleep(1000);
      const { result } = await send('Runtime.evaluate', {
        expression: `!!document.querySelector(${JSON.stringify(ready)}) && !document.getElementById('splash') && !document.body.innerText.includes('UniPi hit a problem')`,
        returnByValue: true,
      }, sessionId);
      if (result.value === true) { ok = true; break; }
    }
    if (!ok) return false;
    if (focus) {
      await send('Runtime.evaluate', {
        expression: `document.querySelector(${JSON.stringify(focus)})?.scrollIntoView({ block: 'center' })`,
      }, sessionId);
      await sleep(400);
    }
    await sleep(AFTER_READY_MS);
    const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId);
    const file = `u11-${name}-${shell}.png`;
    writeFileSync(`${OUT}/${file}`, Buffer.from(data, 'base64'));
    console.log(file);
    return true;
  } catch (error) {
    console.log(`attempt failed: ${String(error)}`);
    return false;
  } finally {
    if (targetId) await send('Target.closeTarget', { targetId }).catch(() => {});
    await sleep(300);
  }
}

let pendingShots = [...SHOTS];
for (let round = 1; round <= 4 && pendingShots.length > 0; round += 1) {
  const { browser, send } = await connect();
  const failed = [];
  for (const shot of pendingShots) {
    let done = false;
    for (let attempt = 1; attempt <= 2 && !done; attempt += 1) {
      done = await shoot(send, ...shot);
      if (!done) console.log(`retrying ${shot[0]} ${shot[2]} (attempt ${attempt + 1})`);
    }
    if (!done) failed.push(shot);
  }
  browser.close();
  pendingShots = failed;
  if (pendingShots.length) {
    console.log(`reconnecting for ${pendingShots.length} shot(s)`);
    await sleep(2000);
  }
}
if (pendingShots.length) throw new Error(`pages never became ready: ${pendingShots.map((shot) => shot.join(' ')).join(' | ')}`);
