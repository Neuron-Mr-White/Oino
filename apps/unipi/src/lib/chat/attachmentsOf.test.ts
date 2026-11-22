import { describe, it, expect } from 'vitest';
import { attachmentsOf } from './model';

describe('attachmentsOf', () => {
  it('extracts well-formed entries from details.attachments[]', () => {
    const details = {
      attachments: [
        { path: '/tmp/shot.png', mime: 'image/png', size: 1234, caption: 'the bug', mediaRef: 'm1-1' },
        { path: '/tmp/report.pdf', mime: 'application/pdf' },
      ],
    };
    expect(attachmentsOf(details)).toEqual([
      { path: '/tmp/shot.png', mime: 'image/png', size: 1234, caption: 'the bug', mediaRef: 'm1-1' },
      { path: '/tmp/report.pdf', mime: 'application/pdf', size: undefined, caption: undefined, mediaRef: undefined },
    ]);
  });

  it('defaults a missing mime and drops entries with no path', () => {
    const details = { attachments: [{ path: '/a.bin' }, { mime: 'image/png' }, null, 'nope'] };
    expect(attachmentsOf(details)).toEqual([{ path: '/a.bin', mime: 'application/octet-stream', size: undefined, caption: undefined, mediaRef: undefined }]);
  });

  it('is empty for missing/non-array/malformed details', () => {
    expect(attachmentsOf(undefined)).toEqual([]);
    expect(attachmentsOf(null)).toEqual([]);
    expect(attachmentsOf({})).toEqual([]);
    expect(attachmentsOf({ attachments: 'nope' })).toEqual([]);
  });
});
