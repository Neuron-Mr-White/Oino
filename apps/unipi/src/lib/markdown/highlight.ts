/**
 * Closed-fence code highlighting (UNI-156): CodeMirror 6 / Lezer grammars,
 * loaded lazily per language and cached by `lang:source`. Plain escaped
 * text renders first (the caller does that synchronously); `highlightHtml`
 * resolves once the grammar (and, for legacy modes, StreamLanguage) has
 * loaded, so callers can swap it in. Unknown/unsupported languages return
 * `undefined` — callers keep the plain rendering.
 */
import { highlightCode, classHighlighter } from '@lezer/highlight';
import type { Language } from '@codemirror/language';
import { escapeHtml } from './escape';

type Loader = () => Promise<Language>;

async function fromModule(get: (mod: any) => Language | { language: Language }, path: Promise<any>): Promise<Language> {
  const mod = await path;
  const result = get(mod);
  return 'language' in result && !('parser' in result) ? result.language : (result as Language);
}

async function legacy(path: Promise<any>, pick: (mod: any) => any): Promise<Language> {
  const { StreamLanguage } = await import('@codemirror/language');
  const mod = await path;
  return StreamLanguage.define(pick(mod));
}

/** Canonical language id ← common fence aliases. */
const ALIASES: Record<string, string> = {
  js: 'javascript', jsx: 'jsx', mjs: 'javascript', cjs: 'javascript',
  ts: 'typescript', tsx: 'tsx',
  py: 'python', py3: 'python', python3: 'python',
  rs: 'rust',
  sh: 'shell', bash: 'shell', zsh: 'shell', console: 'shell',
  yml: 'yaml',
  md: 'markdown',
  htm: 'html',
  'c++': 'cpp', cc: 'cpp', cxx: 'cpp',
};

const LOADERS: Record<string, Loader> = {
  javascript: () => fromModule((m) => m.javascript({ jsx: true, typescript: false }), import('@codemirror/lang-javascript')),
  jsx: () => fromModule((m) => m.javascript({ jsx: true }), import('@codemirror/lang-javascript')),
  typescript: () => fromModule((m) => m.javascript({ typescript: true }), import('@codemirror/lang-javascript')),
  tsx: () => fromModule((m) => m.javascript({ jsx: true, typescript: true }), import('@codemirror/lang-javascript')),
  python: () => fromModule((m) => m.python(), import('@codemirror/lang-python')),
  rust: () => fromModule((m) => m.rust(), import('@codemirror/lang-rust')),
  json: () => fromModule((m) => m.json(), import('@codemirror/lang-json')),
  css: () => fromModule((m) => m.css(), import('@codemirror/lang-css')),
  html: () => fromModule((m) => m.html(), import('@codemirror/lang-html')),
  markdown: () => fromModule((m) => m.markdown(), import('@codemirror/lang-markdown')),
  sql: () => fromModule((m) => m.sql(), import('@codemirror/lang-sql')),
  yaml: () => fromModule((m) => m.yaml(), import('@codemirror/lang-yaml')),
  shell: () => legacy(import('@codemirror/legacy-modes/mode/shell'), (m) => m.shell),
  toml: () => legacy(import('@codemirror/legacy-modes/mode/toml'), (m) => m.toml),
  go: () => legacy(import('@codemirror/legacy-modes/mode/go'), (m) => m.go),
  c: () => legacy(import('@codemirror/legacy-modes/mode/clike'), (m) => m.c),
  cpp: () => legacy(import('@codemirror/legacy-modes/mode/clike'), (m) => m.cpp),
  java: () => legacy(import('@codemirror/legacy-modes/mode/clike'), (m) => m.java),
  csharp: () => legacy(import('@codemirror/legacy-modes/mode/clike'), (m) => m.csharp),
  ruby: () => legacy(import('@codemirror/legacy-modes/mode/ruby'), (m) => m.ruby),
  swift: () => legacy(import('@codemirror/legacy-modes/mode/swift'), (m) => m.swift),
  dockerfile: () => legacy(import('@codemirror/legacy-modes/mode/dockerfile'), (m) => m.dockerFile),
  lua: () => legacy(import('@codemirror/legacy-modes/mode/lua'), (m) => m.lua),
  perl: () => legacy(import('@codemirror/legacy-modes/mode/perl'), (m) => m.perl),
  kotlin: () => legacy(import('@codemirror/legacy-modes/mode/clike'), (m) => m.kotlin),
  xml: () => legacy(import('@codemirror/legacy-modes/mode/xml'), (m) => m.xml),
};

