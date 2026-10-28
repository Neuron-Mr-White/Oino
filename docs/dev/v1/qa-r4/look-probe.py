#!/usr/bin/env python3
"""QA-r4 look probe: compare Oino screenshots against the A2.2 palette and the unipi
reference shots by sampling pixels at semantic regions (sidebar, window bg, cards, shell
accent). QA evidence for docs/dev/v1/qa-r4.md (not product code).

    python3 docs/dev/v1/qa-r4/look-probe.py shot.png [shot.png …]
"""

from __future__ import annotations

import sys
from collections import Counter
from pathlib import Path

from PIL import Image

# A2.2 tokens (design.md §A2.2), RGB
TOKENS = {
    "window 0A0908": (0x0A, 0x09, 0x08),
    "surface-1 141211": (0x14, 0x12, 0x11),
    "surface-2 1C1917": (0x1C, 0x19, 0x17),
    "surface-3 26211F": (0x26, 0x21, 0x1F),
    "line 2E2825": (0x2E, 0x28, 0x25),
    "shell F03018": (0xF0, 0x30, 0x18),
    "claw F07818": (0xF0, 0x78, 0x18),
    "amber F0A818": (0xF0, 0xA8, 0x18),
    "gold F0C030": (0xF0, 0xC0, 0x30),
    "text F5EFE6": (0xF5, 0xEF, 0xE6),
    "muted A39A90": (0xA3, 0x9A, 0x90),
    "success 5AD25A": (0x5A, 0xD2, 0x5A),
    "danger F03C32": (0xF0, 0x3C, 0x32),
    "info 28BEF0": (0x28, 0xBE, 0xF0),
    # light
    "light bg FAF7F2": (0xFA, 0xF7, 0xF2),
    "light surface F2EDE6": (0xF2, 0xED, 0xE6),
    "light surface E9E2D9": (0xE9, 0xE2, 0xD9),
    "light text 2A2420": (0x2A, 0x24, 0x20),
}

def dist(a, b):
    return sum((x - y) ** 2 for x, y in zip(a, b)) ** 0.5

def probe(path: str) -> None:
    im = Image.open(path).convert("RGB")
    w, h = im.size
    print(f"\n== {path} ({w}x{h})")
    regions = {
        "top-left (window/glow)": (6, 6),
        "sidebar mid": (w // 14, h // 2),
        "sidebar bottom": (w // 14, h - 40),
        "center": (w // 2, int(h * 0.45)),
        "bottom-center": (w // 2, h - 12),
        "right edge": (w - 6, int(h * 0.5)),
    }
    for name, (x, y) in regions.items():
        px = im.getpixel((x, y))
        best = min(TOKENS.items(), key=lambda kv: dist(px, kv[1]))
        d = dist(px, best[1])
        tag = best[0] if d < 26 else "no token match"
        print(f"  {name:<26} rgb{px}  -> {tag} (d={d:.0f})")
    # dominant palette
    small = im.resize((w // 4, h // 4))
    counts = Counter(small.getdata())
    print("  dominant colours:")
    hits = []
    for col, n in counts.most_common(40):
        best = min(TOKENS.items(), key=lambda kv: dist(col, kv[1]))
        d = dist(col, best[1])
        if d < 12 and n > 200:
            hits.append(f"{best[0]} rgb{col} x{n}")
    seen = set()
    for hit in hits:
        key = hit.split(" rgb")[0]
        if key in seen:
            continue
        seen.add(key)
        print(f"    {hit}")
    if not seen:
        print("    (no A2 token colour dominates)")

if __name__ == "__main__":
    for p in sys.argv[1:]:
        probe(p)
