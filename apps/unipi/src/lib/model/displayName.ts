/**
 * A model's display name (composer chip, picker rows, toasts): the catalogue's
 * `name` when it is a real name, else one derived from the id — routers and
 * local catalogues often send the id itself as the name
 * (`ollamacloud/deepseek-v4.1-flash`), which reads as a path, not a model.
 *
 *   deepseek-v4.1-flash          → DeepSeek V4.1 Flash
 *   ollamacloud/glm-4.7          → GLM 4.7
 *   anthropic/claude-opus-4-5    → Claude Opus 4.5
 *   gpt-5.1-codex-mini           → GPT 5.1 Codex Mini
 *   qwen3-coder:30b              → Qwen3 Coder 30B
 */

/** Words with a fixed brand spelling (lowercase key). */
const BRANDS: Record<string, string> = {
  deepseek: 'DeepSeek',
  gpt: 'GPT',
  glm: 'GLM',
  claude: 'Claude',
  gemini: 'Gemini',
  gemma: 'Gemma',
  llama: 'Llama',
  qwen: 'Qwen',
  kimi: 'Kimi',
  minimax: 'MiniMax',
  mistral: 'Mistral',
  mixtral: 'Mixtral',
  codestral: 'Codestral',
  grok: 'Grok',
  phi: 'Phi',
  sonnet: 'Sonnet',
  opus: 'Opus',
  haiku: 'Haiku',
  oss: 'OSS',
  vl: 'VL',
  moe: 'MoE',
  it: 'IT',
  r1: 'R1',
  k2: 'K2',
};

const isVersionish = (w: string) => /^\d+(\.\d+)*$/.test(w);

function word(w: string): string {
  const lower = w.toLowerCase();
  if (BRANDS[lower]) return BRANDS[lower]!;
  // v4.1 → V4.1; 30b → 30B; 128k → 128K
  if (/^v\d/.test(lower)) return `V${w.slice(1)}`;
  if (/^\d+(\.\d+)?[bkm]$/.test(lower)) return w.toUpperCase();
  // qwen3 → Qwen3, o3 stays o3 (OpenAI's lowercase series)
  if (/^o\d/.test(lower)) return lower;
  const brand = /^([a-z]+)(\d[\w.]*)$/.exec(lower);
  if (brand && BRANDS[brand[1]!]) return `${BRANDS[brand[1]!]}${brand[2]}`;
  if (isVersionish(w)) return w;
  return w.charAt(0).toUpperCase() + w.slice(1);
}

/** The id's last path segment as words (`claude-opus-4-5` joins `4 5` → `4.5`). */
export function prettyModelId(id: string): string {
  const leaf = id.split('/').filter(Boolean).pop() ?? id;
  const words = leaf
    .replace(/[:_]/g, '-')
    .split('-')
    .filter(Boolean);
  const out: string[] = [];
  for (const w of words) {
    const prev = out[out.length - 1];
    // Single-digit runs are a dotted version split by the id's dashes (4-5 → 4.5).
    if (prev && /^\d+$/.test(prev) && /^\d$/.test(w) && prev.length <= 2) {
      out[out.length - 1] = `${prev}.${w}`;
      continue;
    }
    out.push(w);
  }
  return out.map(word).join(' ');
}

/** A `name` that is really an id (has a path separator, or equals the id). */
const looksLikeId = (name: string, id?: string) => name === id || name.includes('/') || /^[a-z0-9.:_-]+$/.test(name);

/** `provider: Name` prefixes (OpenRouter style) dropped. */
const stripVendor = (name: string) => /^[^:]{1,24}:\s+(.+)$/.exec(name)?.[1] ?? name;

export function modelDisplayName(model: { id?: string; name?: string } | undefined): string {
  if (!model) return '';
  const name = typeof model.name === 'string' ? model.name.trim() : '';
  const id = typeof model.id === 'string' ? model.id.trim() : '';
  if (name && !looksLikeId(name, id)) return stripVendor(name);
  const source = id || name;
  return source ? prettyModelId(source) : '';
}

/** Where the model comes from, for menus and tooltips: `oino · ollamacloud/deepseek-v4.1-flash`. */
export function modelOrigin(model: { provider?: string; id?: string } | undefined): string {
  if (!model) return '';
  return [model.provider, model.id].filter(Boolean).join(' · ');
}
