#!/usr/bin/env python3
"""QA-r4 exploratory pass: walk every screen of Oino Desktop with the automation driver,
click everything, check focus/keyboard/empty/error states and both themes. Screenshots to
docs/dev/v1/qa-r4/shots/explore/. QA tooling (not product code).

    python3 docs/dev/v1/qa-r4/exploratory.py dark|light
"""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
D = ["python3", str(ROOT / "scripts/gui-drive.py"), "--socket",
     str(ROOT / "target/tmp/local-run/automation.sock")]
OUT = ROOT / f"docs/dev/v1/qa-r4/shots/explore-{sys.argv[1] if len(sys.argv) > 1 else 'dark'}"
FAILS: list[str] = []


def call(method, *args, timeout=60):
    r = subprocess.run(D + [method, *args], capture_output=True, text=True, timeout=timeout)
    if r.returncode != 0:
        return None, r.stderr.strip() or r.stdout.strip()
    try:
        return json.loads(r.stdout), None
    except json.JSONDecodeError:
        return r.stdout.strip(), None


def tree():
    r, err = call("tree", "--json", timeout=30)
    return r["elements"] if r else []


def state():
    r, err = call("state", timeout=30)
    return r if r else {}


def check(name, ok, note=""):
    print(f"{'ok  ' if ok else 'FAIL'} {name} {note}")
    if not ok:
        FAILS.append(name)


def shot(name):
    r, err = call("shot", str(OUT / f"{name}.png"), timeout=60)
    check(f"shot {name}", r is not None, str(err or ""))


