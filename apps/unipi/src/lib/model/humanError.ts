/**
 * Shared provider / run error humanizer (G-05, G-07): turns raw HTTP + JSON
 * provider bodies (`401: {"message":"User not found.","code":401}`) into one
 * calm, actionable human line.
 */

const PROVIDER_NAMES: Record<string, string> = {
  openrouter: 'OpenRouter',
  anthropic: 'Anthropic',
  openai: 'OpenAI',
  google: 'Google',
  gemini: 'Google Gemini',
  deepseek: 'DeepSeek',
  xai: 'xAI',
  groq: 'Groq',
  mistral: 'Mistral',
  together: 'Together AI',
  fireworks: 'Fireworks',
  oino: 'Oino Router',
  omniroute: 'OmniRoute',
};

export function displayProviderName(provider: string | undefined): string | undefined {
  if (!provider) return undefined;
  const clean = provider.trim();
  if (!clean) return undefined;
  return PROVIDER_NAMES[clean.toLowerCase()] ?? clean[0]!.toUpperCase() + clean.slice(1);
}

function extractJsonMessage(raw: string): { code?: number; message?: string } {
  const idx = raw.indexOf('{');
  if (idx < 0) return {};
  try {
    const parsed = JSON.parse(raw.slice(idx)) as Record<string, unknown>;
    const errObj = parsed.error && typeof parsed.error === 'object' ? (parsed.error as Record<string, unknown>) : undefined;
    const msg = typeof errObj?.message === 'string' ? errObj.message : typeof parsed.message === 'string' ? parsed.message : undefined;
    const code = typeof errObj?.code === 'number' ? errObj.code : typeof parsed.code === 'number' ? parsed.code : undefined;
    return { ...(code !== undefined ? { code } : {}), ...(msg ? { message: msg.trim() } : {}) };
  } catch {
    return {};
  }
}

/**
 * Formats a provider / turn failure into a readable human line (never raw JSON).
 * `withSettingsHint` appends "— fix it in Settings › Providers" for interactive chat views.
 */
export function humanProviderError(raw: string | undefined, provider?: string, opts: { settingsHint?: boolean } = {}): string {
  const text = (raw ?? '').trim();
  if (!text) return 'The model request failed.';
  const who = displayProviderName(provider);
  const { code, message } = extractJsonMessage(text);
  const statusMatch = /^(?:HTTP\s+)?(\d{3})\b/i.exec(text) ?? /\bstatus(?:\s+code)?[:\s]+(\d{3})\b/i.exec(text);
  const status = code ?? (statusMatch ? Number(statusMatch[1]) : undefined);

  if (status === 401 || status === 403 || /\bunauthori[sz]ed\b|invalid.{0,16}api.?key|authentication.{0,12}failed/i.test(text)) {
    const base = `${who ?? 'The provider'} rejected the key (${status ?? 401})`;
    return opts.settingsHint ? `${base} — fix it in Settings › Providers.` : base;
  }
  if (status === 402 || /insufficient.{0,16}(?:credits|quota|balance)|billing/i.test(text)) {
    return `${who ?? 'The provider'} reported insufficient credits${status ? ` (${status})` : ''}.`;
  }
  if (status === 429 || /rate[- ]?limit|too many requests/i.test(text)) {
    return `${who ?? 'The provider'} rate-limited the request (429). Try again shortly.`;
  }
  if (status && status >= 500 && status < 600) {
    return `${who ?? 'The provider'} returned a server error (${status})${message ? ` — ${message}` : ''}.`;
  }
  if (message) {
    return `${who ? `${who}: ` : ''}${message}${status ? ` (${status})` : ''}`;
  }
  return text.replace(/\bHTTP\s+(\d{3})\s+\1\b/gi, 'HTTP $1');
}
