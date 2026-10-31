import { describe, expect, it } from 'vitest';
import { joinBase, plausiblePath, scanFileRefs, toolBaseDir } from './fileRefScan';

const paths = (text: string, mode: 'prose' | 'code' = 'prose') => scanFileRefs(text, mode).map((c) => c.path);
const one = (text: string, mode: 'prose' | 'code' = 'prose') => {
  const c = scanFileRefs(text, mode);
  expect(c).toHaveLength(1);
  return c[0]!;
};

describe('scanFileRefs — shapes seen in real sessions (UNI-204)', () => {
  it('explicit forms: @path, ~/, ./, ../, absolute', () => {
    expect(paths('see @src/lib/foo.ts and ~/notes/todo.md, ./run.sh, ../x/b.rs or /home/u/p/a.ts')).toEqual([
      'src/lib/foo.ts',
      '~/notes/todo.md',
      './run.sh',
      '../x/b.rs',
      '/home/u/p/a.ts',
    ]);
  });

  it('bare relative paths with a slash', () => {
    expect(paths('edit packages/fusion/src/picker.ts then crates/kanboard/src/commands.rs')).toEqual(['packages/fusion/src/picker.ts', 'crates/kanboard/src/commands.rs']);
  });

  it('bare file names with a known extension', () => {
    expect(paths('updated Composer.tsx, package.json and CHANGELOG.md')).toEqual(['Composer.tsx', 'package.json', 'CHANGELOG.md']);
  });

  it('directories: trailing slash, or ≥ 3 segments', () => {
    expect(paths('look in packages/dream/ and unipi-app/apps/mobile')).toEqual(['packages/dream/', 'unipi-app/apps/mobile']);
    expect(paths('~/.unipi/config/notify/')).toEqual(['~/.unipi/config/notify/']);
    expect(paths('?? docs/\n?? .git/', 'code')).toEqual(['docs/', '.git/']);
  });

  it('path:line and path:line:col (grep, rg, tsc --pretty, eslint)', () => {
    const c = one('crates/kanboard/src/serve/mod.rs:208');
    expect(c).toMatchObject({ path: 'crates/kanboard/src/serve/mod.rs', line: 208, start: 0, end: 36 });
    expect(one('at src/a.ts:12:5 here')).toMatchObject({ path: 'src/a.ts', line: 12, col: 5, start: 3, end: 16 });
  });

  it('grep output lines: path:line: content', () => {
    const c = one('picker.ts:15:  *   ↑/↓ select', 'code');
    expect(c).toMatchObject({ path: 'picker.ts', line: 15, start: 0, end: 12 });
    expect(paths('input-shortcuts/README.md:52: | `chordKey` | `alt+s` |', 'code')).toEqual(['input-shortcuts/README.md']);
  });

  it("tsc's path(line,col)", () => {
    expect(one("src/lib/x.ts(12,5): error TS2322: Type 'a'", 'code')).toMatchObject({ path: 'src/lib/x.ts', line: 12, col: 5, end: 18 });
  });

  it('GitHub-style #L anchors', () => {
    expect(one('see docs/a.md#L40')).toMatchObject({ path: 'docs/a.md', line: 40 });
  });

  it('file:// URLs (node stack traces)', () => {
    const c = one('    at file:///home/u/p/node_modules/tsx/dist/register.mjs:1:789', 'code');
    expect(c).toMatchObject({ path: '/home/u/p/node_modules/tsx/dist/register.mjs', line: 1, col: 789, explicit: true });
    expect(c.start).toBe(7);
  });

  it('node stack frames in parentheses', () => {
    expect(one('    at run (/home/u/p/src/a.js:10:3)', 'code')).toMatchObject({ path: '/home/u/p/src/a.js', line: 10, col: 3 });
  });

  it('python tracebacks', () => {
    expect(one('  File "/home/u/p/app.py", line 12, in main', 'code')).toMatchObject({ path: '/home/u/p/app.py', line: 12 });
  });

  it('--flag=value and key=value', () => {
    expect(paths('cargo test --manifest-path=crates/host/Cargo.toml', 'code')).toEqual(['crates/host/Cargo.toml']);
  });

  it('trailing sentence punctuation is not part of the path', () => {
    expect(paths('see src/a.ts. Then src/b.ts, and src/c.ts!')).toEqual(['src/a.ts', 'src/b.ts', 'src/c.ts']);
    expect(one('(see `x`) in docs/plan.md:')).toMatchObject({ path: 'docs/plan.md' });
  });

  it('git status / porcelain lines', () => {
    expect(paths(' M packages/fusion/src/nudge.ts\n?? packages/dream/\nR  a/old.ts -> a/new.ts', 'code')).toEqual(['packages/fusion/src/nudge.ts', 'packages/dream/', 'a/old.ts', 'a/new.ts']);
  });

  it('dotfiles: well-known ones in prose, any in code', () => {
    expect(paths('add it to .gitignore')).toEqual(['.gitignore']);
    expect(paths('.mise.local', 'code')).toEqual(['.mise.local']);
    // A dotfile with a real extension (seen in an `ls` answer): code only, and not a compound extension.
    expect(paths('.unipi-workspace.json .eslintrc.json .prettierrc.yml', 'code')).toEqual(['.unipi-workspace.json', '.eslintrc.json', '.prettierrc.yml']);
    expect(paths('.d.ts .spec.tsx .module.css', 'code')).toEqual([]);
    expect(paths('the .foo thing')).toEqual([]);
  });

  it('Makefile and friends', () => {
    expect(paths('run the Makefile target')).toEqual(['Makefile']);
  });
});

