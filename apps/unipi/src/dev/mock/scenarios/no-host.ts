/**
 * `no-host` (WP-A1): nothing paired — every host-dependent screen shows the
 * shared NoHostState ("No PC paired yet · Add a PC").
 */
import type { MockScenario } from '../host';

export const noHost: MockScenario = { name: 'no-host', hosts: [] };
