/**
 * The mock host (app spec §3.2/§3.15) for captures and tests. Captures set
 * `window.__UNIPI_HOST__ = createMockHost(scenario).client` (vite.config.capture.ts);
 * tests call `setHostClientFactory(createMockHost(scenario).client)`.
 * Production code never imports this module.
 */
export { createMockHost, MockHost, MockWorld } from '../../dev/mock/host';
export type { MockHostSpec, MockLogEntry, MockOp, MockScenario, MockSession, MockFsNode } from '../../dev/mock/host';
