/**
 * Which catalogue models the picker offers (F-02): chat models of providers
 * that work. Non-chat models (image / speech / embedding…) are never listed;
 * models of a provider whose key is rejected or missing are set apart with the
 * reason instead of mixed in. The host's per-model `kind` / readiness wins
 * when it sends them; otherwise the provider rows (`auth_providers`) decide.
 */
import type { ModelInfo } from './picker';
import { providerUsable, type Readiness } from './readiness';

const NON_CHAT_ID = /(^|[\s/_.:-])(image|img|imagen|dall-?e|flux|speech|tts|stt|whisper|audio|transcri\w*|embed\w*|rerank\w*|moderation|video|sora|veo|music|lyria|vision-?gen)([\s/_.:-]|\d|$)/i;

/** Whether the model chats (text in, text out). `kind` from the host wins; ids are the fallback. */
export function isChatModel(m: ModelInfo): boolean {
  if (m.kind) return m.kind === 'chat' || m.kind === 'text' || m.kind === 'llm';
  if (m.output && m.output.length && !m.output.includes('text')) return false;
  return !NON_CHAT_ID.test(`${m.id} ${m.name ?? ''}`);
}

export type Unavailable = { reason: string };

/** `undefined` = usable; otherwise why not (calm, human). */
export function unavailableReason(m: ModelInfo, readiness: Readiness | undefined): string | undefined {
  if (m.ready === false) return m.reason || 'Not available';
  if (m.ready === true || !readiness?.known) return undefined;
  const p = readiness.providers.find((x) => x.id === m.provider);
  if (!p) return undefined;
  if (providerUsable(p)) return undefined;
  if (p.configured || p.ready === false) return p.reason ? `${p.name || p.id}: ${p.reason}` : `${p.name || p.id}: the key was rejected`;
  return `${p.name || p.id} isn’t set up`;
}

/** Chat models split into those that can run and those that can’t (with the reason). */
export function splitAvailability(
  all: readonly ModelInfo[],
  readiness: Readiness | undefined,
  keepKey?: string,
): { usable: ModelInfo[]; unavailable: Array<{ model: ModelInfo; reason: string }> } {
  const usable: ModelInfo[] = [];
  const unavailable: Array<{ model: ModelInfo; reason: string }> = [];
  for (const m of all) {
    const key = `${m.provider}/${m.id}`;
    if (!isChatModel(m) && key !== keepKey) continue;
    const reason = unavailableReason(m, readiness);
    if (reason && key !== keepKey) unavailable.push({ model: m, reason });
    else usable.push(m);
  }
  return { usable, unavailable };
}

/** Pick-time failure → one calm sentence (never a provider's JSON). */
export function calmModelError(error: unknown, name?: string): string {
  const raw = String((error as { message?: unknown })?.message ?? error ?? '');
  const who = name ? `${name} ` : 'This model ';
  if (/\b(401|403)\b|authentication|unauthori[sz]ed|invalid.{0,12}key|rejected/i.test(raw)) return `${who}can’t be used: its provider rejected the key. Fix it in Settings › Providers or pick another model.`;
  if (/\b429\b|rate.?limit|quota/i.test(raw)) return `${who}is rate-limited right now. Try again in a moment or pick another model.`;
  if (/\b404\b|not found|unknown model/i.test(raw)) return `${who}isn’t available from its provider. Pick another model.`;
  if (/^\s*[{[]/.test(raw) || raw.length > 200) return `${who}couldn’t be switched to. Pick another model.`;
  return raw || `${who}couldn’t be switched to.`;
}
