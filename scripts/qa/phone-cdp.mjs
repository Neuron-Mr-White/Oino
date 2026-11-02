// phone-cdp.mjs — tiny CDP driver for the Unipi Android WebView (forwarded to localhost:$CDP_PORT, default 9333
// by `phone-sandbox.sh cdp`). Commands:
//   eval <js>            print the JSON result      measure       targets < 44 px, font sizes, tiny text, h-overflow
//   where <q> [nth]      device px centre of an element (css, aria-label or text) for `adb input tap`
//   click <q>            DOM click (no real touch)  shot <file>   CDP screenshot of the WebView
const port = process.env.CDP_PORT || 9333;
const list = await (await fetch(`http://localhost:${port}/json`)).json();
const page = list.find((p) => p.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener('open', r));
let id = 0;
const pending = new Map();
ws.addEventListener('message', (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) return { error: r.result.exceptionDetails.exception?.description || r.result.exceptionDetails.text };
  return r.result?.result?.value;
};
const [cmd, arg] = process.argv.slice(2);
const MEASURE = `(() => {
  const vis = (el) => { const r = el.getBoundingClientRect(); const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none' && r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth; };
  const label = (el) => (el.getAttribute('aria-label') || el.innerText || el.title || el.placeholder || el.className || el.tagName).toString().trim().replace(/\\s+/g,' ').slice(0,40);
  const out = { viewport: [innerWidth, innerHeight], dpr: devicePixelRatio, hash: location.hash, small: [], fonts: {}, tinyText: [], overflowX: document.documentElement.scrollWidth > innerWidth ? document.documentElement.scrollWidth : 0, hoverOnly: 0, safeTop: getComputedStyle(document.documentElement).getPropertyValue('--sat') };
  const sel = 'button,a,[role=button],[role=tab],[role=menuitem],[role=switch],[role=checkbox],input,select,textarea,summary,[tabindex]:not([tabindex="-1"])';
  for (const el of document.querySelectorAll(sel)) { if (!vis(el)) continue; const r = el.getBoundingClientRect();
    if (r.width < 44 || r.height < 44) out.small.push(Math.round(r.width)+'x'+Math.round(r.height)+' @'+Math.round(r.left)+','+Math.round(r.top)+' '+label(el)); }
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) { const t = walker.currentNode; if (!t.textContent.trim()) continue; const el = t.parentElement; if (!vis(el)) continue;
    const fs = parseFloat(getComputedStyle(el).fontSize); out.fonts[fs] = (out.fonts[fs]||0)+1;
    if (fs < 12) out.tinyText.push(fs+'px '+t.textContent.trim().slice(0,30)); }
  out.tinyText = [...new Set(out.tinyText)].slice(0,25);
  return out; })()`;
if (cmd === 'eval') console.log(JSON.stringify(await ev(arg), null, 1));
else if (cmd === 'shot') { const r = await send('Page.captureScreenshot', { format: 'png' }); (await import('fs')).writeFileSync(arg, Buffer.from(r.result.data, 'base64')); console.log(arg); }
else if (cmd === 'measure') console.log(JSON.stringify(await ev(MEASURE), null, 1));
else if (cmd === 'where') {
  console.log(await ev(`(() => { const q = ${JSON.stringify(arg)}; const nth = ${JSON.stringify(process.argv[4]||'0')}|0;
    const ok = e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    let c = []; try { c = [...document.querySelectorAll(q)].filter(ok); } catch {}
    if (!c.length) { const all = [...document.querySelectorAll('*')].filter(ok);
      c = all.filter(e => (e.getAttribute('aria-label')||'').trim() === q);
      if (!c.length) c = all.filter(e => e.children.length === 0 && (e.textContent||'').trim() === q);
      if (!c.length) c = all.filter(e => e.innerText && e.innerText.trim() === q).reverse();
      if (!c.length) c = all.filter(e => e.children.length === 0 && (e.textContent||'').includes(q)); }
    const el = c[nth]; if (!el) return 'NOTFOUND';
    let r = el.getBoundingClientRect(); if (r.bottom < 0 || r.top > innerHeight) { el.scrollIntoView({ block: 'center' }); r = el.getBoundingClientRect(); } return Math.round((r.left + r.width/2) * devicePixelRatio) + ' ' + Math.round((r.top + r.height/2) * devicePixelRatio); })()`));
}
else if (cmd === 'click') {
  console.log(JSON.stringify(await ev(`(() => { const q = ${JSON.stringify(arg)};
    let el = null; try { el = [...document.querySelectorAll(q)].find(e => e.getBoundingClientRect().width > 0); } catch {}
    if (!el) { const all = [...document.querySelectorAll('button,a,[role=button],[role=tab],[role=menuitem],[role=option],li,summary,label,div,span')].filter(e => e.getBoundingClientRect().width > 0);
      el = all.find(e => (e.getAttribute('aria-label')||'').trim() === q) || all.find(e => e.innerText && e.innerText.trim() === q) || all.find(e => (e.getAttribute('aria-label')||'').includes(q)); }
    if (!el) return 'NOT FOUND';
    const r = el.getBoundingClientRect(); el.scrollIntoView({block:'center'}); el.click(); return 'clicked '+el.tagName+' '+Math.round(r.width)+'x'+Math.round(r.height); })()`)));
}
ws.close();
