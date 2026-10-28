#!/usr/bin/env python3
"""QA-r4 local fallback suite runner (charter fallback while `ti` is unreachable).

Mirrors `apps/oino-desktop/scripts/local-app.sh` (private Xvfb :95 + own dbus + scratch
agent dir, one app instance, lavapipe, reduce motion) but adds what the `cargo xtask gui
suite` path does on `ti`: per-scenario `settings` merging and a clean scratch state before
each scenario, so the unipi/zcode/qa-r4 suite manifests can run locally with
`scripts/gui-drive.py --socket … run --no-restart`.

Usage:
    python3 docs/dev/v1/qa-r4/local-suite.py <suite-name> [--only a,b] [--backend demo|faux|real]
    python3 docs/dev/v1/qa-r4/local-suite.py --file docs/dev/v1/gui/scenarios/x.json

Not product code: QA tooling kept as evidence for docs/dev/v1/qa-r4.md.
"""

from __future__ import annotations

import argparse
import json
import shlex
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
RUN = ROOT / "target/tmp/local-run"
SOCK = RUN / "automation.sock"
DISPLAY = ":95"
DRIVER = ROOT / "scripts/gui-drive.py"
SUITES = ROOT / "docs/dev/v1/gui/suites"

STATE = {}  # pids of the display/dbus/app we started


def sh(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=ROOT, **kw)


def sh_ok(cmd: list[str]) -> None:
    """Best-effort helper step: never fails the run (missing `convert` etc.)."""
    try:
        subprocess.run(cmd, cwd=ROOT, stderr=subprocess.DEVNULL)
    except OSError:
        pass


def stop() -> None:
    for f in ("app", "dbus", "xvfb"):
        p = RUN / f"{f}.pid"
        if p.is_file():
            pid = p.read_text().strip()
            subprocess.run(["kill", pid], stderr=subprocess.DEVNULL)
            p.unlink(missing_ok=True)
    SOCK.unlink(missing_ok=True)
    time.sleep(1.0)


def merge(base: dict, extra: dict) -> dict:
    out = dict(base)
    for k, v in extra.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = {**out[k], **v}
        else:
            out[k] = v
    return out