/** Normalises a fence's language token (`ts`, `JS`, `python3`…) to our id. */
export function canonicalLang(lang: string): string {
  const base = lang.trim().toLowerCase().split(/\s+/)[0] ?? '';
  return ALIASES[base] ?? base;
}

export function isSupportedLang(lang: string): boolean {
  return canonicalLang(lang) in LOADERS;
}

/** File extension (lowercase, no dot) → fence-style language id, for the
 * editor (UNI-155): picks the same grammar the chat's code fences use. */
const EXT_TO_LANG: Record<string, string> = {
  js: 'javascript', jsx: 'jsx', mjs: 'javascript', cjs: 'javascript',
  ts: 'typescript', tsx: 'tsx',
  py: 'python',
  rs: 'rust',
  sh: 'shell', bash: 'shell', zsh: 'shell',
  yml: 'yaml', yaml: 'yaml',
  md: 'markdown', markdown: 'markdown',
  html: 'html', htm: 'html',
  css: 'css',
  json: 'json', jsonc: 'json',
  sql: 'sql',
  toml: 'toml',
  go: 'go',
  c: 'c', h: 'c',
  cpp: 'cpp', cc: 'cpp', cxx: 'cpp', hpp: 'cpp',
  java: 'java',
  cs: 'csharp',
  rb: 'ruby',
  swift: 'swift',
  lua: 'lua',
  pl: 'perl',
  kt: 'kotlin', kts: 'kotlin',
  xml: 'xml',
  dockerfile: 'dockerfile',
};

/** The canonical language id for a file path's extension (or basename for
 * `Dockerfile`), or undefined when there is no grammar for it. */
export function langForPath(path: string): string | undefined {
  const base = path.split('/').pop() ?? path;
  if (/^dockerfile$/i.test(base)) return 'dockerfile';
  const dot = base.lastIndexOf('.');
  if (dot < 0) return undefined;
  const ext = base.slice(dot + 1).toLowerCase();
  return EXT_TO_LANG[ext];
}

/** Resolves the CodeMirror `Language` for a file path, loading its
 * grammar lazily (same cache as `highlightHtml`). Undefined when the
 * extension has no grammar (the editor still works, just unhighlighted). */
export async function languageForPath(path: string): Promise<Language | undefined> {
  const lang = langForPath(path);
  if (!lang) return undefined;
  const promise = loadLanguage(lang);
  return promise ? promise : undefined;
}

const langCache = new Map<string, Promise<Language>>();
function loadLanguage(id: string): Promise<Language> | undefined {
  const loader = LOADERS[id];
  if (!loader) return undefined;
  let p = langCache.get(id);
  if (!p) {
    p = loader();
    langCache.set(id, p);
  }
  return p;
}

const htmlCache = new Map<string, string>();
const HTML_CACHE_MAX = 500;

/** Highlights `source` for `lang`, resolving once the grammar is ready. Cached by `lang:source`. */
export async function highlightHtml(source: string, lang: string): Promise<string | undefined> {
  const id = canonicalLang(lang);
  const key = `${id}:${source}`;
  const hit = htmlCache.get(key);
  if (hit !== undefined) return hit;
  const languagePromise = loadLanguage(id);
  if (!languagePromise) return undefined;
  const language = await languagePromise;
  const tree = language.parser.parse(source);
  let out = '';
  highlightCode(
    source,
    tree,
    classHighlighter,
    (code, classes) => {
      out += classes ? `<span class="${classes}">${escapeHtml(code)}</span>` : escapeHtml(code);
    },
    () => {
      out += '\n';
    },
  );
  if (htmlCache.size > HTML_CACHE_MAX) htmlCache.clear();
  htmlCache.set(key, out);
  return out;
}

/** Clears the in-memory caches (tests). */
export function resetHighlightCache(): void {
  langCache.clear();
  htmlCache.clear();
}
