#!/usr/bin/env python3
"""UNI-200: build the Unicrab mascot rig from the pixel-art PNG.

    python3 apps/mobile/scripts/sprite-from-png.py \
        apps/mobile/src/assets/unicrab-pixel.png apps/mobile/src/assets/unicrab.sprite.json

No hand-drawn frames: this samples the 1254x1254 artwork back down to its
pixel grid and labels every cell with the body part it belongs to, so the
renderers (components/Unicrab.tsx on the web, UnicrabView.kt on Android)
can animate parts procedurally.

1. Grid. The art is upscaled pixel art whose cells are NOT all the same
   width (it was drawn on a loose ~22-cell grid). The cell boundaries are
   recovered from colour-edge density: every column/row where many pixels
   change colour is a boundary candidate; peaks are picked greedily with a
   minimum spacing, and any gap well over the median cell size is split
   evenly (two cells of the same colour side by side leave no edge).
2. Colour. Each cell takes the median colour of its inner area (borders
   trimmed so anti-aliasing doesn't leak in); transparent where the alpha
   majority is clear. Colours are snapped to a small palette (greedy
   clustering), so the JSON stays tiny and the result stays crisp.
3. Parts. Outline = pure black, visor = the near-black face plate, eyes =
   the pale yellow cells inside the visor (split left/right). The remaining
   coloured cells form islands separated by the outline (4-connected
   components): the island touching the visor is the body, islands whose
   centre is above the visor are claws, the ones below are legs (left/right
   by the body's centre column). Outline cells join the part of the
   nearest coloured cell (multi-source BFS), so a moved part carries its
   own outline.

Output: { width, height, palette: ["#rrggbb", ...], parts: { name: [[x, y, colourIndex], ...] },
          source: {...} }. Pure stdlib + Pillow + numpy.
"""
from __future__ import annotations

import json
import sys
from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image

PART_ORDER = ["legs-left", "legs-right", "body", "claw-left", "claw-right", "visor", "eye-left", "eye-right"]


def boundaries(profile: np.ndarray, lo: int, hi: int, min_gap: int) -> list[int]:
    smooth = np.convolve(profile.astype(float), np.ones(5) / 5, "same")
    chosen = [lo, hi]
    floor = smooth.max() * 0.08
    for i in np.argsort(-smooth):
        if smooth[i] < floor:
            break
        if i <= lo + min_gap // 2 or i >= hi - min_gap // 2:
            continue
        if all(abs(int(i) - j) >= min_gap for j in chosen):
            chosen.append(int(i))
    chosen.sort()
    gaps = np.diff(chosen)
    median = float(np.median(gaps))
    out = [chosen[0]]
    for a, b in zip(chosen, chosen[1:]):
        n = max(1, int(round((b - a) / median))) if (b - a) > 1.6 * median else 1
        for k in range(1, n):
            out.append(int(round(a + (b - a) * k / n)))
        out.append(b)
    return out


def cluster_palette(colours: list[tuple[int, int, int]], radius: int = 48) -> tuple[list[tuple[int, int, int]], list[int]]:
    """Greedy clustering: most common colour first, absorb everything within `radius` (L1)."""
    arr = np.array(colours)
    uniq, counts = np.unique(arr, axis=0, return_counts=True)
    order = np.argsort(-counts)
    centres: list[np.ndarray] = []
    members: list[list[int]] = []
    for idx in order:
        c = uniq[idx]
        for k, centre in enumerate(centres):
            if np.abs(centre - c).sum() <= radius:
                members[k].append(idx)
                break
        else:
            centres.append(c.copy())
            members.append([idx])
    # recompute each centre as the count-weighted mean
    palette = []
    for mem in members:
        w = counts[mem]
        palette.append(tuple(int(round(v)) for v in (uniq[mem] * w[:, None]).sum(0) / w.sum()))
    pal = np.array(palette)
    labels = [int(np.abs(pal - np.array(c)).sum(1).argmin()) for c in colours]
    return palette, labels