describe('scanFileRefs — false positives it must refuse', () => {
  it('URLs (any scheme but file://) and scheme-less domains', () => {
    expect(paths('see https://example.com/a/b.ts and ssh://git@x/y.git or github.com/foo/bar')).toEqual([]);
  });

  it('version numbers, dates, fractions, times', () => {
    expect(paths('bump to 1.2.3, v3.0.0-alpha.35 on 2026/10/09 at 12:30, 1/2 done, 0.87.1')).toEqual([]);
  });

  it('npm package names', () => {
    expect(paths('install @scope/pkg and @scope/pkg-tui')).toEqual([]);
  });

  it('a/b in prose', () => {
    expect(paths('and/or read/write I/O yes/no client/server')).toEqual([]);
  });

  it('slash commands and single-segment absolute words', () => {
    expect(paths('type /model or /unipi:goal, check /tmp')).toEqual([]);
  });

  it('emails and @mentions', () => {
    expect(paths('mail me@x.com or ping @alice')).toEqual([]);
  });

  it('method calls and property access', () => {
    expect(paths('call foo.ts(1) — no, call path.join() and console.log(x)')).toEqual([]);
    expect(paths('use window.innerWidth or this.state.cwd')).toEqual([]);
  });

  it('regexes and globs', () => {
    expect(paths('match /^[a-z]+$/ or **/*.ts')).toEqual([]);
  });

  it('bare words with an unknown extension', () => {
    expect(paths('e.g. foo.bar, i.e. Node.prototype')).toEqual([]);
  });

  it('member access after a call/index and glob tails', () => {
    expect(paths('items().length and arr[0].ts and **/*.test.ts and {a}.json', 'code')).toEqual([]);
  });

  it('a lone extension is not a dotfile', () => {
    expect(paths('files ending in .md or .test.ts', 'code')).toEqual([]);
  });

  it('a lone ~ or /', () => {
    expect(paths('cd ~ then / and ~/')).toEqual([]);
  });
});

describe('plausiblePath', () => {
  it('@scope/pkg vs @scope/pkg/file.ts', () => {
    expect(plausiblePath('scope/pkg', 'prose', true)).toBe(false);
    expect(plausiblePath('scope/pkg/index.ts', 'prose', true)).toBe(true);
    expect(plausiblePath('src/a.ts', 'prose', true)).toBe(true);
  });
  it('2-segment in code needs nothing extra', () => {
    expect(plausiblePath('crates/kanboard', 'code')).toBe(true);
    expect(plausiblePath('crates/kanboard', 'prose')).toBe(false);
  });
});

describe('joinBase / toolBaseDir', () => {
  it('joins tool-relative output onto the tool path', () => {
    expect(joinBase('README.md', 'packages')).toBe('packages/README.md');
    expect(joinBase('./a.ts', 'src/')).toBe('src/a.ts');
    expect(joinBase('/abs.ts', 'src')).toBe('/abs.ts');
    expect(joinBase('a.ts', undefined)).toBe('a.ts');
  });
  it('a file argument means its directory', () => {
    expect(toolBaseDir('packages/fusion/src/picker.ts')).toBe('packages/fusion/src');
    expect(toolBaseDir('packages')).toBe('packages');
    expect(toolBaseDir('picker.ts')).toBeUndefined();
    expect(toolBaseDir(undefined)).toBeUndefined();
  });
});
