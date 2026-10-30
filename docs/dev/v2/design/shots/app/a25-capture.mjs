#!/usr/bin/env node
/**
 * WP-A25 capture driver (app spec §1 "Visual verification"): the harness app on
 * http://127.0.0.1:1450 with the MockHost scenarios, desktop 1280×820 and phone
 * 412×915, one PNG per shot into this folder.
 *
 *   node docs/dev/v2/design/shots/app/a25-capture.mjs [shot-name…]
 *
 * Needs `npx vite --config vite.config.capture.ts` (port 1450) running in
 * apps/unipi. Uses a private headless Chromium over CDP (the shared
 * agent-browser tab belongs to whoever is watching it); no app or host state
 * outside /tmp is touched.
 */
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

const BASE = 'http://127.0.0.1:1450';
const OUT = new URL('.', import.meta.url).pathname;
const CHROME =
  process.env.CHROME_BIN ??
  '/home/oi/.cache/ms-playwright/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell';
const PORT = 9600 + (process.pid % 200);
const PROFILE = `/tmp/a25-cap-${process.pid}`;

async function launch() {
  const args = [
    '--headless=new',
    `--remote-debugging-port=${PORT}`,
    `--user-data-dir=${PROFILE}`,
    '--no-sandbox',
    '--hide-scrollbars',
    '--force-device-scale-factor=1',
    '--font-render-hinting=none',
    'about:blank',
  ];
  const proc = spawn(CHROME, args, { stdio: 'ignore', detached: true });
  process.on('exit', () => {
    try {
      process.kill(-proc.pid, 'SIGKILL');
    } catch {}
  });
  for (let i = 0; i < 100; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${PORT}/json/version`);
      if (res.ok) return proc;
    } catch {}
    await sleep(150);
  }
  throw new Error('chrome did not start');
}

class Conn {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.pending = new Map();
    ws.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.id && this.pending.has(message.id)) {
        this.pending.get(message.id)(message);
        this.pending.delete(message.id);
      }
    };
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.onopen = resolve;
      ws.onerror = reject;
    });
    return new Conn(ws);
  }
  send(method, params = {}, timeout = 25000) {
    const id = ++this.id;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`cdp timeout: ${method}`));
      }, timeout);
      this.pending.set(id, (message) => {
        clearTimeout(timer);
        resolve(message);
      });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
}

class Page {
  constructor(conn) {
    this.conn = conn;
  }
  static async open(width, height, mobile) {
    const created = await (await fetch(`http://127.0.0.1:${PORT}/json/new?about:blank`, { method: 'PUT' })).json();
    const page = new Page(await Conn.connect(created.webSocketDebuggerUrl));
    await page.send('Page.enable');
    await page.send('Runtime.enable');
    await page.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile });
    return page;
  }
  send(method, params = {}, timeout = 25000) {
    return this.conn.send(method, params, timeout);
  }
  async eval(expression) {
    const result = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.result?.exceptionDetails) throw new Error(JSON.stringify(result.result.exceptionDetails).slice(0, 400));
    return result.result?.result?.value;
  }
  async goto(url, settle = 2200) {
    await this.send('Page.navigate', { url });
    await this.waitFor(`document.querySelector('[data-testid=app-shell]') || document.querySelector('[data-testid=phone-shell]')`, 25000);
    await sleep(settle);
  }
  async waitFor(expression, timeout = 15000) {
    const started = Date.now();
    while (Date.now() - started < timeout) {
      try {
        if (await this.eval(`!!(${expression})`)) return true;
      } catch {}
      await sleep(120);
    }
    throw new Error(`timeout waiting for ${expression}`);
  }
  async click(selector) {
    const ok = await this.eval(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return false; el.click(); return true; })()`);
    if (!ok) throw new Error(`no element ${selector}`);
    await sleep(600);
  }
  async shot(file) {
    const result = await this.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, 30000);
    mkdirSync(OUT, { recursive: true });
    writeFileSync(`${OUT}${file}`, Buffer.from(result.result.data, 'base64'));
    console.log('wrote', file);
  }
  async close() {
    try {
      await this.send('Page.close');
    } catch {}
    try {
      this.conn.ws.close();
    } catch {}
  }
}

const url = (scenario, route, shell) => `${BASE}/?scenario=${scenario}&shell=${shell}#${route}`;