def build(png: Path) -> dict:
    im = np.array(Image.open(png).convert("RGBA")).astype(int)
    alpha = im[:, :, 3] > 128
    rgb = im[:, :, :3].copy()
    rgb[~alpha] = 255
    ys, xs = np.where(alpha)
    x0, x1, y0, y1 = int(xs.min()), int(xs.max()) + 1, int(ys.min()), int(ys.max()) + 1
    dx = (np.abs(np.diff(rgb, axis=1)).sum(2) > 60).sum(0)
    dy = (np.abs(np.diff(rgb, axis=0)).sum(2) > 60).sum(1)
    min_gap = max(8, int((x1 - x0) / 36))
    bx = boundaries(dx, x0, x1, min_gap)
    by = boundaries(dy, y0, y1, min_gap)
    w, h = len(bx) - 1, len(by) - 1

    cells: dict[tuple[int, int], tuple[int, int, int]] = {}
    for j in range(h):
        for i in range(w):
            ta, tb = by[j], by[j + 1]
            la, lb = bx[i], bx[i + 1]
            trim_y, trim_x = max(1, (tb - ta) // 6), max(1, (lb - la) // 6)
            a = alpha[ta + trim_y : tb - trim_y, la + trim_x : lb - trim_x]
            if a.size == 0 or a.mean() < 0.5:
                continue
            p = rgb[ta + trim_y : tb - trim_y, la + trim_x : lb - trim_x][a]
            cells[(i, j)] = tuple(int(v) for v in np.median(p, 0))

    keys = list(cells)
    palette, labels = cluster_palette([cells[k] for k in keys])
    index = {k: labels[n] for n, k in enumerate(keys)}

    lum = [0.299 * r + 0.587 * g + 0.114 * b for r, g, b in palette]
    outline = int(np.argmin(lum))
    # visor: the darkest NON-outline palette entry with low saturation
    dark = [k for k, (r, g, b) in enumerate(palette) if k != outline and max(r, g, b) < 70]
    visor_ids = set(dark)
    eye_ids = {k for k, (r, g, b) in enumerate(palette) if b > 120 and r > 200 and g > 200}

    part: dict[tuple[int, int], str] = {}
    visor_cells = [k for k in keys if index[k] in visor_ids]
    if not visor_cells:
        raise SystemExit("no visor cells found — palette: %r" % (palette,))
    vx0 = min(x for x, _ in visor_cells); vx1 = max(x for x, _ in visor_cells)
    vy0 = min(y for _, y in visor_cells); vy1 = max(y for _, y in visor_cells)
    vcx = (vx0 + vx1) / 2
    for k in visor_cells:
        part[k] = "visor"
    for k in keys:
        x, y = k
        if index[k] in eye_ids and vx0 <= x <= vx1 and vy0 <= y <= vy1:
            part[k] = "eye-left" if x < vcx else "eye-right"

    # islands of coloured (non-outline, unlabelled) cells
    coloured = {k for k in keys if k not in part and index[k] != outline}
    seen: set[tuple[int, int]] = set()
    islands: list[list[tuple[int, int]]] = []
    for start in sorted(coloured):
        if start in seen:
            continue
        comp, q = [], deque([start])
        seen.add(start)
        while q:
            c = q.popleft(); comp.append(c)
            for d in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                n = (c[0] + d[0], c[1] + d[1])
                if n in coloured and n not in seen:
                    seen.add(n); q.append(n)
        islands.append(comp)

    def touches_visor(comp):
        return any((x + dx_, y + dy_) in part and part[(x + dx_, y + dy_)] == "visor" for x, y in comp for dx_, dy_ in ((1, 0), (-1, 0), (0, 1), (0, -1)))

    body_islands = [c for c in islands if touches_visor(c)]
    body_cells = {k for c in body_islands for k in c}
    for comp in islands:
        cx = sum(x for x, _ in comp) / len(comp)
        cy = sum(y for _, y in comp) / len(comp)
        if comp in body_islands:
            name = "body"
        elif len(comp) <= 2 and any(abs(x - bx_) + abs(y - by_) <= 1 for x, y in comp for bx_, by_ in body_cells):
            name = "body"
        elif cy < vy0:
            name = "claw-left" if cx < vcx else "claw-right"
        else:
            name = "legs-left" if cx < vcx else "legs-right"
        for k in comp:
            part[k] = name

    # outline cells: nearest labelled cell wins (multi-source BFS over the grid)
    q = deque((k, part[k]) for k in keys if k in part)
    dist = {k: 0 for k in part}
    pending = {k for k in keys if k not in part}
    while q and pending:
        (x, y), name = q.popleft()
        for d in ((1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1), (1, -1), (-1, 1)):
            n = (x + d[0], y + d[1])
            if n in pending:
                pending.discard(n)
                part[n] = name if name not in ("eye-left", "eye-right") else "visor"
                q.append((n, part[n]))

    parts: dict[str, list[list[int]]] = {name: [] for name in PART_ORDER}
    for (x, y), name in sorted(part.items(), key=lambda kv: (kv[0][1], kv[0][0])):
        parts[name].append([x, y, index[(x, y)]])

    # The visor is solid under the eyes too, so any eye mask can be drawn on top of it.
    visor_fill = min(visor_ids)
    parts["visor"] += [[x, y, visor_fill] for name in ("eye-left", "eye-right") for x, y, _ in parts[name]]
    parts["visor"].sort(key=lambda c: (c[1], c[0]))
    eyes = eye_variants(parts, parts["eye-left"][0][2])

    return {
        "width": w,
        "height": h,
        "palette": ["#%02x%02x%02x" % c for c in palette],
        "roles": {"outline": outline, "visor": sorted(visor_ids), "eye": sorted(eye_ids)},
        "parts": parts,
        "eyes": eyes,
        "source": {"png": png.name, "size": [int(im.shape[1]), int(im.shape[0])], "crop": [x0, y0, x1, y1], "columns": bx, "rows": by},
    }


def eye_variants(parts: dict, colour: int) -> dict:
    """Eye masks the renderers swap between (procedural, from the labelled eye cells):
    open = as drawn; left/right = shifted one cell (scanning); blink/closed = a
    one-row line on the eye's bottom row, two cells wide, towards the middle;
    happy = a ^ (bottom-left, top-middle, bottom-right)."""
    out: dict[str, list[list[int]]] = {"open": [], "left": [], "right": [], "blink": [], "closed": [], "happy": []}
    for name, inward in (("eye-left", 1), ("eye-right", -1)):
        cells = parts[name]
        xs = sorted({x for x, _, _ in cells}); ys = sorted({y for _, y, _ in cells})
        x, top, bottom = xs[0] if inward == 1 else xs[-1], ys[0], ys[-1]
        out["open"] += [[cx, cy, c] for cx, cy, c in cells]
        out["left"] += [[cx - 1, cy, c] for cx, cy, c in cells]
        out["right"] += [[cx + 1, cy, c] for cx, cy, c in cells]
        line = [[x, bottom, colour], [x + inward, bottom, colour]]
        out["blink"] += line
        out["closed"] += line
        out["happy"] += [[x - 1, bottom, colour], [x, top, colour], [x + 1, bottom, colour]]
    return out


def ascii_map(sprite: dict) -> str:
    glyph = {"body": "B", "claw-left": "L", "claw-right": "R", "visor": "V", "eye-left": "e", "eye-right": "E", "legs-left": "l", "legs-right": "r"}
    grid = [["." for _ in range(sprite["width"])] for _ in range(sprite["height"])]
    for name, cells in sprite["parts"].items():
        for x, y, c in cells:
            g = glyph[name]
            grid[y][x] = g.lower() if c == sprite["roles"]["outline"] and g not in "eE" else g
    return "\n".join("".join(r) for r in grid)


if __name__ == "__main__":
    src = Path(sys.argv[1] if len(sys.argv) > 1 else "apps/mobile/src/assets/unicrab-pixel.png")
    dst = Path(sys.argv[2] if len(sys.argv) > 2 else src.with_name("unicrab.sprite.json"))
    sprite = build(src)
    dst.write_text(json.dumps(sprite, separators=(",", ":")) + "\n")
    print(ascii_map(sprite))
    print(f"{sprite['width']}x{sprite['height']} cells, {len(sprite['palette'])} colours -> {dst}")
