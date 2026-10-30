/**
 * File-path candidates in chat text (UNI-204 rework). Pure: no DOM, no
 * Solid — the transcript renderer, the user-bubble renderer, tool output
 * and the detection-report script all share it.
 *
 * This only finds *candidates*. Nothing becomes tappable until the bridge
 * confirms the path exists on the PC (`paths_stat`, lib/chat/pathStat.ts),
 * so the scanner can be generous about shapes that are common in real
 * sessions (bare `Composer.tsx`, `src/lib/foo.ts`, `path:12:5`, tsc's
 * `path(12,5)`, `file:///…` stack frames, grep/rg output) while still
 * refusing the obvious non-paths up front (URLs, version numbers, npm
 * package names, slash commands, dates, method calls) to keep the stat
 * batches small.
 *
 * Shapes (see fileRefScan.test.ts and docs/m5/fixtures/uni204-corpus.json):
 * - explicit: `@path`, `~/…`, `./…`, `../…`, `/abs/…` (≥ 2 segments, or one
 *   with a known extension), `file:///abs…`;
 * - relative with a slash: `src/lib/foo.ts`, `packages/dream/`; in prose a
 *   2-segment one needs an extension or a trailing slash (`and/or`);
 * - bare names with a known extension (`Composer.tsx`, `package.json`) or a
 *   well-known name (`Makefile`, `Dockerfile`); dotfiles in code;
 * - any of the above + `:line[:col]`, `(line,col)` or `#Lline`;
 * - Python's `File "x.py", line 12`.
 */

export type ScanMode = 'prose' | 'code';

export type FileRefCandidate = {
  /** Span in the scanned text to underline (includes a leading `@`/`file://` and the `:line` suffix). */
  start: number;
  end: number;
  /** The path as written, with `@`/`file://` stripped and the line suffix removed (trailing `/` kept). */
  path: string;
  line?: number;
  col?: number;
  /** Written with an explicit path marker (`@`, `~/`, `./`, `../`, `/`, `file://`). */
  explicit: boolean;
};

/** File extensions worth treating a bare `name.ext` as a file mention. */
export const KNOWN_EXT = new Set(
  (
    'ts tsx mts cts js jsx mjs cjs json jsonc json5 jsonl ndjson md mdx markdown txt rst adoc rs toml yaml yml lock ' +
    'py pyi ipynb sh zsh fish ps1 bat go mod sum java kt kts gradle swift mm c h cc cpp cxx hpp hh cs fs rb php pl lua ' +
    'dart ex exs erl hrl hs ml mli zig nim nix proto graphql gql sql prisma html htm css scss sass less styl vue svelte astro ' +
    'svg png jpg jpeg gif webp avif ico bmp tiff pdf log csv tsv xml xsd plist ini cfg conf properties service desktop ' +
    'mp4 mp3 wav ogg oga webm mov mkv flac m4a zip tar gz tgz xz bz2 zst 7z apk aab ipa deb rpm dmg wasm map patch diff ' +
    'db sqlite sqlite3 tf tfvars hcl dockerfile makefile cmake mk bazel bzl kdl ron tex bib org snap tmpl tpl j2 hbs ejs pug ' +
    'ttf otf woff woff2 jar war class so dylib dll exe bin out pem crt pub keystore jks'
  ).split(/\s+/),
);

/** Extension-less file names worth linking when bare. */
const KNOWN_NAMES = new Set(['Makefile', 'Dockerfile', 'Justfile', 'Procfile', 'Rakefile', 'Gemfile', 'Brewfile', 'Containerfile', 'Vagrantfile', 'LICENSE', 'CODEOWNERS']);
/** Dotfiles worth linking even in prose. */
const KNOWN_DOTFILES = new Set(['.gitignore', '.gitattributes', '.env', '.npmrc', '.nvmrc', '.editorconfig', '.prettierrc', '.eslintrc', '.dockerignore', '.gitmodules', '.tool-versions', '.envrc']);
/** A first segment that's really a host name (`github.com/a/b`): a URL without its scheme. */
const DOMAIN_SEG = /^[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:com|org|net|io|dev|ai|app|co|me|my|one|sh|gg|xyz|so|to|ly|be|de|uk|us|cn|ru|jp|edu|gov|info|biz|tv|cc|nrn)$/i;