/** One shot per shell: route + optional pre-shot interaction. */
const SHOTS = [
  // `scrollTo` = the section (or row) a shot is about; the driver scrolls its
  // parent so the shot is not all hero text.
  { name: 'mcp-desktop', scenario: 'cap-mcp', route: '/settings/mcp', shell: 'desktop', scrollTo: '#sec-mcp' },
  { name: 'mcp-phone', scenario: 'cap-mcp', route: '/settings/mcp', shell: 'phone', scrollTo: '#sec-mcp' },
  { name: 'mcp-nochat-desktop', scenario: 'cap-mcp-nochat', route: '/settings/mcp', shell: 'desktop', scrollTo: '[data-testid="cap-mcp-nochat"]' },
  { name: 'mcp-nochat-phone', scenario: 'cap-mcp-nochat', route: '/settings/mcp', shell: 'phone', scrollTo: '[data-testid="cap-mcp-nochat"]' },
  { name: 'mcp-manage-phone', scenario: 'cap-mcp', route: '/settings/mcp', shell: 'phone', scrollTo: '[data-testid="manage-mcp.status"]' },
  { name: 'memory-manage-phone', scenario: 'cap-memory', route: '/settings/memory', shell: 'phone', scrollTo: '[data-testid="manage-memory.browse"]' },
  { name: 'skills-manage-phone', scenario: 'cap-skills', route: '/settings/skills', shell: 'phone', scrollTo: '[data-testid="manage-skills.hub"]' },
  { name: 'agents-manage-phone', scenario: 'cap-agents', route: '/settings/subagents', shell: 'phone', scrollTo: '[data-testid="manage-agents.profiles"]' },
  { name: 'providers-nochat-phone', scenario: 'providers-nochat', route: '/settings/providers', shell: 'phone', scrollTo: '[data-testid="providers-nochat"]' },
  { name: 'mcp-panel-desktop', scenario: 'cap-mcp-panel', route: '/settings/mcp', shell: 'desktop', settle: 3200 },
  { name: 'mcp-panel-phone', scenario: 'cap-mcp-panel', route: '/settings/mcp', shell: 'phone', settle: 3200 },
  { name: 'memory-desktop', scenario: 'cap-memory', route: '/settings/memory', shell: 'desktop', scrollTo: '#sec-memory' },
  { name: 'memory-phone', scenario: 'cap-memory', route: '/settings/memory', shell: 'phone', scrollTo: '#sec-memory' },
  { name: 'memory-panel-desktop', scenario: 'cap-memory-panel', route: '/settings/memory', shell: 'desktop', settle: 3400 },
  { name: 'memory-panel-phone', scenario: 'cap-memory-panel', route: '/settings/memory', shell: 'phone', settle: 3400 },
  { name: 'skills-desktop', scenario: 'cap-skills', route: '/settings/skills', shell: 'desktop', scrollTo: '#sec-skills' },
  { name: 'skills-phone', scenario: 'cap-skills', route: '/settings/skills', shell: 'phone', scrollTo: '#sec-skills' },
  { name: 'skills-panel-desktop', scenario: 'cap-skills-panel', route: '/settings/skills', shell: 'desktop', settle: 3400 },
  { name: 'agents-desktop', scenario: 'cap-agents', route: '/settings/subagents', shell: 'desktop', scrollTo: '#sec-subagents' },
  { name: 'agents-phone', scenario: 'cap-agents', route: '/settings/subagents', shell: 'phone', scrollTo: '#sec-subagents' },
  { name: 'agents-panel-desktop', scenario: 'cap-agents-panel', route: '/settings/subagents', shell: 'desktop', settle: 3400 },
  { name: 'providers-desktop', scenario: 'providers', route: '/settings/providers', shell: 'desktop' },
  { name: 'providers-phone', scenario: 'providers', route: '/settings/providers', shell: 'phone', scrollTo: '#sec-providers' },
  { name: 'providers-nochat-desktop', scenario: 'providers-nochat', route: '/settings/providers', shell: 'desktop' },
  { name: 'import-desktop', scenario: 'cap-import', route: '/settings/mcp', shell: 'desktop', settle: 3000 },
  { name: 'import-phone', scenario: 'cap-import', route: '/settings/mcp', shell: 'phone', settle: 3000 },
  { name: 'palette-manage-desktop', scenario: 'cap-palette', route: '/s/local/cap001', shell: 'desktop', settle: 3000 },
  { name: 'about-desktop', scenario: 'cap-mcp', route: '/settings/about', shell: 'desktop', settle: 2600 },
  { name: 'about-phone', scenario: 'cap-mcp', route: '/settings/about', shell: 'phone', settle: 2600, scrollTo: '#sec-about' },
  { name: 'cycle-set-desktop', scenario: 'model-cycle-set', route: '/s/this-pc/s001', shell: 'desktop', settle: 2800 },
  { name: 'cycle-set-phone', scenario: 'model-cycle-set', route: '/s/this-pc/s001', shell: 'phone', settle: 2800 },
];

const only = process.argv.slice(2);
const wanted = only.length ? SHOTS.filter((shot) => only.some((name) => shot.name.includes(name))) : SHOTS;

await launch();
for (const shot of wanted) {
  const page = await Page.open(shot.shell === 'phone' ? 412 : 1280, shot.shell === 'phone' ? 915 : 820, shot.shell === 'phone');
  try {
    await page.goto(url(shot.scenario, shot.route, shot.shell), shot.settle ?? 2400);
    if (shot.scrollTo) {
      // Bring a section/row into frame: scroll its nearest scroll parent
      // (`.app-content` in both shells) so a shot shows the thing it is about.
      await page.eval(`(() => {
        const el = document.querySelector(${JSON.stringify(shot.scrollTo)});
        const scroller = el?.closest('.app-content') || document.querySelector('.app-content') || document.scrollingElement;
        if (!el || !scroller) return false;
        scroller.scrollTop += el.getBoundingClientRect().top - 90;
        return true;
      })()`);
      await sleep(500);
    }
    await page.shot(`a25-${shot.name}.png`);
  } catch (error) {
    console.error(`failed ${shot.name}: ${error.message}`);
  } finally {
    await page.close();
  }
}
process.exit(0);
