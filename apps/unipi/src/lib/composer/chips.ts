/**
 * Inline chips (UNI-212): the composer draft stays a plain string, the wire
 * text pi gets; any token in it that parsed and will definitely resolve is
 * shown as an atomic chip instead of raw text:
 *
 * - attachments: `[file: <name>]@att:<id>` in the draft (an upload tracked
 *   in lib/chat/attachments.ts by id); at send time each becomes the
 *   UNI-203 wire mention `[file: <name>]@<host path>` in place
 *   (`resolveAttachmentTokens`). In a sent bubble the host-path form is
 *   what shows up, chipped the same way;
 * - images (UNI-285): `[image]@img:<id>` in the draft (a pending image by
 *   id), labelled "image 1", "image 2"… by its position in the pending
 *   list — the token carries no number, so renumbering after a remove needs
 *   no text rewrite. At send time each becomes the plain text
 *   `[image N]`, N matching the order the images are sent in
 *   (`resolveImageTokens`), so pi sees "compare [image 1] with [image 2]";
 *   a sent bubble chips `[image N]` back to that image's viewer;
 * - `@path` mentions verified to exist on the PC (picked from the @ menu,
 *   or confirmed through the bridge's file finder);
 * - a leading `/skill:<name>` or `/<command>` that's in the session's
 *   command list (pi only expands those at the very start of a prompt);
 *   the long-horizon mode commands (`/graph`, `/goal`, `/ralph`, `/swarm`,
 *   `/regular`) are command chips flagged `mode`;
 * - `$<skill>` anywhere in the text (owner spec 2026-11-03), for a skill in
 *   the session's list (`skill:<name>`). At send time `serializeSkills`
 *   turns the first one into pi's own `/skill:<name> …` invocation at the
 *   start of the prompt (the only form the engine expands); the others stay
 *   `$name` references the model resolves through the system prompt's
 *   `<available_skills>` list;
 * - in sent bubbles, pi's expanded `<skill name="…">…</skill>` block (what
 *   a `/skill:` prompt becomes in the transcript) collapses to its chip.
 *
 * Unresolved text stays plain. Pure (no DOM, no Solid) so the editor, the
 * bubble renderer and the tests share one tokenizer.
 */

export type Chip =
  | { kind: 'file'; raw: string; name: string; attId?: string; path?: string }
  | { kind: 'image'; raw: string; imgId?: string; n?: number }
  | { kind: 'path'; raw: string; path: string }
  /** `form: 'dollar'`: a `$name` reference anywhere in the text (else the leading `/skill:name` or pi's block). */
  | { kind: 'skill'; raw: string; name: string; form?: 'dollar' }
  /** `mode`: a long-horizon mode switch (`MODE_COMMANDS`). */
  | { kind: 'command'; raw: string; name: string; mode?: true };

/** The long-horizon mode commands (owner spec 2026-11-03): a chip of its own look; sent as the
 *  command itself, which switches the chat's mode. */
export const MODE_COMMANDS: ReadonlySet<string> = new Set(
  ['graph', 'goal', 'ralph', 'swarm', 'regular'].flatMap((m) => [m, `unipi:${m}`]),
);

export type Seg = { text: string } | { chip: Chip };

/** Draft-side attachment reference: `@att:<id>` instead of a host path. */
export const ATT_PREFIX = 'att:';

/** Draft-side image reference (UNI-285): `[image]@img:<id>` instead of a number. */
export const IMG_PREFIX = 'img:';

export function attachmentToken(name: string, id: string): string {
  // `]` would end the bracket early; a name can't break the token.
  return `[file: ${name.replace(/[\]\n]/g, '_')}]@${ATT_PREFIX}${id}`;
}

/** The draft token for a pending image. Its number is never part of the
 *  token: the chip's label comes from the token's position in the draft, so
 *  renumbering after a remove needs no text rewrite. */
export function imageToken(id: string): string {
  return `[image]@${IMG_PREFIX}${id}`;
}

/** The ids of the draft's `[image]@img:<id>` chips, in the order they sit in
 *  the text (= attach order). A chip's number is its 1-based index here. */
export function imageChipIds(text: string): string[] {
  return [...text.matchAll(IMAGE_RE)].map((m) => m[1]!);
}

const FILE_RE = /\[file: ([^\]\n]*)\]@(\S+)/g;
const IMAGE_RE = /\[image]@img:(\S+)/g;
/** A sent message's `[image N]` reference (N = its position among that
 *  message's images). */
