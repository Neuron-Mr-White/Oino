/**
 * UNI-218 QA2 shared components: PageHeader, EmptyState / NoHostState, PcChip,
 * and the ui.css tokens every screen relies on.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { PageHeader } from './PageHeader';
import { EmptyState, NoHostState } from './EmptyState';
import { PcChip } from './PcChip';
import { resetLayout } from '../lib/layout';
import { setPlatformOverride } from '../lib/platform';
import { lore } from '../lore';

function desktop(on: boolean) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: on ? 1440 : 412 });
  window.matchMedia = ((query: string) => ({ matches: on, media: query, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia;
  resetLayout();
}

afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  delete (window as { matchMedia?: unknown }).matchMedia;
  resetLayout();
});

describe('<PageHeader>', () => {
  it('renders lead, title, tools and meta in their slots', () => {
    render(() => (
      <PageHeader title="Chat" lead={<button>pc</button>} tools={<button aria-label="Refresh" />} meta="3 pi sessions" />
    ));
    const head = screen.getByTestId('page-head');
    expect(head).toHaveAttribute('data-lead');
    expect(head.querySelector('.page-head-lead')).toHaveTextContent('pc');
    expect(head.querySelector('.page-head-tools [aria-label=Refresh]')).toBeTruthy();
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Chat');
    expect(head.querySelector('.page-head-title .large-title')).toBeTruthy();
    expect(head.querySelector('.page-head-meta')).toHaveTextContent('3 pi sessions');
  });

  it('no lead: no data-lead (phone puts tools on the title row), no meta line', () => {
    render(() => <PageHeader title="Sessions" />);
    const head = screen.getByTestId('page-head');
    expect(head).not.toHaveAttribute('data-lead');
    expect(head.querySelector('.page-head-meta')).toBeNull();
  });

  it('titleSlot replaces the plain h1 (the board project picker)', () => {
    render(() => <PageHeader title="Board" titleSlot={<button>QA2 Main project</button>} />);
    expect(screen.queryByRole('heading', { level: 1 })).toBeNull();
    expect(screen.getByText('QA2 Main project').closest('.page-head-title')).toBeTruthy();
  });

  it('desktop shell: the needs-you bell sits in the header tools (F9); phone: not (it floats)', () => {
    setPlatformOverride('tauri-desktop');
    desktop(true);
    render(() => <PageHeader title="Files" />);
    expect(screen.getByTestId('page-head').querySelector('.page-head-tools [aria-label="Needs you"]')).toBeTruthy();
    cleanup();
    desktop(false);
    render(() => <PageHeader title="Files" />);
    expect(screen.queryByLabelText('Needs you')).toBeNull();
  });

  it('bell={false} keeps it out even on the desktop', () => {
    setPlatformOverride('tauri-desktop');
    desktop(true);
    render(() => <PageHeader title="Files" bell={false} />);
    expect(screen.queryByLabelText('Needs you')).toBeNull();
  });
});

describe('<EmptyState>', () => {
  it('crab by default, no lore line unless the screen passes one (F11)', () => {
    render(() => <EmptyState title="Nothing here" />);
    const el = screen.getByTestId('empty-state');
    expect(el.querySelector('img.crab')).toBeTruthy();
    expect(el).toHaveTextContent('Nothing here');
    for (const line of lore) expect(el.textContent).not.toContain(line);
    expect(el).toHaveAttribute('role', 'status');
  });

  it('icon, body, actions, compact and error tone', () => {
    render(() => (
      <EmptyState compact tone="error" icon={<svg data-testid="ic" />} title="Couldn't load" body="Host offline" action={<button>Retry</button>} />
    ));
    const el = screen.getByTestId('empty-state');
    expect(el).toHaveAttribute('data-compact');
    expect(el).toHaveAttribute('data-tone', 'error');
    expect(el).toHaveAttribute('role', 'alert');
    expect(el.querySelector('img.crab')).toBeNull();
    expect(el.querySelector('.empty-state-icon [data-testid=ic]')).toBeTruthy();
    expect(el.querySelector('p')).toHaveTextContent('Host offline');
    expect(el.querySelector('.empty-state-actions')).toHaveTextContent('Retry');
  });

  it('NoHostState: one wording + "Add a PC" for every host screen (F12)', () => {
    const onAdd = vi.fn();
    render(() => <NoHostState what="its files" onAdd={onAdd} />);
    expect(screen.getByText('No PC paired yet')).toBeInTheDocument();
    expect(screen.getByText(/to see its files here/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Add a PC' }));
    expect(onAdd).toHaveBeenCalledOnce();
  });
});

describe('<PcChip>', () => {
  it('name, state dot, live LED, label and click', () => {
    const onClick = vi.fn();
    render(() => <PcChip name="coffee" state="ok" live="live" onClick={onClick} />);
    const chip = screen.getByLabelText('PC: coffee');
    expect(chip).toHaveClass('pc-chip');
    expect(chip.querySelector('.pc-dot')).toHaveAttribute('data-state', 'ok');
    expect(chip.querySelector('.pc-dot')).toHaveAttribute('data-live', 'live');
    fireEvent.click(chip);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it('no PC yet: "Choose a PC", checking dot', () => {
    render(() => <PcChip onClick={() => {}} />);
    expect(screen.getByText('Choose a PC')).toBeInTheDocument();
    expect(screen.getByLabelText('PC: none').querySelector('.pc-dot')).toHaveAttribute('data-state', 'checking');
  });
});

describe('ui.css tokens (DESIGN.md §1.6)', () => {
  const css = readFileSync(resolve(__dirname, '../ui.css'), 'utf8');
  const main = readFileSync(resolve(__dirname, '../main.tsx'), 'utf8');

  it('is imported after every other stylesheet in main.tsx, so it wins', () => {
    const imports = [...main.matchAll(/^import '([^']+\.css)';/gm)].map((m) => m[1]);
    expect(imports.at(-1)).toBe('./ui.css');
  });

  it('one gutter: 16 on the phone, 20 on the desktop (F2)', () => {
    expect(css).toMatch(/:root \{[^}]*--page-gutter: 16px/);
    expect(css).toMatch(/html\[data-shell=desktop\] \{ --page-gutter: 20px; \}/);
  });

  it('one section label: 12px / 500 / .1em / uppercase / text-dim (F3)', () => {
    const rule = /\.section-label, \.board-section-label[^{]*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    for (const part of ['font-size: 12px', 'font-weight: 500', 'letter-spacing: .1em', 'text-transform: uppercase', 'var(--color-text-dim)']) expect(rule).toContain(part);
  });

  it('primary buttons are shell red, never the error red (F6)', () => {
    const rule = /\.btn-primary, \.pair-primary[^{]*\{([^}]*)\}/.exec(css)?.[1] ?? '';
    expect(rule).toContain('var(--color-shell)');
    expect(css).not.toMatch(/primary[^{]*\{[^}]*--color-wm-red/);
  });

  it('desktop floating surfaces share --radius-float (F17) and scrollbars are styled (F22)', () => {
    expect(css).toMatch(/--radius-float: 16px/);
    expect(css).toMatch(/\.palette[^{]*\{ border-radius: var\(--radius-float\)/);
    expect(css).toMatch(/::-webkit-scrollbar-thumb \{[^}]*background-color/);
  });
});
