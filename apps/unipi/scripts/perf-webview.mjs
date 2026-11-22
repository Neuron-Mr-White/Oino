#!/usr/bin/env node
/**
 * v3-perf P5 (spec §3 T9): webview render budget on the `perf-stream` mock scenario.
 *
 *   npx vite --config vite.config.capture.ts --port 1465 &
 *   node scripts/perf-webview.mjs [--url http://127.0.0.1:1465] [--chats 10] [--secs 10] [--width 1440] [--shot out.png] [--label before]
 *
 * `--width` under 900 opens the phone shell (mobile + touch emulation, 915 px tall), else the desktop shell.
 *
 * Headless Chromium (playwright-core; `CHROME` / `PLAYWRIGHT_CORE` env override the paths).
 * Main-thread ms per frame = CPU time (`tdur`; `--wall` = wall time, inflated on a loaded box)
 * of top-level tasks on the renderer main thread (trace `RunTask`),
 * bucketed into 16.67 ms vsync windows; p50 / p95 / max over the run (+ p95 per 10 s).
 * Also: long tasks (PerformanceObserver), DOM node count, JS heap, rAF frame intervals.
 */
import { createRequire } from 'node:module';
import { existsSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const URL_BASE = arg('url', 'http://127.0.0.1:1465');
const CHATS = Number(arg('chats', '10'));
const SECS = Number(arg('secs', '10'));
const SHOT = arg('shot');
const LABEL = arg('label', 'run');
const WIDTH = Number(arg('width', '1440'));
const PHONE = WIDTH < 900;

const require = createRequire(import.meta.url);
const pwPath = process.env.PLAYWRIGHT_CORE ?? `${homedir()}/.bun/install/cache/playwright-core@1.64.1@@@1`;
const { chromium } = require(pwPath);
const chromeDir = `${homedir()}/.agent-browser/browsers`;
const chrome = process.env.CHROME ?? (existsSync(chromeDir) ? `${chromeDir}/${readdirSync(chromeDir).sort().at(-1)}/chrome` : undefined);

const pct = (xs, p) => {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
};
const r1 = (x) => Math.round(x * 10) / 10;

const browser = await chromium.launch({ executablePath: chrome, headless: true, args: ['--enable-precise-memory-info', '--js-flags=--expose-gc'] });
const page = await (
  await browser.newContext({ viewport: { width: WIDTH, height: PHONE ? 915 : 900 }, ...(PHONE ? { isMobile: true, hasTouch: true } : {}) })
).newPage();
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
await page.addInitScript(() => {
  window.__LT__ = [];
  try {
    new PerformanceObserver((l) => {
      for (const e of l.getEntries()) window.__LT__.push(e.duration);
    }).observe({ type: 'longtask', buffered: true });
  } catch {}
});
await page.goto(`${URL_BASE}/#/s/pc-coffee/big?scenario=perf-stream&shell=${PHONE ? 'phone' : 'desktop'}&chats=${CHATS}`);
await page.waitForSelector('[data-testid="tl-tail"]', { timeout: 30_000 });
// Let snapshots land and the streams start (scenario starts drivers at +1.5 s), then warm up.
await page.waitForTimeout(Number(arg('warm', '4')) * 1000);

const cdp = await page.context().newCDPSession(page);
await page.evaluate(() => {
  window.__LT__ = [];
  window.__FR__ = [];
  let last = performance.now();
  const loop = (t) => {
    window.__FR__.push(t - last);
    last = t;
    if (window.__FR_ON__) requestAnimationFrame(loop);
  };
  window.__FR_ON__ = true;
  requestAnimationFrame(loop);
});
const events = [];
cdp.on('Tracing.dataCollected', (d) => events.push(...d.value));
const done = new Promise((res) => cdp.once('Tracing.tracingComplete', res));
const PROFILE = process.argv.includes('--profile');
const CPU = !process.argv.includes('--wall');
if (PROFILE) {
  await cdp.send('Profiler.enable');
  await cdp.send('Profiler.setSamplingInterval', { interval: 200 });
  await cdp.send('Profiler.start');
}
await cdp.send('Tracing.start', { categories: 'devtools.timeline,disabled-by-default-devtools.timeline', transferMode: 'ReportEvents' });
await page.waitForTimeout(SECS * 1000);
await cdp.send('Tracing.end');
let hot;
if (PROFILE) {
  const { profile } = await cdp.send('Profiler.stop');
  const self = new Map();
  const dt = new Map();
  profile.samples.forEach((id, i) => dt.set(id, (dt.get(id) ?? 0) + (profile.timeDeltas[i] ?? 0)));
  for (const n of profile.nodes) {
    const f = n.callFrame;
    const k = `${f.functionName || '(anon)'} ${f.url.split('/').slice(-2).join('/')}:${f.lineNumber + 1}`;
    self.set(k, (self.get(k) ?? 0) + (dt.get(n.id) ?? 0) / 1000);
  }
  // Inclusive time per function (a frame counted once per sample stack).
  const parent = new Map();
  for (const n of profile.nodes) for (const c of n.children ?? []) parent.set(c, n.id);
  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const key = (n) => `${n.callFrame.functionName || '(anon)'} ${n.callFrame.url.split('/').slice(-2).join('/')}:${n.callFrame.lineNumber + 1}`;
  const incl = new Map();
  for (const [id, t] of dt) {
    const seen = new Set();
    for (let cur = id; cur !== undefined; cur = parent.get(cur)) {
      const k = key(byId.get(cur));
      if (seen.has(k)) continue;
      seen.add(k);
      incl.set(k, (incl.get(k) ?? 0) + t / 1000);
    }
  }
  globalThis.__incl = [...incl.entries()].filter(([k]) => k.includes('/src/') || /\.tsx?/.test(k)).sort((a, b) => b[1] - a[1]).slice(0, 45).map(([k, v]) => `${(v / SECS).toFixed(1)} ms/s  ${k}`);
  hot = [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 25).map(([k, v]) => `${(v / SECS).toFixed(1)} ms/s  ${k}`);
}
await done;
const page_ = await page.evaluate(() => {
  window.__FR_ON__ = false;
  return {
    longTasks: window.__LT__,
    frames: window.__FR__.slice(1),
    dom: document.getElementsByTagName('*').length,
    heap: performance.memory ? performance.memory.usedJSHeapSize : 0,
    tailText: document.querySelector('[data-testid="tl-tail"]')?.textContent?.length ?? 0,
  };
});
if (SHOT) await page.screenshot({ path: SHOT });

// The renderer main thread: the thread that ran the most RunTask time (CrRendererMain).
const names = new Map();
for (const e of events) if (e.ph === 'M' && e.name === 'thread_name') names.set(`${e.pid}:${e.tid}`, e.args.name);
const tasks = events.filter((e) => e.ph === 'X' && e.name === 'RunTask' && names.get(`${e.pid}:${e.tid}`) === 'CrRendererMain');
const byThread = new Map();
for (const t of tasks) byThread.set(`${t.pid}:${t.tid}`, (byThread.get(`${t.pid}:${t.tid}`) ?? 0) + t.dur);
const main = [...byThread.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
const mt = tasks.filter((t) => `${t.pid}:${t.tid}` === main);
const t0 = Math.min(...mt.map((t) => t.ts));
const t1 = Math.max(...mt.map((t) => t.ts + t.dur));
const FRAME = 16_667;
const buckets = new Float64Array(Math.ceil((t1 - t0) / FRAME) + 1);
for (const t of mt) {
  // Spread a task over the vsync windows it covers.
  // CPU time (`tdur`, thread time) when the trace has it: on a loaded box wall time counts preemption.
  const cpu = CPU && t.tdur !== undefined && t.dur > 0 ? Math.min(1, t.tdur / t.dur) : 1;
  let a = t.ts - t0;
  const b = a + t.dur;
  while (a < b) {
    const i = Math.floor(a / FRAME);
    const edge = Math.min(b, (i + 1) * FRAME);
    buckets[i] += (edge - a) * cpu;
    a = edge;
  }
}
const perFrame = Array.from(buckets, (us) => us / 1000);
// The longest tasks and what ran inside them (direct children by duration).
const longest = [...mt].sort((a, b) => b.dur - a.dur).slice(0, 4).map((t) => {
  const kids = events.filter((e) => e.ph === 'X' && `${e.pid}:${e.tid}` === main && e !== t && e.ts >= t.ts && e.ts + e.dur <= t.ts + t.dur && e.dur > 1000);
  const fn = (e) => e.name + (e.args?.data?.functionName ? ` ${e.args.data.functionName}@${String(e.args.data.url ?? '').split('/').pop()?.split('?')[0]}:${e.args.data.lineNumber}` : '');
  return `${r1(t.dur / 1000)}ms: ` + kids.sort((a, b) => b.dur - a.dur).slice(0, 5).map((e) => `${fn(e)} ${r1(e.dur / 1000)}`).join(' | ');
});
// What fills the slow frames (> 8 ms): top-level slices + the JS entry function, ms per slow frame.
const slow = new Set(perFrame.flatMap((ms, i) => (ms > 8 ? [i] : [])));
const slowBy = {};
for (const e of events) {
  if (e.ph !== 'X' || `${e.pid}:${e.tid}` !== main || !slow.has(Math.floor((e.ts - t0) / FRAME))) continue;
  if (!['FunctionCall', 'TimerFire', 'FireAnimationFrame', 'UpdateLayoutTree', 'Layout', 'Paint', 'PrePaint', 'Layerize', 'v8.callFunction'].includes(e.name)) continue;
  const fn = e.args?.data?.functionName ? ` ${e.args.data.functionName}@${String(e.args.data.url ?? '').split('/').pop()?.split('?')[0]}:${e.args.data.lineNumber}` : '';
  const k = e.name + fn;
  slowBy[k] = (slowBy[k] ?? 0) + e.dur / 1000;
}
// Where the time goes (top-level slices on the main thread).
const kinds = {};
for (const e of events) {
  if (e.ph !== 'X' || `${e.pid}:${e.tid}` !== main) continue;
  if (['FunctionCall', 'EvaluateScript', 'v8.callFunction', 'TimerFire', 'FireAnimationFrame', 'UpdateLayoutTree', 'Layout', 'Paint', 'PrePaint', 'Layerize', 'ParseHTML', 'MajorGC', 'MinorGC'].includes(e.name)) kinds[e.name] = (kinds[e.name] ?? 0) + e.dur / 1000;
}
const secs = (t1 - t0) / 1e6;
const out = {
  label: LABEL,
  width: WIDTH,
  chats: CHATS,
  secs: r1(secs),
  mainMsPerFrame: { p50: r1(pct(perFrame, 50)), p95: r1(pct(perFrame, 95)), p99: r1(pct(perFrame, 99)), max: r1(Math.max(...perFrame)), mean: r1(perFrame.reduce((a, b) => a + b, 0) / perFrame.length) },
  p95Per10s: Array.from({ length: Math.ceil(perFrame.length / 600) }, (_, i) => r1(pct(perFrame.slice(i * 600, (i + 1) * 600), 95))),
  mainBusyPct: r1((mt.reduce((a, t) => a + t.dur, 0) / (t1 - t0)) * 100),
  rafIntervalMs: { p50: r1(pct(page_.frames, 50)), p95: r1(pct(page_.frames, 95)), max: r1(Math.max(...page_.frames)), fps: r1(page_.frames.length / SECS) },
  longTasks: { count: page_.longTasks.length, totalMs: r1(page_.longTasks.reduce((a, b) => a + b, 0)), max: r1(Math.max(0, ...page_.longTasks)) },
  domNodes: page_.dom,
  heapMB: r1(page_.heap / 1048576),
  msPerSecBy: Object.fromEntries(Object.entries(kinds).sort((a, b) => b[1] - a[1]).map(([k, v]) => [k, r1(v / secs)])),
  slowFrames: slow.size,
  longest,
  slowBy: Object.fromEntries(Object.entries(slowBy).sort((a, b) => b[1] - a[1]).slice(0, 14).map(([k, v]) => [k, r1(v / Math.max(1, slow.size))])),
  tailChars: page_.tailText,
  errors: errors.slice(0, 5),
  ...(hot ? { hot, incl: globalThis.__incl } : {}),
};
console.log(JSON.stringify(out, null, 2));
await browser.close();
