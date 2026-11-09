/** Shared UI arbitration; persistence remains in each updater's existing store. */
import { createSignal } from 'solid-js';

const [extensionOpen, setExtensionOpen] = createSignal(false);
const [sessionClosed, setSessionClosed] = createSignal<string | undefined>();
export { extensionOpen };
export function setExtensionUpdateOpen(open: boolean): void { setExtensionOpen(open); }
export function closeUpdateForSession(version?: string): void { if (version) setSessionClosed(version); }
export function sessionUpdateClosed(version?: string): boolean { return !!version && sessionClosed() === version; }
export function resetUpdateSurface(): void { setExtensionOpen(false); setSessionClosed(undefined); }
