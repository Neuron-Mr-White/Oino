/**
 * Phone error copy (phone spec §3.4, PH-14 / PH-28): one table mapping what a
 * failed turn said to calm, actionable copy — a title, a one-line reason and,
 * for setup errors, the one fix the app offers ("Choose a model", "Add a
 * provider"). Never host file paths, `/login` or other CLI wording.
 */

export type ErrorFix = 'model' | 'provider';
export type ErrorCopy = { title: string; reason: string; fix?: ErrorFix };

type Rule = { test: RegExp; copy: ErrorCopy };

const RULES: Rule[] = [
  { test: /no model selected|no model (?:is )?(?:configured|available)|model not set/i, copy: { title: 'No model selected', reason: 'Choose a model for this chat, then retry.', fix: 'model' } },
  {
    test: /no (?:api key|credentials?)|not logged in|\/login|no provider|provider (?:is )?not configured|missing api key/i,
    copy: { title: 'No provider connected', reason: 'Add a model provider on this PC, then retry.', fix: 'provider' },
  },
  {
    test: /\b(401|403)\b|unauthori[sz]ed|invalid.{0,16}api.?key|rejected the key|authentication.{0,12}failed/i,
    copy: { title: 'The provider rejected the key', reason: 'Fix the key in Settings › Providers, or choose another model.', fix: 'provider' },
  },
  { test: /\b402\b|insufficient.{0,16}(?:credits|quota|balance)|billing/i, copy: { title: 'Out of credits', reason: 'The provider reported insufficient credits. Top up or choose another model.', fix: 'model' } },
  { test: /\b429\b|rate[- ]?limit|too many requests|quota/i, copy: { title: 'The provider is limiting requests', reason: 'Wait a moment and retry, or choose another model.' } },
  { test: /\b(404)\b|model.{0,24}not found|unknown model|no such model/i, copy: { title: 'This model isn’t available', reason: 'The provider has no such model. Choose another one.', fix: 'model' } },
  { test: /\b5\d\d\b|overloaded|server error|internal error/i, copy: { title: 'The provider had a problem', reason: 'Its servers returned an error. Retry in a moment.' } },
  { test: /network|fetch failed|ECONN|ENOTFOUND|EAI_AGAIN|timed? ?out|socket hang up/i, copy: { title: 'Couldn’t reach the provider', reason: 'Check the PC’s connection and retry.' } },
  { test: /context (?:length|window)|too many tokens|maximum context|prompt is too long/i, copy: { title: 'The chat is too long for this model', reason: 'Compact the chat or choose a model with a larger context.' } },
];

/** Strips what a phone user can't act on: host paths, CLI hints, "See: …" doc links. */
export function cleanReason(text: string): string {
  return text
    .split('\n')[0]!
    .replace(/\bSee:?\s+\S+/gi, '')
    .replace(/Use \/\w+[^.]*\.?/g, '')
    .replace(/(?:^|\s)(?:~|\/)[\w.\-/]+\/[\w.\-]+/g, '')
    .replace(/\s{2,}/g, ' ')
    .replace(/\s+([.,;:])/g, '$1')
    .trim();
}

export function errorCopy(raw: string): ErrorCopy {
  for (const rule of RULES) if (rule.test.test(raw)) return rule.copy;
  const reason = cleanReason(raw);
  return { title: 'The model request failed', reason: reason && reason.length <= 160 ? reason : reason ? `${reason.slice(0, 157).trimEnd()}…` : 'Retry, or choose another model.' };
}
