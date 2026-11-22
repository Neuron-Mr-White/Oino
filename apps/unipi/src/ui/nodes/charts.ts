/**
 * Chart helpers (ui-protocol §13.3): LTTB downsampling (Largest-Triangle-
 * Three-Buckets) to ≤ 2 × pixel width, and bucket max for sparklines.
 */

/** LTTB: keeps the visual shape of `points` in at most `threshold` points. */
export function lttb(points: ReadonlyArray<readonly [number, number]>, threshold: number): Array<[number, number]> {
  const n = points.length;
  if (threshold >= n || threshold < 3) return points.map((p) => [p[0], p[1]]);
  const out: Array<[number, number]> = [[points[0]![0], points[0]![1]]];
  const every = (n - 2) / (threshold - 2);
  let a = 0;
  for (let i = 0; i < threshold - 2; i++) {
    const avgStart = Math.floor((i + 1) * every) + 1;
    const avgEnd = Math.min(Math.floor((i + 2) * every) + 1, n);
    let ax = 0;
    let ay = 0;
    for (let j = avgStart; j < avgEnd; j++) {
      ax += points[j]![0];
      ay += points[j]![1];
    }
    const len = Math.max(1, avgEnd - avgStart);
    ax /= len;
    ay /= len;
    const start = Math.floor(i * every) + 1;
    const end = Math.floor((i + 1) * every) + 1;
    const [px, py] = points[a]!;
    let best = -1;
    let pick = start;
    for (let j = start; j < end; j++) {
      const area = Math.abs((px - ax) * (points[j]![1] - py) - (px - points[j]![0]) * (ay - py));
      if (area > best) {
        best = area;
        pick = j;
      }
    }
    out.push([points[pick]![0], points[pick]![1]]);
    a = pick;
  }
  out.push([points[n - 1]![0], points[n - 1]![1]]);
  return out;
}

/** Bucket max of a sparkline series down to `buckets` values (nulls stay absent). */
export function bucketMax(data: ReadonlyArray<number | null>, buckets: number): Array<number | null> {
  if (buckets <= 0 || data.length <= buckets) return [...data];
  const out: Array<number | null> = [];
  const size = data.length / buckets;
  for (let b = 0; b < buckets; b++) {
    let best: number | null = null;
    for (let i = Math.floor(b * size); i < Math.floor((b + 1) * size); i++) {
      const v = data[i];
      if (typeof v === 'number' && (best === null || v > best)) best = v;
    }
    out.push(best);
  }
  return out;
}
