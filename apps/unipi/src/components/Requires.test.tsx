import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { Requires, UnavailableNotice } from './Requires';
import { setPlatformOverride } from '../lib/platform';

afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
});

describe('<Requires>', () => {
  it('renders children when the capability is there', () => {
    setPlatformOverride('tauri-mobile');
    render(() => <Requires cap="chat"><p>chat ui</p></Requires>);
    expect(screen.getByText('chat ui')).toBeInTheDocument();
    expect(screen.queryByTestId('unavailable-notice')).toBeNull();
  });

  it('web: shows the standard notice naming the feature', () => {
    setPlatformOverride('web');
    render(() => <Requires cap="chat"><p>chat ui</p></Requires>);
    expect(screen.queryByText('chat ui')).toBeNull();
    const notice = screen.getByTestId('unavailable-notice');
    expect(notice).toHaveAttribute('data-kind', 'needs-app');
    expect(notice).toHaveTextContent('Not available in the web version');
    expect(notice).toHaveTextContent('Chat');
  });

  it('web page notice links to the app site and back to the board', () => {
    setPlatformOverride('web');
    render(() => <Requires cap="terminal"><p>t</p></Requires>);
    const link = screen.getByTestId('get-app-link');
    expect(link).toHaveAttribute('href', 'https://oino.dev/download');
    expect(link).toHaveAttribute('target', '_blank');
    expect(screen.getByText('Open the board')).toHaveAttribute('href', '#/board');
  });

  it('inline notices and app-mode notices carry no app link', () => {
    setPlatformOverride('web');
    render(() => <Requires cap="terminal" variant="inline"><p>t</p></Requires>);
    expect(screen.queryByTestId('get-app-link')).toBeNull();
  });

  it('every listed cap must be present; the first missing one is explained', () => {
    setPlatformOverride('tauri-desktop');
    render(() => <Requires cap={['files', 'camera']}><p>x</p></Requires>);
    expect(screen.getByTestId('unavailable-notice')).toHaveAttribute('data-kind', 'phone-only');
  });

  it('fallback={null} hides; a custom fallback replaces the notice', () => {
    setPlatformOverride('web');
    render(() => (
      <>
        <Requires cap="haptics" fallback={null}><p>haptics row</p></Requires>
        <Requires cap="terminal" fallback={<p>custom</p>}><p>term</p></Requires>
      </>
    ));
    expect(screen.queryByText('haptics row')).toBeNull();
    expect(screen.getByText('custom')).toBeInTheDocument();
    expect(screen.queryByTestId('unavailable-notice')).toBeNull();
  });

  it('UnavailableNotice: inline variant, Desktop only wording', () => {
    setPlatformOverride('tauri-mobile');
    render(() => <UnavailableNotice cap="fileDragDrop" variant="inline" />);
    const notice = screen.getByTestId('unavailable-notice');
    expect(notice).toHaveAttribute('data-variant', 'inline');
    expect(notice).toHaveTextContent('Desktop only');
  });
});
