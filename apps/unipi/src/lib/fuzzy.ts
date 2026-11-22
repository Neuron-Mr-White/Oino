export type FuzzyMatch = { score: number; positions: number[] };

function isLetter(char: string): boolean {
  return /\p{L}/u.test(char);
}

function isDigit(char: string): boolean {
  return /\p{N}/u.test(char);
}

function boundary(target: string, index: number): boolean {
  if (index === 0) return true;
  const prev = target[index - 1]!;
  const current = target[index]!;
  return '/-_.: '.includes(prev) || (/[a-z]/.test(prev) && /[A-Z]/.test(current)) || (isLetter(prev) && isDigit(current)) || (isDigit(prev) && isLetter(current));
}

function termMatch(term: string, target: string): FuzzyMatch | null {
  const needle = term.toLocaleLowerCase();
  const folded = target.toLowerCase();
  let best: FuzzyMatch | null = null;
  for (let start = 0; start < folded.length; start += 1) {
    if (folded[start] !== needle[0]) continue;
    const positions = [start];
    let cursor = start + 1;
    let ok = true;
    for (let i = 1; i < needle.length; i += 1) {
      while (cursor < folded.length && folded[cursor] !== needle[i]) cursor += 1;
      if (cursor >= folded.length) { ok = false; break; }
      positions.push(cursor++);
    }
    if (!ok) continue;
    let score = 0;
    for (let i = 0; i < positions.length; i += 1) {
      const pos = positions[i]!;
      score += 16;
      if (i > 0) {
        const gap = pos - positions[i - 1]! - 1;
        if (gap === 0) score += 8;
        else score -= Math.min(12, gap);
      }
      if (boundary(target, pos)) score += 10;
      if (i === 0 && pos === 0) score += 6;
    }
    if (folded.includes(needle)) score += 20;
    score -= target.length / 8;
    if (!best || score > best.score) best = { score, positions };
  }
  return best;
}

/** fzf-style match of `query` against `target`; every whitespace-delimited term must match. */
export function fuzzyMatch(query: string, target: string): FuzzyMatch | null {
  const terms = query.trim().split(/\s+/).filter(Boolean);
  if (!terms.length) return { score: 0, positions: [] };
  let score = 0;
  const positions: number[] = [];
  for (const term of terms) {
    const match = termMatch(term, target);
    if (!match) return null;
    score += match.score;
    positions.push(...match.positions);
  }
  return { score, positions: [...new Set(positions)].sort((a, b) => a - b) };
}

/** Rank items by their best searchable field, preserving original order for ties. */
export function fuzzyRank<T>(query: string, items: readonly T[], texts: (item: T) => readonly string[]): Array<{ item: T; score: number; field: number; positions: number[] }> {
  if (!query.trim()) return items.map((item) => ({ item, score: 0, field: 0, positions: [] }));
  return items.map((item, index) => {
    let best: { score: number; field: number; positions: number[] } | undefined;
    texts(item).forEach((text, field) => {
      const match = fuzzyMatch(query, text);
      if (match) {
        const score = match.score - 2 * field;
        if (!best || score > best.score) best = { score, field, positions: match.positions };
      }
    });
    return best ? { item, index, ...best } : null;
  }).filter((row): row is { item: T; index: number; score: number; field: number; positions: number[] } => row !== null)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ index: _index, ...row }) => row);
}
