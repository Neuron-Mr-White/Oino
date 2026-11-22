/**
 * What the Session modal is showing (WP-A17, D-7): a leaf module on purpose.
 *
 * The capture scenarios (`src/dev/mock/scenarios/modal-*.ts`) open the modal
 * from their `setup`, and the mock host is bootstrapped from `lib/host/factory`
 * — importing `SessionModal.tsx` there would close a module cycle
 * (factory → mock boot → scenarios → SessionModal → ui registries → session
 * store → factory) that breaks HMR in the capture harness. The signal lives
 * here so scenarios import only this file; `SessionModal.tsx` re-exports it.
 */
import { createSignal } from 'solid-js';

export type SessionModalTab = 'context' | 'running' | 'info';
/** Info sub-pages (G-13): the `/info <page>` pages. */
export type InfoPage = 'session' | 'usage' | 'tools' | 'skills' | 'modules';

export type SessionModalTarget = {
  hostId: string;
  sessionId: string;
  tab: SessionModalTab;
  infoPage?: InfoPage;
  /** The reserved registry entry that opened it (its `$close` is sent on dismiss). */
  entryId?: string;
};

const [target, setTarget] = createSignal<SessionModalTarget>();

/** What the modal shows (`undefined` = closed). */
export const sessionModal = target;

/** The setter (the modal component owns closing, which also drops the entry id). */
export const setSessionModalTarget = setTarget;

/** Opens the modal on a session's tab (the app-side entry points all funnel here). */
export function openSessionModal(t: Omit<SessionModalTarget, 'tab'> & { tab?: SessionModalTab }): void {
  setTarget({ tab: 'context', ...t });
}