def start(backend: str, scene: str | None, env: list[str], settings: dict) -> None:
    stop()
    agent = RUN / "agent"
    project = RUN / "project"
    binp = ROOT / "apps/target/release/oino-desktop"
    if not binp.is_file():
        sys.exit(f"no binary at {binp}")
    subprocess.run(
        ["rm", "-rf", str(agent / "sessions"), str(agent / "state"),
         str(project / ".oino"), str(project / "docs")]
    )
    (agent / "logs").mkdir(parents=True, exist_ok=True)
    project.mkdir(parents=True, exist_ok=True)
    (project / "README.md").write_text("local project\n")
    args: list[str] = []
    base: dict = {}
    if backend == "demo":
        args = ["--demo"]
    elif backend == "faux":
        base = {"defaultProvider": "faux", "defaultModel": "faux-1"}
        sh_ok(["cp", str(ROOT / "extensions/dist/provider-faux.wasm"), str(agent / "provider-faux.wasm")])
        args = ["--faux"]
    elif backend == "real":
        base = {"defaultProvider": "oino", "defaultModel": "deepseek/deepseek-flash"}
        (agent / "cache/models").mkdir(parents=True, exist_ok=True)
        sh_ok(["cp", str(Path.home() / ".oino/cache/models/oino.json"),
              str(agent / "cache/models/oino.json")])
        sh_ok(["cp", str(Path.home() / ".oino/auth.json"), str(agent / "auth.json")])
        sh_ok(["chmod", "600", str(agent / "auth.json")])
        # A solid red test PNG for the attachment scenarios (no ImageMagick needed).
        try:
            from PIL import Image
            Image.new("RGB", (96, 96), (224, 48, 24)).save(RUN / "attach-red.png")
        except OSError:
            pass
    else:
        sys.exit(f"unknown backend {backend}")
    (agent / "settings.json").write_text(json.dumps(merge(base, settings)))
    (agent / "desktop.json").write_text(json.dumps({"onboarded": True}))

    xvfb = subprocess.Popen(
        ["Xvfb", DISPLAY, "-screen", "0", "1600x1000x24", "-nolisten", "tcp"],
        stdout=(RUN / "xvfb.log").open("wb"), stderr=subprocess.STDOUT,
    )
    STATE["xvfb"] = xvfb.pid
    (RUN / "xvfb.pid").write_text(str(xvfb.pid))
    for _ in range(50):
        if Path(f"/tmp/.X11-unix/X{DISPLAY.lstrip(':')}").exists():
            break
        time.sleep(0.2)
    dbus = sh(["bash", "-c", "eval $(dbus-launch --sh-syntax) && echo $DBUS_SESSION_BUS_ADDRESS && echo $DBUS_SESSION_BUS_PID"],
              capture_output=True, text=True)
    lines = dbus.stdout.strip().splitlines()
    addr, pid = lines[0], lines[-1]
    (RUN / "dbus.pid").write_text(pid)

    extra_env = dict(kv.split("=", 1) for kv in env)
    if scene:
        extra_env["OINO_DESKTOP_DEMO_SCENE"] = scene
    import os

    app_env = dict(os.environ)
    app_env.update({
        "DISPLAY": DISPLAY,
        "DBUS_SESSION_BUS_ADDRESS": addr,
        "OINO_AGENT_DIR": str(agent),
        "OINO_DESKTOP_SIZE": "1600x1000",
        "OINO_REDUCE_MOTION": "1",
        "OINO_DESKTOP_AUTOMATION": str(SOCK),
        "VK_ICD_FILENAMES": "/usr/share/vulkan/icd.d/lvp_icd.json",
        "OINO_DESKTOP_THEME": os.environ.get("QA_THEME", app_env.get("OINO_DESKTOP_THEME", "")),
    })
    app_env.pop("WAYLAND_DISPLAY", None)
    app_env.update(extra_env)
    log = (RUN / "app.log").open("wb")
    app = subprocess.Popen(
        [str(binp), *args], cwd=project, env=app_env,
        stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    STATE["app"] = app.pid
    (RUN / "app.pid").write_text(str(app.pid))
    time.sleep(2)
    if app.poll() is not None:
        sys.exit(f"app exited rc={app.returncode}; see {RUN / 'app.log'}")


def wait_up(timeout_s: float = 60) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        r = sh(["python3", str(DRIVER), "--socket", str(SOCK), "wait", "composer",
                "--timeout", "2000"], capture_output=True)
        if r.returncode == 0:
            return
        time.sleep(1.0)
    sys.exit("the app did not come up with a composer")


def run_scenario(path: Path, backend_override: str | None) -> tuple[bool, float, str]:
    spec = json.loads(path.read_text())
    backend = backend_override or spec.get("backend", "demo")
    print(f"### {path.name} (backend {backend})", flush=True)
    start(backend, spec.get("scene"), spec.get("env", []), spec.get("settings", {}))
    wait_up()
    cmd = ["python3", str(DRIVER), "--socket", str(SOCK), "run", "--no-restart", str(path)]
    t0 = time.time()
    r = sh(cmd)
    return r.returncode == 0, time.time() - t0, (r.stdout or "") + (r.stderr or "")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("suite", nargs="?")
    ap.add_argument("--file", action="append", default=[])
    ap.add_argument("--only")
    ap.add_argument("--backend")
    args = ap.parse_args()

    files: list[tuple[str, Path]] = []
    if args.suite:
        manifest = json.loads((SUITES / f"{args.suite}.json").read_text())
        for s in manifest["scenarios"]:
            stem = Path(s["file"]).stem
            if args.only and not any(w in (s["group"], stem) for w in args.only.split(",")):
                continue
            files.append((s["group"], ROOT / s["file"]))
    for f in args.file:
        files.append(("adhoc", ROOT / f))

    results = []
    for group, path in files:
        ok, secs, out = run_scenario(path, args.backend)
        print(f"### {path.name}: {'PASS' if ok else 'FAIL'} ({secs:.0f}s)", flush=True)
        results.append((group, path.name, ok, secs))
        stop()

    print("\n== summary ==")
    for g, n, ok, s in results:
        print(f"{'PASS' if ok else 'FAIL'}  {g:<18} {n} ({s:.0f}s)")
    passed = sum(1 for r in results if r[2])
    out = RUN / f"suite-{args.suite or 'adhoc'}.json"
    out.write_text(json.dumps(
        {"suite": args.suite or "adhoc", "results": [
            {"group": g, "file": n, "ok": ok, "seconds": s} for g, n, ok, s in results]},
        indent=1))
    print(f"{passed}/{len(results)} passed — {out}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    finally:
        stop()
