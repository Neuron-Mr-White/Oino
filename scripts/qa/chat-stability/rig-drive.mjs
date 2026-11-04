// [owner-fix-2] The chat stability steps on the REAL app (scripts/qa/rig.sh: the .deb file
// tree on WebKitGTK, driven over WebDriver), with the same in-page sampler as drive.mjs.
// Opens one chat COPY from the sidebar, then: wheel up through all history, a scrollbar thumb
// drag, fold toggles, flicks (fast wheel spins both ways), a 3 s hold, then the jump button.
// Wheel and drag are REAL X input (xin.py, XTEST on the rig's Xvfb -> GTK -> WebKitGTK):
// WebKitWebDriver wheel actions were dropped/sign-flipped in the rig. Xvfb has no touchpad,
// so a true kinetic fling (GTK momentum scrolling) cannot be produced here. Writes <out>/<label>.json and a screenshot sequence <out>/<label>-NN-<step>.png.
//
//   QA_HOME=… QA_PORT=… node rig-drive.mjs <sessionId> <label> <outDir>
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { SAMPLER } from './sampler.mjs';

const [sid, label, out] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const QA = process.env.QA_HOME;
const BASE = `http://127.0.0.1:${process.env.QA_PORT ?? 4444}`;
const S = () => `/session/${readFileSync(`${QA}/run/session`, 'utf8').trim()}`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function wd(method, path, body) {
  const r = await fetch(BASE + S() + path, { method, headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${JSON.stringify(j).slice(0, 400)}`);
  return j.value;
}
const ev = (expr) => wd('POST', '/execute/sync', { script: `return (${expr})`, args: [] });
const eva = (body) => wd('POST', '/execute/async', { script: `const done = arguments[arguments.length - 1]; (async () => { ${body} })().then(done, (e) => done({ error: String(e) }))`, args: [] });
let shot = 0;
const snap = async (name) => writeFileSync(join(out, `${label}-${String(++shot).padStart(2, '0')}-${name}.png`), Buffer.from(await wd('GET', '/screenshot'), 'base64'));
// The sampler skips anchor checks for 120 ms after reader input. X input is sent after the
// call starts and WebKitGTK animates a wheel scroll (~150-250 ms), so after an X gesture the
// mark is moved 300 ms into the future: only movement after the browser settled counts.
const input = (aheadMs = 0) => ev(`(window.__RL_INPUT__ = Math.max(window.__RL_INPUT__ || 0, performance.now() + ${aheadMs}), 1)`);
const tlRect = () => ev(`(() => { const r = document.querySelector('[data-testid=tl]').getBoundingClientRect(); return { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) } })()`);
let R;
let W;
const XIN = join(dirname(fileURLToPath(import.meta.url)), 'xin.py');
const xin = (...args) => execFileSync('python3', [XIN, ...args.map(String)], { env: { ...process.env, DISPLAY: process.env.QA_DISPLAY ?? ':147' }, stdio: ['ignore', 'ignore', 'ignore'] });
/** Real wheel clicks (X buttons 4/5; ~50 px each in WebKitGTK) over the middle of the chat; dy in px. */
async function wheel(dy, delayMs = 16) {
  await input();
  xin('move', W.x + R.x + Math.round(R.w / 2), W.y + R.y + Math.round(R.h / 2), ';', 'wheel', dy < 0 ? 'up' : 'down', Math.max(1, Math.round(Math.abs(dy) / 50)), delayMs);
  await input(300);
}
const state = () => ev(`(() => { const tl = document.querySelector('[data-testid=tl]'); return tl ? { top: Math.round(tl.scrollTop), h: tl.scrollHeight, ch: tl.clientHeight, rows: tl.querySelectorAll('.tl-vrow').length, earlier: !!document.querySelector('[data-testid=tl-earlier]'), jump: !!document.querySelector('[data-testid=tl-jump]'), hash: location.hash } : null })()`);
const sampler = () => ev('JSON.parse(JSON.stringify(window.__RL__))');

const result = { label, sid, steps: {} };
try {
  await ev(SAMPLER);
  // Reader input as the PAGE sees it: WebKitGTK can deliver X wheel clicks late when the web
  // process is busy, then animates each (~200 ms), so every wheel event marks input 250 ms ahead.
  await ev(`(() => { if (!window.__RL_WHEEL__) { window.__RL_WHEEL__ = 1; addEventListener('wheel', () => { window.__RL_INPUT__ = Math.max(window.__RL_INPUT__ || 0, performance.now() + 250); }, { capture: true, passive: true }); } return 1 })()`);
  // one sampler per page: a second chat in the same window starts from zero
  await ev(`(() => { const R = window.__RL__; for (const k of Object.keys(R)) if (typeof R[k] === 'number') R[k] = 0; R.hash = [location.hash]; R.lastHash = location.hash; R.worst = ''; return 1 })()`);
  // ── open: click the chat in the sidebar ──
  const opened = await ev(`(() => { const a = document.querySelector('a[href*="${sid}"]'); if (!a) return false; window.__RL__.opened = 1; a.click(); return true })()`);
  if (!opened) throw new Error('no sidebar row for ' + sid);
  const t0 = Date.now();
  await sleep(250);
  await snap('open-250ms');
  for (let i = 0; i < 60 && !(await ev(`!!document.querySelector('[data-testid=tl] .tl-vrow')`)); i++) await sleep(250);
  await sleep(2500);
  await snap('open-3s');
  R = await tlRect();
  W = await wd('GET', '/window/rect'); // no window manager: viewport origin = window origin
  result.steps.open = { ms: Date.now() - t0, ...(await state()), sampler: await sampler() };

  // ── wheel up through all history ──
  let n = 0;
  let stillTop = 0;
  for (; n < 1500; n++) {
    await wheel(-400);
    await sleep(40);
    const s = await state();
    if (s.top === 0) {
      await sleep(500); // the reader pauses at the top: the scroll goes idle → one chunk
      const after = await state();
      if (after.top === 0 && !after.earlier) {
        if (++stillTop > 2) break;
      } else stillTop = 0;
    }
    if (n % 150 === 75) await snap(`wheel-${n}`);
  }
  await snap('wheel-top');
  result.steps.wheelUp = { wheels: n, ...(await state()), sampler: await sampler() };

  // ── a real scrollbar drag: press the thumb, move it to the middle, release ──
  const geo = await ev(`(() => { const tl = document.querySelector('[data-testid=tl]'); const r = tl.getBoundingClientRect(); const sw = tl.offsetWidth - tl.clientWidth; const th = Math.max(20, tl.clientHeight * tl.clientHeight / tl.scrollHeight); const top = r.top + (tl.clientHeight - th) * tl.scrollTop / Math.max(1, tl.scrollHeight - tl.clientHeight); return { sw, x: Math.round(r.right - Math.max(2, sw / 2)), y: Math.round(top + th / 2), mid: Math.round(r.top + tl.clientHeight / 2) } })()`);
  await input(20 * 30 + 400); // the whole drag is reader input (the page sees no event for a thumb drag)
  xin('drag', W.x + geo.x, W.y + geo.y, W.x + geo.x, W.y + geo.mid, 20);
  await input(300);
  await sleep(800);
  await snap('drag-middle');
  result.steps.drag = { geo, ...(await state()), sampler: await sampler() };

  // ── folds (mid-history, detached): open and close visible folds; the header must hold still ──
  const heads = `[...document.querySelectorAll('[data-testid=tl] .tl-vrow .proc-head, [data-testid=tl] .tl-vrow [data-testid=tl-agent-toggle]')].filter((h) => { const v = document.querySelector('[data-testid=tl]').getBoundingClientRect(); const r = h.getBoundingClientRect(); return r.top > v.top + 40 && r.bottom < v.bottom - 120; })`;
  for (let i = 0; i < 80 && !(await ev(`${heads}.length`)); i++) { await wheel(-300); await sleep(60); }
  await sleep(500);
  const folds = [];
  for (let i = 0; i < 6; i++) {
    const r = await eva(`const hs = ${heads}; const h = hs[${i} % Math.max(1, hs.length)]; if (!h) return null;
      const before = h.getBoundingClientRect().top; window.__RL_INPUT__ = performance.now();
      h.click(); await new Promise((r) => setTimeout(r, 600)); const open = h.getBoundingClientRect().top;
      const body = h.closest('.tl-vrow').getBoundingClientRect().height;
      h.click(); await new Promise((r) => setTimeout(r, 600));
      return { text: (h.innerText || '').slice(0, 60), before: Math.round(before), afterOpen: Math.round(open), rowH: Math.round(body), afterClose: Math.round(h.getBoundingClientRect().top) };`);
    if (r) folds.push(r);
    if (i === 0) await snap('fold');
  }
  result.steps.folds = { folds, maxMove: Math.max(0, ...folds.map((f) => Math.max(Math.abs(f.afterOpen - f.before), Math.abs(f.afterClose - f.before)))) };

  // ── flicks: fast wheel bursts, then a decaying tail (what a kinetic fling sends) ──
  for (const dir of [-1, 1]) {
    await wheel(dir * 6000, 4); // 120 clicks in ~0.5 s: a hard spin
    await sleep(600);
  }
  await snap('flick');
  result.steps.flick = { ...(await state()), sampler: await sampler() };

  // ── detached: the view must stay put for 3 s (nothing re-attaches it) ──
  await wheel(-600);
  await sleep(400);
  const held0 = await state();
  await sleep(3000);
  const held1 = await state();
  result.steps.hold = { before: held0.top, after: held1.top, jump: held1.jump };

  // ── back to the bottom by the jump button ──
  const jb = await ev(`(() => { const b = document.querySelector('[data-testid=tl-jump]'); if (b) b.click(); return !!b })()`);
  await sleep(1200);
  await snap('jump-bottom');
  const end = await state();
  result.steps.bottom = { jumpButton: jb, ...end, atBottom: end.h - end.ch - end.top <= 1 };
  result.sampler = await sampler();
} catch (e) {
  result.error = String(e?.stack ?? e);
}
writeFileSync(join(out, `${label}.json`), JSON.stringify(result, null, 1));
const s = result.sampler ?? {};
console.log(JSON.stringify({ label, error: result.error, openMs: result.steps.open?.ms, frames: s.frames, overlaps: s.overlaps, gaps: s.gaps, jumps: s.jumps, offBottom: s.offBottom ?? 0, maxJump: Math.round(s.maxJump ?? 0), worst: s.worst, blank: s.blank, skeleton: s.skeleton ?? 0, hash: s.hash, maxFrame: Math.round(s.maxFrame ?? 0), rowsAdded: s.rowsAdded, wheels: result.steps.wheelUp?.wheels, top: result.steps.wheelUp?.top, earlier: result.steps.wheelUp?.earlier, drag: result.steps.drag && { from: 0, to: result.steps.drag.top, h: result.steps.drag.h, sw: result.steps.drag.geo?.sw }, foldMove: result.steps.folds?.maxMove, hold: result.steps.hold, bottom: result.steps.bottom }));
