// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { readFileSync } from 'node:fs';
import { afterEach, expect, it } from 'vitest';
import { boardUi, resetBoardUi } from '../../lib/board/ui';
import { oin20Report, stressReport } from '../../dev/mock/report-fixtures';
import { ReportReader } from './Viewers';

afterEach(() => { cleanup(); resetBoardUi(); });

it('renders reports as shared Markdown in the reading column and keeps the header controls', async () => {
  boardUi().setReader({ title: 'OIN-20 · Report', text: oin20Report, actor: 'agent', at: new Date().toISOString() });
  render(() => <ReportReader />);
  const reader = screen.getByTestId('report-reader');
  const prose = reader.querySelector('.reader-body > .md.md-reader');
  expect(prose).not.toBeNull();
  await waitFor(() => expect(prose?.querySelector('code')?.textContent).toBe('ors'));
  expect(prose?.querySelector('p')?.textContent).toContain('Root cause');
  expect(screen.getByText('OIN-20 · Report')).toBeTruthy();
  expect(screen.getByText(/agent ·/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Close reader' }));
  expect(screen.queryByTestId('report-reader')).toBeNull();
});

it('renders fences, tables and plain paragraphs inside the width wrapper', async () => {
  boardUi().setReader({ title: 'Stress report', text: stressReport });
  render(() => <ReportReader />);
  const prose = screen.getByTestId('report-reader').querySelector('.reader-body > .md.md-reader')!;
  await waitFor(() => expect(prose.querySelector('table')).not.toBeNull());
  expect(prose.querySelector('h1')?.textContent).toBe('Layout stress test');
  expect(prose.querySelector('.cb pre code')?.textContent).toContain('unbroken-code-segment');
  expect(prose.querySelector('p code')?.textContent).toContain('unbroken-path-segment');
  boardUi().setReader({ title: 'Plain report', text: 'First paragraph.\n\nSecond paragraph.' });
  await waitFor(() => expect(screen.getByTestId('report-reader').querySelectorAll('.md-reader p')).toHaveLength(2));
});

it('constrains the reading wrapper and keeps overflow inside the reader', () => {
  const css = readFileSync('src/screens/board.css', 'utf8');
  expect(css).toMatch(/\.reader-body\s*\{[^}]*overflow-x:\s*hidden/);
  expect(css).toMatch(/\.reader-body\s*>\s*\.md-reader\s*\{[^}]*max-width:\s*760px;[^}]*margin:\s*0 auto;[^}]*overflow-wrap:\s*anywhere/);
});
