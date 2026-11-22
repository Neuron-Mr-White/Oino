/** `a >= b` for plain MAJOR.MINOR.PATCH strings (string compare breaks at 1.10). */
export function atLeast(a: string, b: string): boolean {
  const pa = a.split(/[.+-]/).slice(0, 3).map(Number);
  const pb = b.split(/[.+-]/).slice(0, 3).map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i]! > pb[i]!;
  return true;
}
