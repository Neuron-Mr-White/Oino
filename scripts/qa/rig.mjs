#!/usr/bin/env node
// Drives the real-app QA rig (scripts/qa/rig.sh up) over WebDriver.
//   start | stop
//   shot [name]                 PNG of the real window -> $QA_HOME/shots/<name>.png (prints path)
//   eval '<js>'                 run JS in the app webview, print the JSON result
//   click '<css>'               click the first match
//   type '<css>' 'text'         focus + type into an element (\n = Enter)
//   key '<key>'                 send a key to the active element (Enter, Escape, Tab, ...)
//   hover '<css>'               move the pointer over an element
//   text ['<css>']              innerText of the element (default body), trimmed to 4000 chars
//   wait '<css>' [ms]           wait until an element exists
//   size <w> <h>                resize the window
//   logs                        tail of the host log
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
const QA = process.env.QA_HOME ?? `${process.env.HOME}/.cache/oino-qa`;
const BASE = `http://127.0.0.1:${process.env.QA_PORT ?? 4444}`;
const SESS = `${QA}/run/session`;
const [cmd, a, b] = process.argv.slice(2);
async function wd(method, path, body) {
  const r = await fetch(BASE + path, { method, headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${JSON.stringify(j).slice(0, 400)}`);
  return j.value;
}
const sid = () => readFileSync(SESS, 'utf8').trim();
const S = (p) => `/session/${sid()}${p}`;
async function find(css) { const v = await wd('POST', S('/element'), { using: 'css selector', value: css }); return Object.values(v)[0]; }
const KEYS = { Enter: '\uE007', Escape: '\uE00C', Tab: '\uE004', Backspace: '\uE003', ArrowUp: '\uE013', ArrowDown: '\uE015', ArrowLeft: '\uE012', ArrowRight: '\uE014' };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
switch (cmd) {
  case 'start': {
    if (existsSync(SESS)) { try { await wd('DELETE', S('')); } catch {} }
    const app = readFileSync(`${QA}/run/app-path`, 'utf8').trim();
    const v = await wd('POST', '/session', { capabilities: { alwaysMatch: { 'tauri:options': { application: app } } } });
    writeFileSync(SESS, v.sessionId); await sleep(4000);
    console.log('session', v.sessionId); break;
  }
  case 'stop': await wd('DELETE', S('')); break;
  case 'shot': {
    mkdirSync(`${QA}/shots`, { recursive: true });
    const p = `${QA}/shots/${a ?? 'shot-' + Date.now()}.png`;
    writeFileSync(p, Buffer.from(await wd('GET', S('/screenshot')), 'base64')); console.log(p); break;
  }
  case 'eval': console.log(JSON.stringify(await wd('POST', S('/execute/sync'), { script: `return (function(){ ${a.includes('return') ? a : 'return ' + a} })()`, args: [] }), null, 1)?.slice(0, 6000)); break;
  case 'click': await wd('POST', S(`/element/${await find(a)}/click`), {}); break;
  case 'type': { const el = await find(a); await wd('POST', S(`/element/${el}/click`), {}); await wd('POST', S(`/element/${el}/value`), { text: b.replace(/\\n/g, '\uE007') }); break; }
  case 'key': await wd('POST', S('/actions'), { actions: [{ type: 'key', id: 'k', actions: [{ type: 'keyDown', value: KEYS[a] ?? a }, { type: 'keyUp', value: KEYS[a] ?? a }] }] }); break;
  case 'hover': { const el = await find(a); await wd('POST', S('/actions'), { actions: [{ type: 'pointer', id: 'm', parameters: { pointerType: 'mouse' }, actions: [{ type: 'pointerMove', origin: { 'element-6066-11e4-a52e-4f735466cecf': el }, x: 0, y: 0 }] }] }); break; }
  case 'text': { const v = await wd('POST', S('/execute/sync'), { script: 'const e=document.querySelector(arguments[0]||"body");return e?e.innerText:null', args: [a ?? 'body'] }); console.log(String(v).slice(0, 4000)); break; }
  case 'wait': { const until = Date.now() + Number(b ?? 10000); for (;;) { try { await find(a); console.log('found'); break; } catch { if (Date.now() > until) { console.log('timeout'); process.exit(2); } await sleep(250); } } break; }
  case 'size': await wd('POST', S('/window/rect'), { width: Number(a), height: Number(b) }); break;
  case 'logs': console.log(readFileSync(`${QA}/agent/host/host.log`, 'utf8').replace(/\x1b\[[0-9;]*m/g, '').split('\n').slice(-40).join('\n')); break;
  default: console.log(readFileSync(new URL(import.meta.url)).toString().split('\n').slice(1, 14).join('\n')); process.exit(1);
}
