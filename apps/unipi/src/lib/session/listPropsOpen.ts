/**
 * [ux-sessions] "Open this host's Properties menu" requests: the Task list header's filter icon
 * (owner 2026-11-03: the Properties icon moved there from the removed host header) asks, the
 * host's group (which owns the menu, its cards and Archive all) answers.
 */
import { createSignal } from 'solid-js';

export type ListPropsRequest = { hostId: string; anchor: DOMRect; seq: number };
const [request, setRequest] = createSignal<ListPropsRequest | undefined>();
let seq = 0;

export const listPropsRequest = request;
export function openListProperties(hostId: string, anchor: DOMRect): void {
  seq += 1;
  setRequest({ hostId, anchor, seq });
}
export function clearListPropertiesRequest(): void {
  setRequest(undefined);
}
