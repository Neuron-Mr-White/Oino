// @vitest-environment jsdom
/**
 * Previewer "Edit as text" (UNI-191): the ⋯/header action on a file
 * that has a known host source opens the editor even without a normal
 * preview, read-only until "Edit anyway".
 */
import { render, screen, cleanup, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, describe, expect, it } from 'vitest';
import { Previewer } from './Previewer';
import { openPreviewer, closePreviewer } from '../../lib/media/previewerState';
import { getEditorTarget, clearEditorTarget } from '../../lib/editorTarget';

function mount() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <Previewer />} />
    </HashRouter>
  ));
}

afterEach(() => {
  cleanup();
  closePreviewer();
  clearEditorTarget();
  location.hash = '#/';
});

describe('Previewer "Edit as text"', () => {
  it('the no-preview "other" card opens the editor with asText for a known source', async () => {
    openPreviewer({ kind: 'other', name: 'data.bin', mime: 'application/octet-stream', source: { hostId: 'host-1', path: '/home/u/data.bin' } });
    mount();
    screen.getByText('Edit as text').click();
    expect(getEditorTarget()).toMatchObject({ hostId: 'host-1', path: '/home/u/data.bin', asText: true });
    await waitFor(() => expect(location.hash).toBe('#/files/edit'));
  });

  it('the header action also opens "Edit as text" for any kind with a source', async () => {
    openPreviewer({ kind: 'text', name: 'notes.log', mime: 'text/plain', text: 'hi', source: { hostId: 'host-2', path: '/home/u/notes.log' } });
    mount();
    screen.getByLabelText('Edit as text').click();
    expect(getEditorTarget()).toMatchObject({ hostId: 'host-2', path: '/home/u/notes.log', asText: true });
  });
});
