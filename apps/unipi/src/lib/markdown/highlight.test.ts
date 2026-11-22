import { beforeEach, describe, expect, it } from 'vitest';
import { canonicalLang, highlightHtml, isSupportedLang, langForPath, languageForPath, resetHighlightCache } from './highlight';

beforeEach(() => resetHighlightCache());

describe('canonicalLang / isSupportedLang', () => {
  it('normalises common fence aliases', () => {
    expect(canonicalLang('JS')).toBe('javascript');
    expect(canonicalLang('ts')).toBe('typescript');
    expect(canonicalLang('py')).toBe('python');
    expect(canonicalLang('sh')).toBe('shell');
    expect(canonicalLang('  python3 extra')).toBe('python');
  });
  it('flags supported vs. unknown languages', () => {
    expect(isSupportedLang('js')).toBe(true);
    expect(isSupportedLang('rust')).toBe(true);
    expect(isSupportedLang('brainfuck')).toBe(false);
    expect(isSupportedLang('')).toBe(false);
  });
});

describe('highlightHtml', () => {
  it('returns tagged spans for a supported language (CodeMirror/Lezer grammar)', async () => {
    const html = await highlightHtml('const a = 1;', 'javascript');
    expect(html).toContain('tok-keyword');
    expect(html).toContain('const');
  });

  it('returns undefined for an unsupported language (caller keeps plain text)', async () => {
    expect(await highlightHtml('whatever', 'brainfuck')).toBeUndefined();
  });

  it('caches by lang:source — same input returns the identical string instance from cache', async () => {
    const a = await highlightHtml('const x = 1;', 'js');
    const b = await highlightHtml('const x = 1;', 'js');
    expect(a).toBe(b);
  });

  it('loads a legacy-mode language (StreamLanguage) too', async () => {
    const html = await highlightHtml('echo "hi"', 'bash');
    expect(html).toBeDefined();
  });

  it('escapes HTML inside highlighted code', async () => {
    const html = await highlightHtml('const s = "<b>";', 'javascript');
    expect(html).not.toContain('<b>');
    expect(html).toContain('&lt;b&gt;');
  });
});

describe('langForPath / languageForPath (UNI-155 editor)', () => {
  it('maps common file extensions to a fence-style language id', () => {
    expect(langForPath('/home/u/app.ts')).toBe('typescript');
    expect(langForPath('/home/u/script.PY')).toBe('python');
    expect(langForPath('/home/u/README.md')).toBe('markdown');
    expect(langForPath('/home/u/Dockerfile')).toBe('dockerfile');
    expect(langForPath('/home/u/no-extension')).toBeUndefined();
    expect(langForPath('/home/u/unknown.xyz')).toBeUndefined();
  });

  it('resolves a CodeMirror Language for a known path', async () => {
    const lang = await languageForPath('/home/u/app.rs');
    expect(lang).toBeDefined();
  });

  it('resolves undefined for a path with no grammar', async () => {
    expect(await languageForPath('/home/u/unknown.xyz')).toBeUndefined();
  });
});
