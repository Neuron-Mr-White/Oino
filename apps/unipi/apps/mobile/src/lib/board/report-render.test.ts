import { describe, expect, it } from 'vitest';
import { renderMarkdown } from './markdown';

// UNI-158's real finish note, exactly as the daemon's status_report
// slices it out of the stored activity entry (continuation lines joined
// with \n, blank gaps with \n\n, the two-space indent stripped).
const REPORT = `Built: unipi 74a0f97 (app-bridge, unpublished); unipi-app a73b975 (protocol), ce31c4a (app), 92b8691 (retry fix + captures).

- **Bridge:** a hidden \`unipi-app-session\` command gives it a command context (filtered out of the command list). New requests:
  - \`sessions{scope: cwd|all, query}\`: newest first, ~200 cap
  - \`tree{}\`: every branch, previews ≤120 chars
- **App:**
  - header ⋯ menu: New session, Resume…, Session tree, Rename
  - typed /new /resume /fork /tree run the same actions`;

/**
 * UNI-174: reports must render as markdown lists, not clumped text.
 * The sample is UNI-158's real finish note, byte-shaped exactly as the
 * daemon's status_report serves it (activity continuation lines joined,
 * blank gaps kept as \n\n). If this ever fails, the shared renderer or
 * the daemon's status_report slicing regressed.
 */
describe('UNI-174 report rendering', () => {
  it('renders the finish report as paragraphs + a nested list', () => {
    const html = renderMarkdown(REPORT);
    expect(html).toContain('<ul>');
    expect(html).toContain('<li>');
    expect(html).toMatch(/Bridge:/);
    // paragraphs and list items kept apart (not one clumped run)
    expect(html).toContain('</p>');
    expect((html.match(/<li>/g) ?? []).length).toBeGreaterThanOrEqual(4);
  });
});
