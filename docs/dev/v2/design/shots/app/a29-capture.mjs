/**
 * WP-A29 phone-shell captures: every scenario of A3–A27 at the two §3.9
 * phone sizes — 412×915 and 360×780 — plus scripted shots of the ⋯ "Panes"
 * group and its layers. Drives the owner's CloakBrowser over raw CDP
 * against the capture harness on port 1450.
 *
 *   node docs/dev/v2/design/shots/app/a29-capture.mjs [--only a,b,c] [--size 412]
 *
 * Routes: scenarios that self-route (files, settings, cron, modals, notify)
 * keep their hash; everything else lands on the world's first session card.
 * Per shot the script records a diagnostic row (horizontal overflow, root
 * testid) into a29-report.json next to the PNGs.
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const CDP = `http://127.0.0.1:${process.env.AGENT_BROWSER_CDP ?? '45859'}`;
const APP = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;
const SRC = new URL('../../../../../../apps/unipi/src/dev/mock/scenarios/index.ts', import.meta.url).pathname;

/** Scenario keys from the SCENARIOS map (quoted entries + shorthand names). */
function scenarioNames() {
  const src = readFileSync(SRC, 'utf8');
  const body = src.slice(src.indexOf('export const SCENARIOS'));
  const names = new Set();
  for (const m of body.matchAll(/'([a-z0-9-]+)':/g)) names.add(m[1]);
  for (const m of body.matchAll(/^\s{2}([a-zA-Z0-9_]+)\s*[:,]/gm)) names.add(m[1].replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`));
  return [...names];
}

/** Routes not derivable from the world's first session card. */
const HASH = {
  'board-lanes': '#/board',
  'board-dashboard': '#/board',
  'cron-list': '#/cron',
  'cron-form': '#/cron',
  'cron-badge': '#/cron',
  'needs-you': '#/chat',
  toasts: '#/chat',
  unread: '#/chat',
  empty: '#/chat',
  'no-host': '#/chat',
  offline: '#/chat',
  'empty-chat': '#/new/pc-coffee',
};
/** Scenarios that must stay on their own route (no session fallback). */
const NO_SESSION_FALLBACK = new Set([...Object.keys(HASH), 'settings-nav', 'settings-search', 'settings-extensions', 'settings-safe-mode', 'files-tree', 'files-search', 'files-preview', 'editor-lsp', 'editor-conflict', 'composer-states', 'composer-paste', 'composer-slash', 'composer-attachments', 'composer-narrow', 'image-editor-ellipse', 'motion-exits', 'shortcuts-list', 'cmdk-empty', 'cmdk-files', 'find-bar']);

const only = process.argv.includes('--only') ? process.argv[process.argv.indexOf('--only') + 1].split(',') : undefined;
const onlySize = process.argv.includes('--size') ? Number(process.argv[process.argv.indexOf('--size') + 1]) : undefined;
/** --diag: skip the PNG, just refresh the route/overflow rows. --no-scripted: skip the ⋯ shots. */
const diagOnly = process.argv.includes('--diag');
const noScripted = process.argv.includes('--no-scripted');

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
    const { resolve: res, reject: rej } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? rej(new Error(msg.error.message)) : res(msg.result);
  }
};
function send(method, params = {}, sessionId, timeoutMs = 25000) {
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
async function evalIn(sessionId, expression) {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, sessionId);
  return r.result?.value;
}
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
}

/** Wait until the app has painted over the splash, then settle. */
async function ready(sessionId, settle = 2600) {
  const probe = `(() => { const s = document.getElementById('splash'); return (!s || s.classList.contains('splash-out')) && !!document.querySelector('[data-testid], main'); })()`;
  for (let i = 0; i < 48; i++) {
    if ((await evalIn(sessionId, probe)) === true) break;
    await sleep(250);
  }
  await sleep(settle);
}

async function page(url) {
  const { targetId } = await send('Target.createTarget', { url, background: true });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  await send('Page.enable', {}, sessionId);
  return { targetId, sessionId };
}

mkdirSync(OUT, { recursive: true });
for (const target of (await send('Target.getTargets')).targetInfos ?? []) {
  if (target.type === 'page' && target.url.includes('127.0.0.1:1450')) await send('Target.closeTarget', { targetId: target.targetId }).catch(() => {});
}

/** Merge into the existing report: reruns (a skipped scenario) must not drop rows. */
const report = [];
const reportKey = (row) => `${row.scenario}|${row.size ?? ''}`;
try {
  const prev = JSON.parse(readFileSync(`${OUT}/a29-report.json`, 'utf8'));
  if (Array.isArray(prev)) report.push(...prev);
} catch {}
const push = (row) => {
  const key = reportKey(row);
  const at = report.findIndex((r) => reportKey(r) === key);
  if (at >= 0) report[at] = row;
  else report.push(row);
};
const names = scenarioNames().filter((n) => !only || only.includes(n));
const sizes = onlySize ? [onlySize] : [412, 360];

for (const name of names) {
  for (const width of sizes) {
    const height = width === 360 ? 780 : 915;
    const hash = HASH[name] ?? '#/';
    try {
      const { targetId, sessionId } = await page(`${APP}/?scenario=${name}&shell=phone${hash}`);
      await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 2, mobile: true }, sessionId);
      await ready(sessionId, 1500);
      // Session fallback: still on / or /chat and the world has cards → open the first one.
      if (!NO_SESSION_FALLBACK.has(name)) {
        const moved = await evalIn(
          sessionId,
          `(() => {
            const h = location.hash;
            if (h.startsWith('#/s/') || h.startsWith('#/files') || h.startsWith('#/settings') || h.startsWith('#/board') || h.startsWith('#/cron') || h.startsWith('#/x/')) return 'kept:' + h;
            const w = window.__UNIPI_MOCK_WORLD__;
            const host = w?.scenario?.hosts?.find((x) => (x.cards?.length ?? 0) > 0);
            const card = host?.cards?.[0];
            if (!card) return 'none:' + h;
            location.hash = '#/s/' + host.id + '/' + card.session_id;
            return 'moved:' + location.hash;
          })()`,
        );
        if (String(moved).startsWith('moved:')) await sleep(2200);
        push({ scenario: name, size: width, route: moved });
      } else push({ scenario: name, size: width, route: await evalIn(sessionId, 'location.hash') });
      const diag = await evalIn(sessionId, `({ sw: document.documentElement.scrollWidth, iw: window.innerWidth, bh: document.body ? document.body.scrollHeight : 0, testid: document.querySelector('[data-testid]')?.getAttribute('data-testid') ?? '' })`);
      const file = `a29-${name}-phone-${width}.png`;
      if (!diagOnly) writeFileSync(`${OUT}/${file}`, Buffer.from(await shot(sessionId), 'base64'));
      push({ scenario: name, size: width, route: report.find((r) => r.scenario === name && r.size === width)?.route, overflow: diag && diag.sw > diag.iw + 1, diag, file });
      console.log(file, diag?.sw > diag?.iw + 1 ? '  ← H-OVERFLOW' : '');
      await send('Target.closeTarget', { targetId });
      await sleep(250);
    } catch (error) {
      push({ scenario: name, size: width, error: String(error?.message ?? error) });
      console.warn(`skip ${name}-${width}: ${error.message}`);
    }
  }
}

// ── scripted ⋯ "Panes" shots (412) ─────────────────────────────────────
const SESSION_URL = `${APP}/?scenario=stream-basic&shell=phone#/s/pc-coffee/s001`;
const SCRIPTED = [
  ['chat', `${APP}/?scenario=stream-basic&shell=phone#/chat`, null],
  ['panes-menu', SESSION_URL, `document.querySelector('[data-testid=conv-more]')?.click()`],
  [
    'pane-git',
    SESSION_URL,
    `document.querySelector('[data-testid=conv-more]')?.click(); setTimeout(() => { [...document.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.trim() === 'Git')?.click(); }, 300)`,
  ],
  [
    'pane-files',
    `${APP}/?scenario=stream-basic&shell=phone#/new/pc-coffee`,
    `document.querySelector('[data-testid=conv-more]')?.click(); setTimeout(() => { [...document.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.trim() === 'Files')?.click(); }, 300)`,
  ],
  [
    'pane-tree',
    SESSION_URL,
    `document.querySelector('[data-testid=conv-more]')?.click(); setTimeout(() => { [...document.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.trim() === 'Tree')?.click(); }, 300)`,
  ],
  [
    'pane-btw',
    SESSION_URL,
    `document.querySelector('[data-testid=conv-more]')?.click(); setTimeout(() => { [...document.querySelectorAll('[role=menuitem]')].find((el) => el.textContent?.trim() === 'Side chat')?.click(); }, 300)`,
  ],
];
for (const [name, url, action] of noScripted ? [] : SCRIPTED) {
  try {
    const { targetId, sessionId } = await page(url);
    await send('Emulation.setDeviceMetricsOverride', { width: 412, height: 915, deviceScaleFactor: 2, mobile: true }, sessionId);
    await ready(sessionId);
    if (action) {
      await evalIn(sessionId, `(() => { ${action}; return true; })()`);
      await sleep(1600);
    }
    const file = `a29-${name}-phone-412.png`;
    writeFileSync(`${OUT}/${file}`, Buffer.from(await shot(sessionId), 'base64'));
    push({ scenario: `shot:${name}`, route: await evalIn(sessionId, 'location.hash') });
    console.log(file);
    await send('Target.closeTarget', { targetId });
    await sleep(250);
  } catch (error) {
    console.warn(`skip ${name}: ${error.message}`);
  }
}
writeFileSync(`${OUT}/a29-report.json`, JSON.stringify(report, null, 2));
browser.close();
console.log(`done — ${report.length} rows in a29-report.json`);