// Tokens: runs of non-delimiter characters, optionally followed by tsc's `(line,col)`.
const TOKEN_RE = /[^\s"'`<>()[\]{},;|*\\]+(?:\(\d+,\d+\))?/g;
// Inside a token: the path, an optional location suffix, and whatever follows a `:` (grep/rg content).
const PARSE_RE = /^([\p{L}\p{N}_.@+~\-/%$]+?)(?::(\d+)(?::(\d+))?|\((\d+),(\d+)\)|#L(\d+)(?:-L?\d+)?)?(:.*)?$/u;
const PY_FRAME_RE = /File "([^"\n]+)", line (\d+)/g;
const TRAIL_PUNCT = /[.,:;!?…]+$/;

const extOf = (seg: string): string | undefined => {
  const i = seg.lastIndexOf('.');
  return i > 0 && i < seg.length - 1 ? seg.slice(i + 1) : undefined;
};
const hasLetterExt = (seg: string) => {
  const e = extOf(seg);
  return !!e && /^[A-Za-z0-9]{1,10}$/.test(e) && /[A-Za-z]/.test(e);
};

/** Whether `p` (no `@`/`file://`/line suffix) is a path shape worth a stat. */
export function plausiblePath(p: string, mode: ScanMode, hadAt = false): boolean {
  if (!p || p.length > 1024) return false;
  if (!/[\p{L}_]/u.test(p)) return false; // `1/2`, `10/09/2026`, `///`
  const dir = p.endsWith('/');
  const body = p.replace(/\/+$/, '');
  if (p.startsWith('~/')) return body.length > 2 && !p.includes('//');
  if (p === '~' || p === '/' || p.startsWith('//')) return false;
  if (p.startsWith('./') || p.startsWith('../')) return body.replace(/^(?:\.\.?\/)+/, '').length > 0;
  const segs = body.split('/');
  if (p.startsWith('/')) {
    segs.shift();
    if (segs.some((s) => !s)) return false;
    // One segment: `/tmp`, `/model` (a slash command) — only with a real extension.
    if (segs.length === 1) return KNOWN_EXT.has((extOf(segs[0]!) ?? '').toLowerCase());
    return true;
  }
  if (segs.some((s) => !s)) return false;
  const last = segs[segs.length - 1]!;
  if (segs.length >= 2) {
    if (DOMAIN_SEG.test(segs[0]!)) return false; // github.com/a/b
    if (segs.every((s) => /^[\d.]+$/.test(s))) return false;
    if (hadAt && segs.length === 2 && !hasLetterExt(last)) return false; // @scope/pkg
    if (/^v?\d+(?:\.\d+)+$/.test(last) && segs.length === 2) return false; // pkg/1.2.3
    if (mode === 'prose' && segs.length === 2 && !dir && !hasLetterExt(last)) return false; // and/or, read/write
    return true;
  }
  // Bare name.
  if (dir) return /^\.?[\p{L}_][\w.-]*$/u.test(body) && !/^v?\d/.test(body); // `docs/`, `.git/`
  if (KNOWN_NAMES.has(body)) return true;
  if (body.startsWith('.')) {
    if (KNOWN_DOTFILES.has(body)) return true;
    // In code: `.unipi`, `.mise.local` — but not a lone extension (`.md`, `.test.ts`).
    const rest = body.slice(1);
    // `.unipi-workspace.json`, `.eslintrc.json`: a dotfile with a real extension, told apart from a
    // compound extension (`.test.ts`, `.d.ts`) by a `-`/`_` or an `rc` name.
    const restExt = extOf(rest);
    if (mode === 'code' && restExt && KNOWN_EXT.has(restExt.toLowerCase())) {
      const name = rest.slice(0, rest.length - restExt.length - 1);
      if (/^[a-z][\w-]*$/i.test(name) && (/[-_]/.test(name) || /rc$/i.test(name))) return true;
    }
    return mode === 'code' && /^[a-z][\w.-]*$/i.test(rest) && !KNOWN_EXT.has(rest.toLowerCase()) && !KNOWN_EXT.has((extOf(rest) ?? '').toLowerCase());
  }
  const ext = extOf(body);
  if (!ext || !KNOWN_EXT.has(ext.toLowerCase())) return false;
  const name = body.slice(0, body.length - ext.length - 1);
  if (!/[\p{L}_]/u.test(name)) return false; // `1.2.ts`?
  if (/^v?\d+(?:\.\d+)*$/.test(name)) return false;
  return true;
}