def wait(*args, timeout_ms=15000):
    r, err = call("wait", *args, "--timeout", str(timeout_ms),
                  timeout=timeout_ms / 1000 + 30)
    return r is not None, err or ""


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    theme = sys.argv[1] if len(sys.argv) > 1 else "dark"

    # 1. welcome / sessions home
    ok, err = wait("composer", timeout_ms=40000)
    check("app up, composer present", ok, err)
    shot(f"01-welcome-{theme}")
    st = state()
    check("route sessions", st.get("route") == "sessions", str(st.get("route")))

    # empty-state hero + suggestions present
    texts = [e.get("label") or "" for e in tree()]
    check("empty session welcome text", any("Oino" in t for t in texts), "")

    # 2. sidebar navigation walk
    for nav, route in (("nav-board", "board"), ("nav-cron", "cron"), ("nav-ext", "extensions"),
                       ("nav-settings", "settings"), ("nav-sessions", "sessions")):
        r, err = call("click", nav)
        ok, err2 = wait("composer", timeout_ms=8000) if route == "sessions" else (True, "")
        st = state()
        check(f"nav {route}", st.get("route") == route, f"{st.get('route')} {err or ''}")
        shot(f"nav-{route}-{theme}")

    # 3. board: empty lane + FAB dialog open/cancel
    call("click", "nav-board")
    time.sleep(0.5)
    ok, err = wait("board-fab", timeout_ms=20000)
    check("board FAB present", ok, err)
    lanes = [e for e in tree() if "lane" in str(e.get("id", ""))]
    check("board lanes render", len(lanes) >= 3, f"{len(lanes)} lanes")
    call("click", "board-fab")
    ok, err = wait("--label", "Add task", timeout_ms=10000)
    check("FAB opens New task dialog", ok, err)
    shot(f"board-new-task-{theme}")
    r, err = call("key", "escape")
    time.sleep(0.5)
    st = state()
    check("Esc closes board dialog", not st.get("dialogs"), str(st.get("dialogs")))

    # 4. automations page + new-job dialog
    call("click", "nav-cron")
    time.sleep(0.5)
    shot(f"automations-{theme}")
    rows = [e for e in tree() if e.get("role") == "row" and "cron" in str(e.get("id", ""))]
    print(f"     automations rows: {len(rows)}")

    # 5. extensions page
    call("click", "nav-ext")
    time.sleep(0.5)
    ext = [e for e in tree() if e.get("role") in ("row", "card")]
    check("extensions page rows", len(ext) > 5, f"{len(ext)}")
    shot(f"extensions-{theme}")

    # 6. settings: nav sections + search
    call("click", "nav-settings")
    time.sleep(0.5)
    nav = [e for e in tree() if str(e.get("id", "")).startswith("set-nav") or e.get("role") == "row"]
    check("settings nav rows", len(nav) > 5, f"{len(nav)}")
    shot(f"settings-{theme}")
    r, err = call("focus", "settings-search") if any(e.get("id") == "settings-search" for e in tree()) else (None, "no search field")
    if r is not None:
        call("type", "theme")
        time.sleep(0.4)
        shot(f"settings-search-{theme}")
        call("type", "\b\b\b\b\b")  # rub out
    else:
        print("     (settings search field id differs; skipped)")

    # 7. palette: scopes, query, empty state, esc
    call("key", "secondary-k")
    ok, err = wait("--label", "New session", timeout_ms=8000)
    check("palette opens with ⌘K", ok, err)
    shot(f"palette-{theme}")
    call("type", "zzzznope")
    time.sleep(0.4)
    empt = [e for e in tree() if "no " in (e.get("label") or "").lower() or "nothing" in (e.get("label") or "").lower()]
    check("palette empty state", len(empt) >= 1, str([e.get("label") for e in empt][:3]))
    shot(f"palette-empty-{theme}")
    call("key", "escape")
    time.sleep(0.3)
    st = state()
    check("Esc closes palette", "palette" not in str(st.get("open", [])), str(st.get("open")))

    # 8. session view: composer focus + typing lands after overlays (QA-1 regression)
    call("click", "nav-sessions")
    ok, err = wait("composer", timeout_ms=10000)
    call("click", "composer")
    call("type", "xy")
    time.sleep(0.3)
    comp = [e for e in tree() if e.get("id") == "composer"]
    check("typing lands in composer", comp and "xy" in str(comp[0].get("value", "")),
          str(comp[:1]))
    # /info overlay + esc, then typing still lands
    call("type", "\b\b")
    call("type", "/info")
    call("key", "enter")
    ok, err = wait("--label", "Session", timeout_ms=20000)
    check("/info panel opens", ok, err)
    shot(f"info-panel-{theme}")
    call("key", "escape")
    time.sleep(0.4)
    call("click", "composer")
    call("type", "qq")
    time.sleep(0.3)
    comp = [e for e in tree() if e.get("id") == "composer"]
    check("focus returns after /info esc", comp and "qq" in str(comp[0].get("value", "")),
          str(comp[:1]))
    call("type", "\b\b")

    # 9. error state: unknown command
    call("type", "/definitely-not-a-command")
    call("key", "enter")
    time.sleep(0.8)
    texts = [e.get("label") or "" for e in tree()]
    check("unknown command feedback", any(("unknown" in t.lower() or "no command" in t.lower()
                                           or "not found" in t.lower()) for t in texts),
          str([t for t in texts if "command" in t.lower()][:3]))
    shot(f"unknown-command-{theme}")

    # 10. side pane launcher tabs
    r, err = call("click", "hdr-side")
    time.sleep(0.4)
    ok, err = wait("--id", "side-add", timeout_ms=8000)
    check("side pane + launcher", ok, err)
    shot(f"side-launcher-{theme}")

    # 11. mode chip / model chip menus (composer toolbar)
    ok, err = wait("mode-chip", timeout_ms=8000)
    if ok:
        call("click", "mode-chip")
        time.sleep(0.4)
        shot(f"mode-menu-{theme}")
        call("key", "escape")
    ok, err = wait("model-trigger", timeout_ms=8000)
    if ok:
        call("click", "model-trigger")
        time.sleep(0.4)
        shot(f"model-menu-{theme}")
        call("key", "escape")

    # 12. theme check by pixel if light
    print(f"\n{len(FAILS)} failures: {FAILS}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
