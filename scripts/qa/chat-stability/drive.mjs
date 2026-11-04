// [chat-load] Chat stability rig driver: headless Chromium over CDP against the vite app
// (run.sh starts it). Opens one chat COPY, then: wheel up through all history, scrollbar
// drag, flicks, fold toggles. Every animation frame the page samples row rects (overlap /
// gap), the first visible turn's on-screen top (anchor jumps in frames without input),
// blank frames, location.hash changes. Writes <out>/<label>.json + a screenshot sequence
// (+ a screencast → <label>.mp4 when ffmpeg is on PATH).
//
//   node drive.mjs <vitePort> <chatFile> <label> <outDir>
import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const [vport, chat, label, out] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const frames = join(out, `${label}-frames`);
mkdirSync(frames, { recursive: true });
const dbg = 9400 + (Number(vport) % 100);
const pw = `${process.env.HOME}/.cache/ms-playwright`;
const shell = readdirSync(pw).filter((d) => d.startsWith('chromium_headless_shell')).sort().at(-1);
const bin = process.env.CHROME_BIN ?? `${pw}/${shell}/chrome-headless-shell-linux64/chrome-headless-shell`;
const chrome = spawn(bin, [`--remote-debugging-port=${dbg}`, '--window-size=1280,820', '--no-sandbox', `--user-data-dir=${out}/.chrome-${vport}`, 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let ws;
let id = 0;
const pending = new Map();
let shot = 0;
let cast = 0;
const castOn = process.env.RL_CAST !== '0';

async function connect() {
  for (let i = 0; i < 60; i++) {
    try {
      const l = await (await fetch(`http://127.0.0.1:${dbg}/json/list`)).json();
      const p = l.find((t) => t.type === 'page');
      if (p) return p.webSocketDebuggerUrl;
    } catch {}
    await sleep(200);
  }
  throw new Error('no chrome');
}
const send = (method, params = {}) =>
  new Promise((res, rej) => {
    const i = ++id;
    pending.set(i, { res, rej });
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const ev = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 600));
  return r.result.value;
};
const snap = async (name) => {
  const r = await send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(join(out, `${label}-${String(++shot).padStart(2, '0')}-${name}.png`), Buffer.from(r.data, 'base64'));
};
/** Marks reader input for the in-page sampler (anchor checks skip those frames). */
const input = () => ev('window.__RL_INPUT__ = performance.now(), 1');
const wheel = async (dy, x = 640, y = 400) => {
  await input();
  await send('Input.dispatchMouseEvent', { type: 'mouseWheel', x, y, deltaX: 0, deltaY: dy });
};

// In-page sampler: one rAF loop, bounded arrays.
const SAMPLER = `(() => {
  if (window.__RL__) return 1;
  const R = window.__RL__ = { frames: 0, overlaps: 0, gaps: 0, blank: 0, jumps: 0, maxJump: 0, worst: '', hash: [location.hash], long: 0, longMs: 0, maxFrame: 0, rowsAdded: 0, lastHash: location.hash };
  window.__RL_INPUT__ = 0;
  let prev; let last = performance.now();
  new MutationObserver((ms) => { for (const m of ms) for (const n of m.addedNodes) if (n.nodeType === 1 && n.classList.contains('tl-vrow')) R.rowsAdded++; }).observe(document.body, { childList: true, subtree: true });
  try { new PerformanceObserver((l) => { for (const e of l.getEntries()) { R.long++; R.longMs += e.duration; } }).observe({ type: 'longtask', buffered: true }); } catch {}
  const tick = (t) => {
    R.frames++; R.maxFrame = Math.max(R.maxFrame, t - last); last = t;
    if (location.hash !== R.lastHash) { R.lastHash = location.hash; if (R.hash.length < 20) R.hash.push(location.hash); }
    const tl = document.querySelector('[data-testid=tl]');
    if (tl) {
      const view = tl.getBoundingClientRect();
      const rows = [...tl.querySelectorAll('.tl-vrow')].map((el) => ({ id: el.dataset.turn, r: el.getBoundingClientRect() }))
        .filter((x) => x.r.height > 0 && x.r.bottom > view.top - 2000 && x.r.top < view.bottom + 2000).sort((a, b) => a.r.top - b.r.top);
      for (let i = 1; i < rows.length; i++) {
        if (rows[i].r.top < rows[i - 1].r.bottom - 2) R.overlaps++;
        else if (rows[i].r.top > rows[i - 1].r.bottom + 2 && rows[i].r.bottom > view.top && rows[i - 1].r.top < view.bottom) R.gaps++;
      }
      const vis = rows.filter((x) => x.r.bottom > view.top + 1 && x.r.top < view.bottom);
      // A blank frame: the chat is open but nothing of it is on screen (no row, no tail, no skeleton).
      if (R.opened && !vis.length) {
        const tail = tl.querySelector('.tl-tail-wrap'); const tr = tail && tail.getBoundingClientRect();
        if (tl.querySelector('[data-testid=tl-skeleton]')) R.skeleton = (R.skeleton || 0) + 1;
        else if (!(tr && tr.height > 0 && tr.bottom > view.top && tr.top < view.bottom)) R.blank++;
      }
      const first = vis[0];
      const quiet = performance.now() - window.__RL_INPUT__ > 120;
      const lockedNow = !document.querySelector('[data-testid=tl-jump]');
      if (prev && first && quiet && !lockedNow) {
        const same = rows.find((x) => x.id === prev.id);
        if (same) { const j = Math.abs(same.r.top - prev.top); if (j > 2) { R.jumps++; if (j > R.maxJump) { R.maxJump = j; R.worst = prev.id + ' ' + Math.round(prev.top) + '→' + Math.round(same.r.top) + ' @' + Math.round(tl.scrollTop); R.trace = window.__TL_TRACE__ ? window.__TL_TRACE__() : undefined; } } }
      }
      // Bottom-locked (no jump button): the bottom is the anchor and must hold exactly.
      if (quiet && !document.querySelector('[data-testid=tl-jump]') && tl.scrollHeight - tl.clientHeight - tl.scrollTop > 1) R.offBottom = (R.offBottom || 0) + 1;
      prev = first ? { id: first.id, top: first.r.top } : undefined;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  return 1;
})()`;

const state = () => ev(`(() => { const tl = document.querySelector('[data-testid=tl]'); return tl ? { top: Math.round(tl.scrollTop), h: tl.scrollHeight, ch: tl.clientHeight, rows: tl.querySelectorAll('.tl-vrow').length, earlier: !!document.querySelector('[data-testid=tl-earlier]'), jump: !!document.querySelector('[data-testid=tl-jump]'), pages: window.__RL_PAGES__ } : null })()`);

const result = { label, chat, steps: {} };
try {
  ws = new WebSocket(await connect());
  await new Promise((r) => (ws.onopen = r));
  ws.onmessage = (m) => {
    const d = JSON.parse(m.data);
    if (d.id && pending.has(d.id)) {
      const p = pending.get(d.id);
      pending.delete(d.id);
      d.error ? p.rej(new Error(JSON.stringify(d.error))) : p.res(d.result);
    } else if (d.method === 'Page.screencastFrame') {
      writeFileSync(join(frames, `${String(++cast).padStart(5, '0')}.jpg`), Buffer.from(d.params.data, 'base64'));
      send('Page.screencastFrameAck', { sessionId: d.params.sessionId }).catch(() => {});
    }
  };
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 820, deviceScaleFactor: 1, mobile: false });
  await send('Page.navigate', { url: `http://127.0.0.1:${vport}/#/chat?scenario=owner-chat&shell=desktop&chat=${encodeURIComponent(chat)}` });
  for (let i = 0; i < 360; i++) {
    const ok = await ev(`!!window.__RL_CHAT__ && !!document.querySelector('a[href*="' + window.__RL_CHAT__.sid + '"]')`).catch(() => false);
    if (ok) break;
    await sleep(500);
  }
  await sleep(1500);
  result.chatInfo = await ev('window.__RL_CHAT__');
  await ev(SAMPLER);
  if (castOn) await send('Page.startScreencast', { format: 'jpeg', quality: 60, maxWidth: 1280, maxHeight: 820, everyNthFrame: 2 });
  // ── open: click the chat in the sidebar ──
  const sid = result.chatInfo.sid;
  await ev(`(() => { const a = document.querySelector('a[href*="${sid}"]'); if (!a) throw new Error('no sidebar row'); a.click(); window.__RL__.opened = 1; return 1 })()`);
  const t0 = Date.now();
  await sleep(300);
  await snap('open-300ms');
  await sleep(2700);
  await snap('open-3s');
  result.steps.open = { ms: Date.now() - t0, ...(await state()), sampler: await ev('JSON.parse(JSON.stringify(window.__RL__))') };

  // ── wheel up through all history ──
  let n = 0;
  let stillTop = 0;
  for (; n < 600; n++) {
    await wheel(-420);
    await sleep(60);
    const s = await state();
    if (s.top === 0) {
      await sleep(400); // the reader pauses at the top: the scroll goes idle → one chunk
      const after = await state();
      if (after.top === 0 && !after.earlier) {
        if (++stillTop > 2) break;
      } else stillTop = 0;
    }
    if (n % 60 === 30) await snap(`wheel-${n}`);
  }
  await snap('wheel-top');
  result.steps.wheelUp = { wheels: n, ...(await state()), sampler: await ev('JSON.parse(JSON.stringify(window.__RL__))') };

  // ── scrollbar drag: headless Chromium draws no scrollbar, so this is what a thumb drag
  // produces: a press on the scroller itself, then scroll events with no wheel, to the middle.
  for (let i = 1; i <= 30; i++) {
    await input();
    await ev(`(() => { const tl = document.querySelector('[data-testid=tl]'); if (${i} === 1) tl.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); tl.scrollTop = (tl.scrollHeight - tl.clientHeight) * ${i} / 60; return 1 })()`);
    await sleep(50);
  }
  await sleep(600);
  await snap('drag-middle');
  result.steps.drag = { ...(await state()), sampler: await ev('JSON.parse(JSON.stringify(window.__RL__))') };

  // ── folds (mid-history, detached): open and close visible folds; the header must hold still ──
  const folds = [];
  // bring a fold header into view (wheel up from the middle until one shows)
  const visibleHeads = `(() => { const tl = document.querySelector('[data-testid=tl]'); const v = tl.getBoundingClientRect(); return [...tl.querySelectorAll('.tl-vrow .proc-head, .tl-vrow [data-testid=tl-agent-toggle]')].filter((h) => { const r = h.getBoundingClientRect(); return r.top > v.top + 40 && r.bottom < v.bottom - 120; }).length })()`;
  for (let i = 0; i < 60 && !(await ev(visibleHeads)); i++) {
    await wheel(-300);
    await sleep(90);
  }
  await sleep(500);
  for (let i = 0; i < 6; i++) {
    const r = await ev(`(async () => {
      const tl = document.querySelector('[data-testid=tl]'); const v = tl.getBoundingClientRect();
      const hs = [...tl.querySelectorAll('.tl-vrow .proc-head, .tl-vrow [data-testid=tl-agent-toggle]')].filter((h) => { const r = h.getBoundingClientRect(); return r.top > v.top + 40 && r.bottom < v.bottom - 120; });
      const h = hs[${i} % Math.max(1, hs.length)]; if (!h) return null;
      const before = h.getBoundingClientRect().top; window.__RL_INPUT__ = performance.now();
      h.click(); await new Promise((r) => setTimeout(r, 500));
      const open = h.getBoundingClientRect().top;
      h.click(); await new Promise((r) => setTimeout(r, 500));
      return { before: Math.round(before), afterOpen: Math.round(open), afterClose: Math.round(h.getBoundingClientRect().top) };
    })()`);
    if (r) folds.push(r);
    if (i === 0) await snap('fold');
  }
  result.steps.folds = { folds, maxMove: Math.max(0, ...folds.map((f) => Math.max(Math.abs(f.afterOpen - f.before), Math.abs(f.afterClose - f.before)))) };

  // ── flicks: fast wheel bursts both ways ──
  const flickPages = (await state()).pages;
  for (let i = 0; i < 25; i++) {
    await wheel(-1600);
    await sleep(16);
  }
  const midFlick = (await state()).pages;
  await sleep(500);
  for (let i = 0; i < 25; i++) {
    await wheel(1600);
    await sleep(16);
  }
  await sleep(700);
  await snap('flick');
  result.steps.flick = { pagesDuringFlick: midFlick - flickPages, ...(await state()), sampler: await ev('JSON.parse(JSON.stringify(window.__RL__))') };

  // ── back to the bottom by the jump button ──
  await ev(`(() => { const b = document.querySelector('[data-testid=tl-jump]'); if (b) b.click(); return !!b })()`);
  await sleep(800);
  await snap('jump-bottom');
  result.steps.bottom = { ...(await state()) };
  result.sampler = await ev('JSON.parse(JSON.stringify(window.__RL__))');
  if (castOn) await send('Page.stopScreencast').catch(() => {});
} catch (e) {
  result.error = String(e?.stack ?? e);
} finally {
  chrome.kill('SIGKILL');
}
writeFileSync(join(out, `${label}.json`), JSON.stringify(result, null, 1));
if (castOn && cast > 10 && spawnSync('ffmpeg', ['-version']).status === 0) {
  spawnSync('ffmpeg', ['-y', '-loglevel', 'error', '-framerate', '30', '-i', join(frames, '%05d.jpg'), '-vf', 'scale=960:-2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-crf', '30', join(out, `${label}.mp4`)]);
}
if (existsSync(frames)) spawnSync('rm', ['-rf', frames, join(out, `.chrome-${vport}`)]);
const s = result.sampler ?? {};
console.log(JSON.stringify({ label, error: result.error, frames: s.frames, overlaps: s.overlaps, gaps: s.gaps, jumps: s.jumps, offBottom: s.offBottom ?? 0, maxJump: Math.round(s.maxJump ?? 0), worst: s.worst, blank: s.blank, hash: s.hash, long: s.long, longMs: Math.round(s.longMs ?? 0), maxFrame: Math.round(s.maxFrame ?? 0), rowsAdded: s.rowsAdded, pages: result.steps.bottom?.pages, wheels: result.steps.wheelUp?.wheels, pagesDuringFlick: result.steps.flick?.pagesDuringFlick, foldMove: result.steps.folds?.maxMove }));
