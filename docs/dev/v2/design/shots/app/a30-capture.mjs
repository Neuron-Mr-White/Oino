/**
 * WP-A30 integration QA captures: the scripted flow list of app.md §5 WP-A30
 * (new chat → stream → tools → file-summary undo → queue/steer → dialog →
 * takeover → Board/Cron/Settings + the wave-3b surfaces), desktop 1280×820 and
 * phone 412×915, against the capture harness on 1450.
 *
 * Usage: node docs/dev/v2/design/shots/app/a30-capture.mjs [--only name,name]
 */
import { mkdirSync, writeFileSync } from 'node:fs';

const CDP = `http://127.0.0.1:${process.env.AGENT_BROWSER_CDP ?? '45859'}`;
const APP = process.env.APP_URL ?? 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;
const only = process.argv.includes('--only') ? process.argv[process.argv.indexOf('--only') + 1]?.split(',') : undefined;

/** [scenario, route] — most scenarios park their data on s001. */
const FLOWS = [
  ['empty-chat', '#/new/pc-coffee'],
  ['stream-basic', '#/s/pc-coffee/s001'],
  ['stream-steer', '#/s/pc-coffee/s001'],
  ['timeline-many', '#/s/pc-coffee/s001'],
  ['tools-mix', '#/s/pc-coffee/s001'],
  ['turn-files-undo', '#/s/pc-coffee/s-files'],
  ['queue-held', '#/s/pc-coffee/s001'],
  ['dialog-approval', '#/s/pc-coffee/s001'],
  ['taken-over', '#/s/pc-coffee/s001'],
  ['board-lanes', '#/board'],
  ['cron-list', '#/cron'],
  ['settings-nav', '#/settings'],
  ['sidebar-many', '#/'],
  ['modal-running', '#/s/pc-coffee/s001'],
  ['git-last-turn', '#/s/pc-coffee/s001'],
  ['status-lh-graph', '#/s/pc-coffee/s001'],
  ['u12-native', '#/s/pc-coffee/s001'],
  ['cap-memory', '#/settings'],
];

const version = await (await fetch(`${CDP}/json/version`)).json();
const browser = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { browser.onopen = resolve; browser.onerror = reject; });
let seq = 0; const pending = new Map();
browser.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id); pending.delete(msg.id);
    msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
  }
};
function send(method, params = {}, sessionId, timeoutMs = 25000) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, timeoutMs);
    pending.set(id, { resolve: (v) => { clearTimeout(timer); resolve(v); }, reject: (e) => { clearTimeout(timer); reject(e); } });
    browser.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The app is up: splash gone. */
async function up(sessionId) {
  for (let i = 0; i < 60; i++) {
    const r = await send('Runtime.evaluate', { expression: `(() => {
      const s = document.getElementById('splash');
      return (s === null || getComputedStyle(s).opacity === '0') && !!document.querySelector('.app, .shell, [class*=shell], main, #root > *');
    })()`, returnByValue: true }, sessionId);
    if (r.result.value === true) return true;
    await sleep(250);
  }
  return false;
}

async function shot(sessionId) {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      await send('Page.bringToFront', {}, sessionId);
      const { data } = await send('Page.captureScreenshot', { format: 'png' }, sessionId);
      return data;
    } catch (error) { if (attempt === 2) throw error; await sleep(900); }
  }
  return undefined;
}

mkdirSync(OUT, { recursive: true });
for (const target of (await send('Target.getTargets')).targetInfos ?? []) {
  if (target.type === 'page' && target.url.includes('127.0.0.1:1450')) await send('Target.closeTarget', { targetId: target.targetId }).catch(() => {});
}
// Warm the dev server.
{
  const { targetId } = await send('Target.createTarget', { url: `${APP}/?scenario=stream-basic&shell=desktop#/s/pc-coffee/s001`, background: true });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  await send('Page.enable', {}, sessionId);
  await up(sessionId); await sleep(2000);
  await send('Target.closeTarget', { targetId }); await sleep(500);
}

const results = [];
for (const [scenario, route] of FLOWS) {
  if (only && !only.includes(scenario)) continue;
  for (const [shell, width, height] of [['desktop', 1280, 820], ['phone', 412, 915]]) {
    const file = `a30-${scenario}-${shell}.png`;
    for (let attempt = 0; attempt < 2; attempt++) {
      try {
        const url = `${APP}/?scenario=${scenario}&shell=${shell}${route}`;
        const { targetId } = await send('Target.createTarget', { url, background: true });
        const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
        await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 2, mobile: shell === 'phone' }, sessionId);
        await send('Page.enable', {}, sessionId);
        await send('Page.bringToFront', {}, sessionId);
        const ready = await up(sessionId);
        await sleep(2600);
        const data = await shot(sessionId);
        writeFileSync(`${OUT}/${file}`, Buffer.from(data, 'base64'));
        const hash = await send('Runtime.evaluate', { expression: 'location.hash', returnByValue: true }, sessionId);
        console.log(`${file} ready=${ready} hash=${hash.result.value}`);
        results.push({ scenario, shell, ready, hash: hash.result.value });
        await send('Target.closeTarget', { targetId }); await sleep(500);
        break;
      } catch (error) {
        if (attempt === 1) { console.warn(`skip ${file}: ${error.message}`); results.push({ scenario, shell, error: error.message }); }
        else await sleep(1200);
      }
    }
  }
}
writeFileSync(`${OUT}/a30-report.json`, JSON.stringify(results, null, 2));
browser.close();
