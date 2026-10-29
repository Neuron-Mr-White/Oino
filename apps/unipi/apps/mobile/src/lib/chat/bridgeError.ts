/**
 * UNI-308: the bridge answers an op it doesn't know with the wire-level
 * message `unknown type <t>` (see unipi `packages/app-bridge/src/wire.ts`
 * `parseIn`'s `bad` branch). That happens whenever the running pi is older
 * than the app — it was started before the feature the app just asked for
 * existed. A raw `"unknown type session_new"` toast means nothing to a user;
 * translate it into something they can act on (restart the pi session).
 *
 * One shared helper so every call site (`runOp`/`request` in `store.ts`,
 * and anywhere else that turns a bridge `error` into user-facing text) uses
 * the same wording, instead of each guessing at its own.
 */
// `unknown type <op>` (older bridges send just `unknown type`).
const UNKNOWN_TYPE_RE = /^unknown type(\s|$)/i;

/** True when `message` is the bridge's own "I don't know this op" wire error. */
export function isUnknownTypeError(message: string | undefined | null): boolean {
  return typeof message === 'string' && UNKNOWN_TYPE_RE.test(message.trim());
}

/**
 * Map a bridge error message to what the user should see. Passes through
 * anything else unchanged (a real failure — bad args, pi busy, a thrown
 * handler — already has a message worth showing as-is).
 */
export function friendlyBridgeError(message: string | undefined | null): string {
  const text = String(message || 'Something went wrong');
  if (isUnknownTypeError(text)) {
    return 'This pi is older than the app — restart that pi session to use this.';
  }
  return text;
}