const IMAGE_LABEL_RE = /\[image (\d+)]/g;
const SKILL_BLOCK_RE = /^<skill name="([^"]+)" location="[^"]*">[\s\S]*?<\/skill>(?:\n\n)?/;
/** A `$skill` reference: after the start, whitespace or an opening bracket; pi skill names are
 *  letters, digits, `-`, `_` and inner dots. */
const SKILL_REF_RE = /(^|[\s(\[{])\$([A-Za-z0-9][\w-]*(?:\.[\w-]+)*)/g;

export type TokenizeOpts = {
  /** Names of the session's commands (without `/`), e.g. `skill:review`, `unipi:goal`.
   *  Its `skill:<name>` entries are also the skills a `$name` reference may name. */
  commands?: ReadonlySet<string>;
  /** Whether an `@path` token (with its `@`) is known to exist on the PC. */
  verified?: (token: string) => boolean;
  /** Editor mode: a typed token only chips once it's finished (followed by
   * whitespace), so the chip never swallows the word being typed. */
  requireTrailingSpace?: boolean;
  /** Bubble mode: collapse pi's expanded `<skill …>` block. */
  skillBlocks?: boolean;
  /** Bubble mode (UNI-285): chip a sent message's `[image N]` back to that
   * image. */
  imageLabels?: boolean;
};

const ends = (text: string, end: number, opts: TokenizeOpts) => (end >= text.length ? !opts.requireTrailingSpace : /\s/.test(text[end]!));
/** A `$skill` ends at whitespace — in a sent bubble also at closing punctuation ("use $review."). */
const skillEnds = (text: string, end: number, opts: TokenizeOpts) =>
  ends(text, end, opts) || (!opts.requireTrailingSpace && end < text.length && /[,.;:!?)\]}'"]/.test(text[end]!));

/** Whether `name` is a skill of the session (`skill:<name>` in its command list). */
export const isSkill = (name: string, commands: ReadonlySet<string> | undefined): boolean => !!commands?.has(`skill:${name}`);

/** Splits `text` into plain runs and chips (see module doc). Adjacent plain
 * runs are merged; an empty string gives `[]`. */
export function tokenize(text: string, opts: TokenizeOpts = {}): Seg[] {
  const hits: Array<{ start: number; end: number; chip: Chip }> = [];
  for (const m of text.matchAll(FILE_RE)) {
    const path = m[2]!;
    const start = m.index!;
    hits.push({
      start,
      end: start + m[0].length,
      chip: path.startsWith(ATT_PREFIX) ? { kind: 'file', raw: m[0], name: m[1]!, attId: path.slice(ATT_PREFIX.length) } : { kind: 'file', raw: m[0], name: m[1]!, path },
    });
  }
  const free = (s: number, e: number) => hits.every((h) => e <= h.start || s >= h.end);

  // UNI-285: a pending image (`[image]@img:<id>`) in the composer draft.
  for (const m of text.matchAll(IMAGE_RE)) {
    const start = m.index!;
    if (!free(start, start + m[0].length)) continue;
    hits.push({ start, end: start + m[0].length, chip: { kind: 'image', raw: m[0], imgId: m[1] } });
  }
  // A sent message's `[image N]` (bubble only — the composer's own token has no number).
  if (opts.imageLabels) {
    for (const m of text.matchAll(IMAGE_LABEL_RE)) {
      const start = m.index!;
      if (!free(start, start + m[0].length)) continue;
      hits.push({ start, end: start + m[0].length, chip: { kind: 'image', raw: m[0], n: Number(m[1]) } });
    }
  }

  // A leading command / skill (pi only expands these at the very start).
  const block = opts.skillBlocks ? SKILL_BLOCK_RE.exec(text) : null;
  if (block) {
    hits.push({ start: 0, end: block[0].length, chip: { kind: 'skill', raw: block[0], name: block[1]! } });
  } else {
    const cmd = /^\/(\S+)/.exec(text);
    if (cmd && opts.commands?.has(cmd[1]!) && ends(text, cmd[0].length, opts) && free(0, cmd[0].length)) {
      const name = cmd[1]!;
      hits.push({
        start: 0,
        end: cmd[0].length,
        chip: name.startsWith('skill:') ? { kind: 'skill', raw: cmd[0], name: name.slice(6) } : { kind: 'command', raw: cmd[0], name, ...(MODE_COMMANDS.has(name) ? { mode: true as const } : {}) },
      });
    }
  }

  // `$skill` references, anywhere (owner spec 2026-11-03).
  if (opts.commands?.size) {
    for (const m of text.matchAll(SKILL_REF_RE)) {
      const start = m.index! + m[1]!.length;
      const end = start + 1 + m[2]!.length;
      if (!isSkill(m[2]!, opts.commands) || !free(start, end) || !skillEnds(text, end, opts)) continue;
      hits.push({ start, end, chip: { kind: 'skill', raw: text.slice(start, end), name: m[2]!, form: 'dollar' } });
    }
  }

  if (opts.verified) {
    for (const m of text.matchAll(/(^|[\s(\[{])(@"[^"\n]+"|@[^\s"]+)/g)) {
      const token = m[2]!;
      const start = m.index! + m[1]!.length;
      const end = start + token.length;
      if (!free(start, end) || !ends(text, end, opts) || !opts.verified(token)) continue;
      hits.push({ start, end, chip: { kind: 'path', raw: token, path: token.replace(/^@"?|"$/g, '') } });
    }
  }

  hits.sort((a, b) => a.start - b.start);
  const out: Seg[] = [];
  let at = 0;
  const pushText = (t: string) => {
    if (!t) return;
    const last = out[out.length - 1];
    if (last && 'text' in last) last.text += t;
    else out.push({ text: t });
  };
  for (const h of hits) {
    if (h.start < at) continue;
    pushText(text.slice(at, h.start));
    out.push({ chip: h.chip });
    at = h.end;
  }
  pushText(text.slice(at));
  return out;
}

/** Every `@path`-looking token in `text` that's finished (followed by
 * whitespace) — the candidates the composer asks the bridge to verify. */
export function pathCandidates(text: string): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(/(^|[\s(\[{])(@"[^"\n]+"|@[^\s"]+)(?=\s)/g)) {
    const token = m[2]!;
    if (token.length > 1 && !token.startsWith(`@${ATT_PREFIX}`) && !token.startsWith(`@${IMG_PREFIX}`)) out.add(token);
  }
  return [...out];
}

/** Inserts `insert` at `caret`, padding it with a space on either side when
 * it would otherwise glue onto a word (a chip must stay its own token).
 * Returns the new text and the caret right after the insert's padding. */
export function insertToken(text: string, caret: number, insert: string): { text: string; caret: number } {
  const at = Math.max(0, Math.min(caret, text.length));
  const before = text.slice(0, at);
  const after = text.slice(at);
  const lead = before && !/\s$/.test(before) ? ' ' : '';
  const trail = after && /^\s/.test(after) ? '' : ' ';
  const next = `${before}${lead}${insert}${trail}${after}`;
  return { text: next, caret: before.length + lead.length + insert.length + trail.length };
}

/** UNI-290: a sent message's text going back into the draft with its images
 * re-attached under `ids` (image N = `ids[N-1]`): each `[image N]` becomes
 * that image's `[image]@img:<id>` chip (first mention only, so an image is
 * never sent twice); images the text never mentioned get a chip at the end. */
export function restoredImageText(text: string, ids: readonly string[]): string {
  const used = new Set<number>();
  let out = text.replace(IMAGE_LABEL_RE, (raw, n: string) => {
    const i = Number(n) - 1;
    if (i < 0 || i >= ids.length || used.has(i)) return raw;
    used.add(i);
    return imageToken(ids[i]!);
  });
  ids.forEach((id, i) => {
    if (used.has(i)) return;
    out = insertToken(out, out.length, imageToken(id)).text;
  });
  return out;
}

/** Removes the chip token `raw` at `start` along with one space next to it
 * (so deleting a chip doesn't leave a double space). */
export function removeToken(text: string, start: number, raw: string): { text: string; caret: number } {
  if (text.slice(start, start + raw.length) !== raw) return { text, caret: start };
  let s = start;
  let e = start + raw.length;
  if (text[e] === ' ') e++;
  else if (s > 0 && text[s - 1] === ' ') s--;
  return { text: text.slice(0, s) + text.slice(e), caret: s };
}

/** Where each chip token sits in the text (for removal by index). */
export function chipSpans(segs: readonly Seg[]): Array<{ chip: Chip; start: number }> {
  const out: Array<{ chip: Chip; start: number }> = [];
  let at = 0;
  for (const s of segs) {
    if ('chip' in s) {
      out.push({ chip: s.chip, start: at });
      at += s.chip.raw.length;
    } else at += s.text.length;
  }
  return out;
}

/** Send-time serialization (UNI-285): every `[image]@img:<id>` token becomes
 * the plain text `[image N]`, N being its 1-based position among the tokens
 * that do resolve — the very order the returned `ids` are in, so the caller
 * sends the images in that order and "[image 1]" really is the first image.
 * Tokens whose image is gone are dropped (one separating space with them). */
export function resolveImageTokens(
  text: string,
  images: readonly { id: string }[],
): { text: string; ids: string[] } {
  const ids: string[] = [];
  let out = '';
  let at = 0;
  for (const m of text.matchAll(IMAGE_RE)) {
    const id = m[1]!;
    const start = m.index!;
    out += text.slice(at, start);
    at = start + m[0].length;
    const seen = ids.indexOf(id);
    if (seen >= 0) out += `[image ${seen + 1}]`;
    // The same image twice (a chip copied and pasted) is sent once; both mentions name it.
    else if (images.some((image) => image.id === id)) {
      ids.push(id);
      out += `[image ${ids.length}]`;
    } else if (text[at] === ' ' && (out === '' || /\s$/.test(out))) at++;
  }
  out += text.slice(at);
  return { text: out, ids };
}

export type AttachmentState = { name: string; status: 'uploading' | 'done' | 'error'; path?: string };

/**
 * Send-time serialization (UNI-212): every `[file: name]@att:<id>` token
 * becomes the UNI-203 wire mention `[file: name]@<host path>` in place;
 * tokens whose upload isn't done (or is unknown) are dropped from the
 * outgoing text and returned in `kept` so the caller can leave them in the
 * draft for a retry/remove. Everything else is sent exactly as typed.
 */
export function resolveAttachmentTokens(text: string, files: (id: string) => AttachmentState | undefined): { text: string; kept: string[]; sentIds: string[] } {
  const kept: string[] = [];
  const sentIds: string[] = [];
  let out = '';
  let at = 0;
  for (const m of text.matchAll(FILE_RE)) {
    const path = m[2]!;
    if (!path.startsWith(ATT_PREFIX)) continue;
    const id = path.slice(ATT_PREFIX.length);
    const file = files(id);
    const start = m.index!;
    out += text.slice(at, start);
    at = start + m[0].length;
    if (file?.status === 'done' && file.path) {
      out += `[file: ${m[1]}]@${file.path}`;
      sentIds.push(id);
    } else {
      if (file) kept.push(m[0]);
      // Swallow one separating space so "a <chip> b" doesn't become "a  b".
      if (text[at] === ' ' && (out === '' || /\s$/.test(out))) at++;
    }
  }
  out += text.slice(at);
  return { text: out, kept, sentIds };
}

/** Draft text loaded back from storage (a reload): the pending attachments behind its
 *  `[image]@img:<id>` / `[file: …]@att:<id>` chips lived in memory and are gone, so their
 *  tokens go too (with one separating space each) instead of showing dead chips. */
export function stripAttachmentTokens(text: string): string {
  return text.replace(/(?:\[image]@img:\S+|\[file: [^\]\n]*\]@att:\S+) ?/g, '');
}

/** The ids of the draft's `[file: …]@att:<id>` chips, in text order. */
export function fileChipIds(text: string): string[] {
  return [...text.matchAll(FILE_RE)].map((m) => m[2]!).filter((p) => p.startsWith(ATT_PREFIX)).map((p) => p.slice(ATT_PREFIX.length));
}

/** The `$skill` references of `text` (session skills only), in order, with offsets. */
export function skillRefs(text: string, commands: ReadonlySet<string> | undefined): Array<{ name: string; start: number; end: number }> {
  const out: Array<{ name: string; start: number; end: number }> = [];
  if (!commands?.size) return out;
  for (const m of text.matchAll(SKILL_REF_RE)) {
    const start = m.index! + m[1]!.length;
    const end = start + 1 + m[2]!.length;
    if (isSkill(m[2]!, commands) && skillEnds(text, end, {})) out.push({ name: m[2]!, start, end });
  }
  return out;
}

/**
 * Send-time serialization of `$skill` chips (pi parity: the engine only expands a
 * `/skill:<name> <args>` prompt, into its `<skill …>` block followed by the args):
 *
 * - a message that already starts with a `/command` (or `/skill:`) is left alone — a command chip
 *   only sits at the start and its arguments are its own;
 * - otherwise the first `$name` becomes the leading `/skill:<name>`: a `$name` that opens the
 *   message is replaced by it, one further in stays where it is as a readable reference
 *   ("fix it with $review" → "/skill:review fix it with $review");
 * - later skills stay `$name` (the model reads them from `<available_skills>`).
 */
export function serializeSkills(text: string, commands: ReadonlySet<string> | undefined): string {
  if (/^\s*\//.test(text)) return text;
  const first = skillRefs(text, commands)[0];
  if (!first) return text;
  const lead = `/skill:${first.name}`;
  if (!text.slice(0, first.start).trim()) {
    const rest = text.slice(first.end).replace(/^[ \t]+/, '');
    return rest ? `${lead} ${rest.replace(/^\n+/, '')}` : lead;
  }
  return `${lead} ${text}`;
}
