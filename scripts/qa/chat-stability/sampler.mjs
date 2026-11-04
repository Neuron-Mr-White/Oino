// [chat-load] In-page frame sampler shared by drive.mjs (headless Chromium over CDP) and
// rig-drive.mjs (the real app: WebKitGTK over WebDriver). Evaluate the string in the page.
// In-page sampler: one rAF loop, bounded arrays.
export const SAMPLER = `(() => {
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
