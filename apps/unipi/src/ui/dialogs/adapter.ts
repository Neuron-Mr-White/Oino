/**
 * App ↔ ui-protocol dialog glue (WP-A9; ui-protocol §3, §9.3): the registry
 * keeps dialogs as `UiRequest` (the pi `extension_ui_request` record without
 * its `type`), while the moved `DialogCard` speaks the app's `Dialog` shape,
 * and answers travel back as `extension_ui_response` on the session stream.
 *
 * WP-U10 widens the body set (`multiSelect`, `form`, `custom` nodes,
 * `clipboardReadImage`, `editExternally`); until then anything the card can't
 * render says so instead of pretending to answer.
 */
import type { UiRequest } from '../types';
import type { AskQuestion, Dialog } from '../../lib/chat/store';

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);

/** `select` options are strings, or `{label, description}` from Oino's `optionDetails`. */
function optionLabels(raw: unknown): string[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  return raw
    .map((o) => (typeof o === 'string' ? o : str((o as { label?: unknown })?.label)))
    .filter((l): l is string => !!l);
}

/** One `UiRequest` as the card's `Dialog` (numeric ids are the card's own business). */
export function piDialog(req: UiRequest): Dialog {
  const title = str(req.title);
  const method = req.method;
  switch (method) {
    case 'select':
      return { id: 0, kind: 'select', title, options: optionLabels(req.options) ?? [] };
    case 'confirm':
      return { id: 0, kind: 'confirm', title, message: str(req.message) };
    case 'input':
      return { id: 0, kind: 'input', title, placeholder: str(req.placeholder), prefill: str(req.prefill) };
    case 'editor':
      return { id: 0, kind: 'editor', title, prefill: str(req.prefill) };
    case 'ask_user':
    case 'multiSelect':
    case 'form':
      if (Array.isArray(req.questions)) return { id: 0, kind: 'ask_user', title, questions: req.questions as AskQuestion[] };
      return { id: 0, kind: 'custom', title };
    default:
      return { id: 0, kind: 'custom', title };
  }
}

/**
 * The `extension_ui_response` for one answer (pi forms): `undefined`/`null`
 * cancels, a boolean is `confirmed`, anything else is the `value`
 * (`{optionId, feedback?}` for approval, the option text for `select`).
 */
export function dialogResponse(id: string, value: unknown): { type: 'extension_ui_response'; id: string; [k: string]: unknown } {
  const base = { type: 'extension_ui_response' as const, id };
  if (value === undefined || value === null) return { ...base, cancelled: true };
  if (typeof value === 'boolean') return { ...base, confirmed: value };
  return { ...base, value };
}
