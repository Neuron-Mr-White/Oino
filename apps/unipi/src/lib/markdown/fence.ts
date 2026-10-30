/**
 * Fence-closed detection (UNI-156): marked treats an unterminated fence at
 * the end of the source as a complete code token too (so streaming text
 * doesn't throw), but we need to tell the two apart — a still-open fence
 * (the last block of a live stream) keeps its code/diagram plain until the
 * matching close line arrives.
 */

/** True when `raw` (a code token's `.raw`) ends with a matching closing fence line. */
export function isFenceClosed(raw: string): boolean {
  const lines = raw.split('\n');
  const openMatch = /^\s{0,3}(```+|~~~+)/.exec(lines[0] ?? '');
  if (!openMatch) return true; // not a fence (indented code block) — always "closed"
  const marker = openMatch[1]!;
  const ch = marker[0]!;
  for (let i = lines.length - 1; i >= 1; i--) {
    const trimmed = lines[i]!.trim();
    if (!trimmed) continue;
    const closeMatch = new RegExp(`^(\\${ch === '`' ? '`' : '~'}{${marker.length},})$`).exec(trimmed);
    return !!closeMatch && closeMatch[1]![0] === ch;
  }
  return false;
}
