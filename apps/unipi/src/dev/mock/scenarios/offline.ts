/**
 * `offline` (WP-A1, Z-90): one paired PC whose HostClient connection is
 * offline — the shared OfflineState with the host's hint and Retry.
 */
import type { MockScenario } from '../host';

export const offline: MockScenario = {
  name: 'offline',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'offline', message: 'No route to coffee', hint: 'Is unipi-host running on the PC? It may be asleep or off the network.' },
    },
  ],
};
