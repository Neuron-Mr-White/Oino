/**
 * `empty` (WP-A1): one paired PC, reachable, with no sessions yet — the
 * first-run empty state of the session list.
 */
import type { MockScenario } from '../host';
import { genFsTree } from '../gen';

export const empty: MockScenario = {
  name: 'empty',
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: [], fs: genFsTree() }],
};
