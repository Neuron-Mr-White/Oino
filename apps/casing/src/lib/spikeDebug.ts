// OINO SPIKE (round 6, dev only): reports the app's UI state to a local sink
// so the headless driver can see what the screens show. Harmless outside dev.
import { isTauri } from './platform';

let last = '';

function sample(): string {
  const w = window as unknown as { __spike?: unknown };
  const path = location.hash;
  const texts: string[] = [];
  // visible buttons/headers cheaply
  document.querySelectorAll<HTMLElement>('h1,h2,h3,p,button,[aria-label],input,textarea,[role=radio],[role=status],[data-testid],[class*=new-chat]').forEach((el) => {
    const tag = el.tagName.toLowerCase();
    let label = el.getAttribute('aria-label') ?? el.getAttribute('data-testid') ?? el.textContent?.trim().slice(0, 60) ?? '';
    if (tag === 'input' || tag === 'textarea') label = `${el.getAttribute('placeholder') ?? ''}=[${(el as HTMLInputElement).value.slice(0, 40)}]`;
    if (el.getAttribute('role') === 'radio') label = `${el.getAttribute('aria-checked') === 'true' ? '(*)' : '( )'} ${label}`;
    if ((el as HTMLButtonElement).disabled) label = `[disabled] ${label}`;
    const r = el.getBoundingClientRect();
    if (r.width > 0 && r.height > 0 && label) {
      texts.push(`${el.tagName.toLowerCase()}@${Math.round(r.x)},${Math.round(r.y)},${Math.round(r.width)}x${Math.round(r.height)}:${label.replace(/\s+/g, ' ').slice(0, 70)}`);
    }
  });
  return JSON.stringify({ tauri: isTauri(), path, items: texts });
}

export function startSpikeDebug(): void {
  if (!import.meta.env.DEV) return;
  setInterval(() => {
    const s = sample();
    if (s === last) return;
    last = s;
    void fetch('http://127.0.0.1:9911/spike', { method: 'POST', body: s }).catch(() => {});
  }, 800);
  void fetch('http://127.0.0.1:9911/hello', { method: 'POST', body: 'boot' }).catch(() => {});
}
