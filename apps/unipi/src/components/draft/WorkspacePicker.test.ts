/**
 * The workspace picker's recent list (owner 2026-11-03 / fix 2026-11-10):
 * scratch-cwd "No workspace" chats must not crowd real projects out of the
 * list, and the folder filter is fuzzy.
 */
import { describe, expect, it } from 'vitest';
import { folderChoices } from '../../lib/session/newSession';
import { fuzzyRank } from '../../lib/fuzzy';
import { recentWorkspaces } from './WorkspacePicker';

const scratch = (id: string) => ({ cwd: `/home/you/.local/share/oino/scratch/${id}` });

describe('recentWorkspaces', () => {
  it('finds the project sessions behind ten newer scratch chats', () => {
    // Newest first, as DraftView sorts by updated_at.
    const sessions = [
      ...Array.from({ length: 10 }, (_, i) => scratch(`s${i}`)),
      { cwd: '/home/you/p/oino' },
      { cwd: '/home/you/p/web' },
    ];
    const paths = folderChoices(sessions, [], undefined, Infinity);
    const items = recentWorkspaces(paths, '/home/you', (p) => p);
    expect(items.map((i) => i.path)).toEqual(['/home/you/p/oino', '/home/you/p/web']);
  });

  it('the default folderChoices cap stays at 8 for the other callers', () => {
    const sessions = Array.from({ length: 10 }, (_, i) => ({ cwd: `/p/${i}` }));
    expect(folderChoices(sessions, [])).toHaveLength(8);
  });
});

describe('picker filter (fuzzy)', () => {
  it('matches “unp” to unipi, recent label/path fields', () => {
    const items = [{ path: '/home/you/p/unipi', label: 'unipi' }];
    expect(fuzzyRank('unp', items, (it) => [it.label, it.path]).map((r) => r.item)).toEqual(items);
  });
});
