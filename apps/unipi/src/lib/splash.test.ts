import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { splashDelay, splashMin, SPLASH_MIN_MS, SPLASH_MIN_MS_DESKTOP } from './splash';

describe('splashDelay', () => {
  it('holds the splash for the minimum time only', () => {
    expect(splashDelay(0)).toBe(SPLASH_MIN_MS);
    expect(splashDelay(400)).toBe(SPLASH_MIN_MS - 400);
    expect(splashDelay(5000)).toBe(0);
  });
});

describe('splashMin (UNI-218 F15)', () => {
  it('is short on a desktop window, unchanged on the phone app', () => {
    expect(splashMin('tauri-desktop', 1280)).toBe(SPLASH_MIN_MS_DESKTOP);
    expect(splashMin('web', 1440)).toBe(SPLASH_MIN_MS_DESKTOP);
    expect(splashMin('tauri-mobile', 915)).toBe(SPLASH_MIN_MS); // phone in landscape
    expect(splashMin('web', 412)).toBe(SPLASH_MIN_MS);
    expect(SPLASH_MIN_MS_DESKTOP).toBeLessThan(SPLASH_MIN_MS);
  });
});

describe('splash wordmark', () => {
  it('spells UNIPI in the logo colours (style.css --color-wm-*)', () => {
    const html = readFileSync(resolve(__dirname, '../../index.html'), 'utf8');
    const css = readFileSync(resolve(__dirname, '../style.css'), 'utf8');
    const letters = [...html.matchAll(/<span class="(wm-[a-z0-9]+)">([A-Z])<\/span>/g)].map((m) => [m[1], m[2]]);
    expect(letters.map(([, l]) => l).join('')).toBe('UNIPI');
    const colour = (cls: string) => new RegExp(`\\.${cls}\\{color:(#[0-9A-F]{6})\\}`).exec(html)?.[1];
    const tokens = ['white', 'cyan', 'green', 'yellow', 'red'].map(
      (name) => new RegExp(`--color-wm-${name}: (#[0-9A-F]{6})`).exec(css)?.[1],
    );
    expect(letters.map(([cls]) => colour(cls))).toEqual(tokens);
    expect(tokens).toEqual(['#FFFFFF', '#28BEF0', '#5AD25A', '#FAC828', '#F03C32']);
  });
});
