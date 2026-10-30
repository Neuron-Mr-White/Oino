/**
 * WP-A24 capture script: drives the owner's CloakBrowser over raw CDP (Node's
 * built-in WebSocket) against the capture harness on port 1450.
 * Usage: npx vite --config vite.config.capture.ts  # then
 *        node docs/dev/v2/design/shots/app/a24-capture.mjs
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = process.env.UNIPI_CDP ?? 'http://127.0.0.1:45859';
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;
/** The page is ready when its own content is on screen (a cold vite graph and
 *  a concurrent HMR update both race a fixed sleep). */
const READY_TIMEOUT_MS = 30000;
const AFTER_READY_MS = 700;

/** `ready` is a selector only the finished page has (the mock host + scenario
 *  setup must have run); [name, scenario, shell, width, height, route, ready] */
const SHOTS = [
  ['nav', 'settings-nav', 'desktop', 1280, 820, '#/settings/appearance', '.settings-stepper'],
  ['nav', 'settings-nav', 'desktop', 1440, 900, '#/settings/appearance', '.settings-stepper'],
  ['nav', 'settings-nav', 'phone', 412, 915, '#/settings', '.settings-phone-group'],
  ['search', 'settings-search', 'desktop', 1280, 820, '#/settings', '.settings-search-hit'],
  ['search', 'settings-search', 'desktop', 1440, 900, '#/settings', '.settings-search-hit'],
  ['search', 'settings-search', 'phone', 412, 915, '#/settings', '.settings-search-hit'],
  ['extensions', 'settings-extensions', 'desktop', 1280, 820, '#/settings/extensions', '[data-testid=extension-crash-notice]'],
  ['extensions', 'settings-extensions', 'phone', 412, 915, '#/settings/extensions', '[data-testid=extension-crash-notice]'],
  ['safe-mode', 'settings-safe-mode', 'desktop', 1280, 820, '#/settings/extensions', '[data-testid=safe-mode-banner]'],
  ['safe-mode', 'settings-safe-mode', 'phone', 412, 915, '#/settings/extensions', '[data-testid=safe-mode-banner]'],
  ['about', 'settings-nav', 'desktop', 1280, 820, '#/settings/about', '[data-testid=update-card]'],
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
    // A dead socket (the browser restarted) must fail the shot, not hang it.
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
async function shoot(name, scenario, shell, width, height, route, ready) {
  const url = `${APP}/?scenario=${scenario}&shell=${shell}${route}`;
  const { targetId } = await send('Target.createTarget', { url, background: true });
  try {
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
    await sleep(AFTER_READY_MS);
    const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId);
    const file = `a24-settings-${name}-${shell}${width === 1440 ? '-1440' : ''}.png`;
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

for (const shot of SHOTS) {
  // A concurrent HMR update from another worker can kill a page mid-boot;
  // one retry with a fresh target settles it.
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    if (await shoot(...shot)) break;
    if (attempt === 3) throw new Error(`page never became ready: ${shot.join(' ')}`);
    console.log(`retrying ${shot[0]} ${shot[2]} (attempt ${attempt + 1})`);
  }
}
browser.close();
