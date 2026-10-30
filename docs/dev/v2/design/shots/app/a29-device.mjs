/**
 * WP-A29 on-device checks (acceptance): drives Chrome on the plugged-in
 * Android phone over `chrome_devtools_remote` (adb forward :9222), with
 * `adb reverse tcp:1450` pointing the phone at the capture harness.
 *
 *   adb reverse tcp:1450 tcp:1450
 *   adb forward tcp:9222 localabstract:chrome_devtools_remote
 *   node docs/dev/v2/design/shots/app/a29-device.mjs
 *
 * Records: §3.6 perf rows (open-to-paint, scroll frames, long tasks, DOM
 * nodes, session switch, streaming input latency), the Back stack (Escape =
 * the app plugin's Back callback; `input keyevent 4` = the OS route-back)
 * and the IME/keyboard inset behaviour. Prints JSON lines to stdout; the
 * summary goes into docs/dev/v2/qa/app-perf.md.
 */
import { execFileSync as sh } from 'node:child_process';

const targets = await (await fetch('http://127.0.0.1:9222/json')).json();
const page = targets.find((t) => t.type === 'page');
if (!page) throw new Error('no page target on the phone');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r, j) => {
  ws.onopen = r;
  ws.onerror = j;
});
let seq = 0;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
  }
};
function send(method, params = {}, timeoutMs = 30000) {
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
    ws.send(JSON.stringify({ id, method, params }));
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const evalIn = async (expression) => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result?.value;
const adb = (args) => sh('adb', args.split(' '), { encoding: 'utf8' }).trim();
const out = (k, v) => console.log(JSON.stringify({ [k]: v }));

await send('Page.enable');
await send('Runtime.enable');
const APP = 'http://127.0.0.1:1450';
const device = adb('shell getprop ro.product.model');
out('device', device);

/** Time (in-page clock) until `selector` matches, from an already-running navigation. */
const untilSelector = (selector, timeout = 15000) =>
  `new Promise((resolve) => {
    const t0 = performance.now();
    const hit = () => document.querySelector(${JSON.stringify(selector)});
    if (hit()) return resolve(0);
    const mo = new MutationObserver(() => { if (hit()) { mo.disconnect(); resolve(performance.now() - t0); } });
    mo.observe(document.documentElement, { childList: true, subtree: true });
    setTimeout(() => { mo.disconnect(); resolve(-1); }, ${timeout});
  })`;

// ── 1. open: timeline-many (80 settled turns + live tail). The dev-server
// first load is not the budgeted number — measure a warm reload AND the
// in-app open from the Chat list (list → session has no stale .tl-turn).
await send('Page.navigate', { url: `${APP}/?scenario=timeline-many&shell=phone#/s/pc-coffee/s001` });
const openCold = await evalIn(untilSelector('[data-testid=tl] .tl-turn', 20000));
const paint = await evalIn(`(performance.getEntriesByType('paint').find(e => e.name === 'first-contentful-paint') || {}).startTime ?? -1`);
await sleep(1200);
const dom = await evalIn(`document.getElementsByTagName('*').length`);
const turns = await evalIn(`document.querySelectorAll('[data-testid=tl] .tl-turn, .tl-turn').length`);
// warm: reload the same URL (modules cached), then in-app open from /chat.
await send('Page.navigate', { url: `${APP}/?scenario=timeline-many&shell=phone#/s/pc-coffee/s001` });
const openWarm = await evalIn(untilSelector('[data-testid=tl] .tl-turn', 20000));
await sleep(800);
const openInApp = await evalIn(`(async () => {
  location.hash = '#/chat';
  await new Promise((r) => { const mo = new MutationObserver(() => { if (!document.querySelector('.tl-turn')) { mo.disconnect(); r(); } }); mo.observe(document.documentElement, { childList: true, subtree: true }); setTimeout(r, 3000); });
  const t0 = performance.now();
  location.hash = '#/s/pc-coffee/s001';
  await (${untilSelector('[data-testid=tl] .tl-turn', 8000)});
  return performance.now() - t0;
})()`);
out('open_80turns_ms', { cold: Math.round(openCold), warm: Math.round(openWarm), inApp: Math.round(openInApp) });
out('fcp_ms', Math.round(paint));
out('dom_nodes_80turns', dom);
out('turns_rendered', turns);

// ── 2. scroll frames + long tasks (synthesized touch scroll, ~4 s)
const scroll = await evalIn(`(async () => {
  const el = document.querySelector('[data-testid=tl]');
  if (!el) return { error: 'no scroller' };
  const longtasks = [];
  try { new PerformanceObserver((l) => longtasks.push(...l.getEntries().map((e) => e.duration))).observe({ entryTypes: ['longtask'] }); } catch {}
  const frames = [];
  let last = performance.now();
  let done = false;
  const tick = (t) => { frames.push(t - last); last = t; if (!done) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  const t0 = performance.now();
  while (performance.now() - t0 < 3800) {
    el.scrollTop = Math.max(0, el.scrollTop - 240 + Math.sin(performance.now() / 300) * 60);
    await new Promise((r) => setTimeout(r, 90));
  }
  done = true;
  frames.shift();
  const over50 = longtasks.filter((d) => d > 50).length;
  const over16 = frames.filter((d) => d > 20).length;
  frames.sort((a, b) => a - b);
  return { frames: frames.length, p50: frames[Math.floor(frames.length / 2)] ?? 0, p95: frames[Math.floor(frames.length * 0.95)] ?? 0, over20ms: over16, longtasks_over50: over50 };
})()`);
out('scroll', scroll);

// ── 3. session switch s001 → s002 → s001. Two-phase: wait for the old
// session's turns to unmount (keyed remount), then the new first turn.
const switchTo = (id) => `(async () => {
  const t0 = performance.now();
  location.hash = '#/s/pc-coffee/${id}';
  await new Promise((r) => { const mo = new MutationObserver(() => { if (!document.querySelector('.tl-turn')) { mo.disconnect(); r(); } }); mo.observe(document.documentElement, { childList: true, subtree: true }); setTimeout(r, 3000); });
  const t1 = performance.now();
  await (${untilSelector('[data-testid=tl] .tl-turn', 8000)});
  return { navMs: t1 - t0, paintMs: performance.now() - t0 };
})()`;
const sw1 = await evalIn(switchTo('s002'));
await sleep(400);
const sw2 = await evalIn(switchTo('s001'));
out('session_switch_ms', [Math.round(sw1.paintMs), Math.round(sw2.paintMs)]);

// ── 4. streaming input latency (live stream, 10 samples)
await send('Page.navigate', { url: `${APP}/?scenario=stream-basic&shell=phone&live=1&speed=1#/s/pc-coffee/s001` });
await evalIn(untilSelector('[data-testid=tl-composer]'));
await sleep(2500);
const latency = await evalIn(`(async () => {
  const field = document.querySelector('.tl-composer-field');
  if (!field) return { error: 'no field' };
  field.focus();
  const samples = [];
  for (let i = 0; i < 10; i++) {
    const t0 = performance.now();
    document.execCommand('insertText', false, 'x');
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    samples.push(performance.now() - t0);
    await new Promise((r) => setTimeout(r, 120));
  }
  field.textContent = '';
  return { samples: samples.map((s) => Math.round(s)), max: Math.round(Math.max(...samples)) };
})()`);
out('stream_input_latency', latency);

// ── 5. Back stack ───────────────────────────────────────────────────────
await send('Page.navigate', { url: `${APP}/?scenario=stream-basic&shell=phone#/s/pc-coffee/s001` });
await evalIn(untilSelector('[data-testid=conv-more]'));
await sleep(800);
const esc = async () => {
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
};
// menu → Git layer → Escape closes the layer, next Escape has nothing to eat
await evalIn(`document.querySelector('[data-testid=conv-more]').click()`);
await sleep(500);
await evalIn(`[...document.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.trim() === 'Git')?.click()`);
await sleep(1200);
const layerUp = await evalIn(`!!document.querySelector('[data-testid=git-view]')`);
await esc();
await sleep(600);
const layerDown = await evalIn(`!document.querySelector('[data-testid=git-view]')`);
// sheet: Tree
await evalIn(`document.querySelector('[data-testid=conv-more]')?.click()`);
await sleep(500);
await evalIn(`[...document.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.trim() === 'Tree')?.click()`);
await sleep(1200);
const sheetUp = await evalIn(`!!document.querySelector('[data-testid=session-tab-sheet]')`);
await esc();
await sleep(700);
const sheetDown = await evalIn(`!document.querySelector('[data-testid=session-tab-sheet]')`);
// OS Back (keyevent 4): conversation route → Chat tab (an open IME eats the
// first Back — standard Android; covered by the IME step below).
await evalIn(`location.hash = '#/chat'`);
await sleep(600);
await evalIn(`location.hash = '#/s/pc-coffee/s001'`);
await sleep(1200);
adb('shell input keyevent 4');
await sleep(900);
const backToChat = await evalIn(`location.hash`);
out('back_stack', { gitLayer: layerUp && layerDown, sheet: sheetUp && sheetDown, osBackHash: backToChat });

// ── 6. IME ──────────────────────────────────────────────────────────────
await evalIn(`location.hash = '#/s/pc-coffee/s001'`);
await sleep(1200);
const before = await evalIn(`({ vh: window.visualViewport?.height ?? innerHeight, vvh: getComputedStyle(document.documentElement).getPropertyValue('--vvh'), bottom: getComputedStyle(document.documentElement).getPropertyValue('--vv-bottom') })`);
const fieldBox = await evalIn(`(() => { const f = document.querySelector('.tl-composer-field'); if (!f) return null; f.scrollIntoView({ block: 'center' }); const r = f.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
if (fieldBox) {
  await send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: fieldBox.x, y: fieldBox.y, id: 1 }] });
  await send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [{ x: fieldBox.x, y: fieldBox.y, id: 1 }] });
}
await sleep(2200);
const after = await evalIn(`({ vh: window.visualViewport?.height ?? innerHeight, vvh: getComputedStyle(document.documentElement).getPropertyValue('--vvh'), bottom: getComputedStyle(document.documentElement).getPropertyValue('--vv-bottom'), focused: !!document.activeElement?.isContentEditable })`);
// Back while the IME is up closes the keyboard, not a layer.
adb('shell input keyevent 4');
await sleep(900);
const imeBack = await evalIn(`({ vh: window.visualViewport?.height ?? innerHeight, hash: location.hash })`);
out('ime', { before, after, afterBack: imeBack });
ws.close();
out('done', true);
