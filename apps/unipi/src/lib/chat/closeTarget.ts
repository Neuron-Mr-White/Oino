/**
 * UNI-260: which chat the "Close this chat?" sheet is for (null = closed).
 * A signal of its own so the chat ⋯ menu and the desktop palette can open it.
 */
import { createSignal } from 'solid-js';
import type { ChatTarget } from './list';

const [target, setTarget] = createSignal<ChatTarget | null>(null);
export const closeChatTarget = target;
export const setCloseChatTarget = (t: ChatTarget | null) => setTarget(t);