/** Finds file-path candidates in `text` (see module doc). Non-overlapping, in order. */
export function scanFileRefs(text: string, mode: ScanMode = 'prose'): FileRefCandidate[] {
  const out: FileRefCandidate[] = [];
  const taken: Array<[number, number]> = [];
  const free = (s: number, e: number) => taken.every(([a, b]) => e <= a || s >= b);

  for (const m of text.matchAll(PY_FRAME_RE)) {
    const p = m[1]!;
    if (!plausiblePath(p, 'code')) continue;
    const start = m.index! + 6;
    out.push({ start, end: start + p.length, path: p, line: Number(m[2]), explicit: p.startsWith('/') });
    taken.push([start, start + p.length]);
  }

  for (const m of text.matchAll(TOKEN_RE)) {
    let tok = m[0];
    let start = m.index!;
    if (!free(start, start + tok.length)) continue;
    // Glued to a closing bracket or a glob star: `foo().length`, `arr[0].ts`, `**/*.test.ts`.
    const before = text[start - 1];
    if (before === ')' || before === ']' || before === '}' || before === '*') continue;
    // `--file=src/a.ts`, `path=/x`: the value only.
    const kv = /^[-\w]*=/.exec(tok);
    if (kv) {
      tok = tok.slice(kv[0].length);
      start += kv[0].length;
    }
    let lead = 0;
    let explicit = false;
    let hadAt = false;
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(tok)) {
      if (!/^file:\/\/\//i.test(tok)) continue; // a URL
      lead = 7;
      explicit = true;
    } else if (tok.startsWith('@')) {
      // `@` must open the token (an email's `a@b.c` doesn't), and `@@x` isn't a mention.
      if (tok[1] === '@' || tok.length < 2) continue;
      lead = 1;
      hadAt = true;
      explicit = true;
    } else if (tok.includes('://')) continue;
    // Something glued to a path-ish token by `:` in front (`error:src/a.ts`) is rare — skip leading-colon cases.
    let body = tok.slice(lead);
    const trimmed = body.replace(TRAIL_PUNCT, (punct) => {
      // Keep the `:` of a grep `path:12:` (handled by PARSE_RE's rest), drop sentence punctuation.
      return /^:\d/.test(punct) ? punct : '';
    });
    body = trimmed;
    const pm = PARSE_RE.exec(body);
    if (!pm) continue;
    let p = pm[1]!;
    // A sentence-ending `.` after the path (`see src/a.ts.`) — already trimmed; a `.` glued before `:12` is not a thing.
    p = p.replace(/\.+$/, '') || p;
    if (!p) continue;
    if (/^(?:~|\.\.?)?\//.test(p) || p.startsWith('~')) explicit = true;
    if (lead === 7 && !p.startsWith('/')) continue;
    // `foo.bar(` — a method call, not a file (tsc's `(l,c)` was consumed by PARSE_RE).
    const after = text[m.index! + m[0].length];
    if (after === '(' && !pm[4]) continue;
    if (!plausiblePath(p, mode, hadAt)) continue;
    const lineStr = pm[2] ?? pm[4] ?? pm[6];
    const colStr = pm[3] ?? pm[5];
    let suffixLen = 0;
    if (pm[2]) suffixLen = 1 + pm[2].length + (pm[3] ? 1 + pm[3].length : 0);
    else if (pm[4]) suffixLen = 3 + pm[4].length + pm[5]!.length;
    else if (pm[6]) suffixLen = body.length - p.length - (pm[7]?.length ?? 0);
    const end = start + lead + p.length + suffixLen;
    out.push({
      start,
      end,
      path: p,
      line: lineStr ? Number(lineStr) : undefined,
      col: colStr ? Number(colStr) : undefined,
      explicit,
    });
    taken.push([start, end]);
  }
  return out.sort((a, b) => a.start - b.start);
}

/** Joins a relative candidate onto `base` (a dir relative to the cwd, or absolute) — for tool output
 * that prints paths relative to the tool's own `path` argument (grep/find/ls). Explicit absolute/home
 * paths pass through. */
export function joinBase(path: string, base: string | undefined): string {
  if (!base || path.startsWith('/') || path.startsWith('~')) return path;
  const b = base.replace(/\/+$/, '');
  if (!b || b === '.') return path;
  return `${b}/${path.replace(/^\.\//, '')}`;
}

/** The directory a tool's `path` argument names, for `joinBase` (a file arg → its parent dir). */
export function toolBaseDir(arg: unknown): string | undefined {
  if (typeof arg !== 'string' || !arg.trim()) return undefined;
  const p = arg.trim().replace(/\/+$/, '');
  const lastSeg = p.split('/').pop() ?? '';
  if (hasLetterExt(lastSeg)) {
    const i = p.lastIndexOf('/');
    return i > 0 ? p.slice(0, i) : i === 0 ? '/' : undefined;
  }
  return p;
}
