/**
 * `empty` (WP-A1): one paired PC, reachable, with no sessions yet — the
 * first-run empty state of the session list.
 */
import type { MockScenario } from '../host';
import { genFsTree } from '../gen';

export const empty: MockScenario = {
  name: 'empty',
  appOps: {
    app_info: () => ({ version: '1.0.2', platform: 'linux', proto: '1' }),
    about_components: () => ({ app: '1.0.2', host: { version: '1.0.1' }, oino: { version: '1.0.2' }, extensions: { count: 0 }, problems: [] }),
  },
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: [], fs: genFsTree() }],
};
