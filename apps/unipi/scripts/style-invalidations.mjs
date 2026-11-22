#!/usr/bin/env node
/**
 * r5-ui D16: which style invalidations dominate `UpdateLayoutTree` while the perf-stream scenario runs.
 *
 *   node scripts/style-invalidations.mjs [--url http://127.0.0.1:1477] [--width 412] [--secs 8]
 *
 * Uses the invalidation-tracking trace category: every StyleInvalidator / StyleRecalcInvalidationTracking
 * event names the reason (attribute / class / pseudo / `:has()` / inherited), the changed node and how many
 * elements were then recalculated (`UpdateLayoutTree` `elementCount`).
 */
import { createRequire } from 'node:module';
import { existsSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
};
const URL_BASE = arg('url', 'http://127.0.0.1:1477');
const WIDTH = Number(arg('width', '412'));
const SECS = Number(arg('secs', '8'));
const PHONE = WIDTH < 900;
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_CORE ?? `${homedir()}/.bun/install/cache/playwright-core@1.64.1@@@1`);
const chromeDir = `${homedir()}/.agent-browser/browsers`;
const chrome = process.env.CHROME ?? (existsSync(chromeDir) ? `${chromeDir}/${readdirSync(chromeDir).sort().at(-1)}/chrome` : undefined);

const browser = await chromium.launch({ executablePath: chrome, headless: true });
const page = await (await browser.newContext({ viewport: { width: WIDTH, height: PHONE ? 915 : 900 }, ...(PHONE ? { isMobile: true, hasTouch: true } : {}) })).newPage();
await page.goto(`${URL_BASE}/#/s/pc-coffee/big?scenario=perf-stream&shell=${PHONE ? 'phone' : 'desktop'}&chats=10`);
await page.waitForSelector('[data-testid="tl-tail"]', { timeout: 30_000 });
await page.waitForTimeout(5000);
// Experiment: `--drop <substr>` deletes every CSS rule whose text contains it (e.g. `:has(`), to price it.
const drop = arg('drop');
const range = arg('range'); // "lo,hi": only the lo..hi-th matching rules (bisecting which one costs)
if (drop) {
  const n = await page.evaluate(([needle, range]) => {
    let removed = 0;
    let seen = 0;
    const [lo, hi] = range ? range.split(',').map(Number) : [0, Infinity];
    const walk = (rules) => {
      for (let i = rules.length - 1; i >= 0; i--) {
        const r = rules[i];
        if (r.cssRules && r.cssRules.length && !r.selectorText) walk(r.cssRules);
        else if (r.cssText.includes(needle) && (seen++ >= lo && seen - 1 < hi)) { r.parentRule ? r.parentRule.deleteRule(i) : r.parentStyleSheet.deleteRule(i); removed++; }
      }
    };
    for (const sheet of document.styleSheets) { try { walk(sheet.cssRules); } catch {} }
    return removed;
  }, [drop, range]);
  console.log(`dropped ${n} rules containing ${drop}`);
  await page.waitForTimeout(1500);
}
const cdp = await page.context().newCDPSession(page);
const events = [];
cdp.on('Tracing.dataCollected', (d) => events.push(...d.value));
const done = new Promise((res) => cdp.once('Tracing.tracingComplete', res));
await cdp.send('Tracing.start', {
  categories: 'devtools.timeline,disabled-by-default-devtools.timeline,disabled-by-default-devtools.timeline.invalidationTracking',
  transferMode: 'ReportEvents',
});
await page.waitForTimeout(SECS * 1000);
await cdp.send('Tracing.end');
await done;
await browser.close();

const ult = events.filter((e) => e.name === 'UpdateLayoutTree' && e.ph === 'X');
const total = ult.reduce((a, e) => a + e.dur, 0) / 1000;
const elems = ult.map((e) => e.args?.elementCount ?? 0);
console.log(`UpdateLayoutTree: ${ult.length} recalcs, ${(total / SECS).toFixed(1)} ms/s, elementCount sum ${elems.reduce((a, b) => a + b, 0)}, max ${Math.max(0, ...elems)}`);
console.log('top by dur:', ult.sort((a, b) => b.dur - a.dur).slice(0, 6).map((e) => `${(e.dur / 1000).toFixed(1)}ms/${e.args?.elementCount}el`).join(' '));
const inv = events.filter((e) => e.name === 'StyleRecalcInvalidationTracking');
const by = new Map();
for (const e of inv) {
  const d = e.args?.data ?? {};
  const k = `${d.reason} | ${d.extraData ?? ''} | ${(d.nodeName ?? '').slice(0, 60)}`;
  by.set(k, (by.get(k) ?? 0) + 1);
}
console.log(`StyleRecalcInvalidationTracking: ${inv.length}`);
for (const [k, v] of [...by.entries()].sort((a, b) => b[1] - a[1]).slice(0, Number(arg('top', '25')))) console.log(String(v).padStart(6), k);
const sel = events.filter((e) => e.name === 'ScheduleStyleInvalidationTracking');
const bs = new Map();
for (const e of sel) {
  const d = e.args?.data ?? {};
  const k = `${d.invalidationSet ?? ''} | ${d.changedClass ?? d.changedAttribute ?? d.changedId ?? ''} | ${(d.nodeName ?? '').slice(0, 50)}`;
  bs.set(k, (bs.get(k) ?? 0) + 1);
}
console.log(`ScheduleStyleInvalidationTracking: ${sel.length}`);
for (const [k, v] of [...bs.entries()].sort((a, b) => b[1] - a[1]).slice(0, Number(arg('top', '25')))) console.log(String(v).padStart(6), k);
