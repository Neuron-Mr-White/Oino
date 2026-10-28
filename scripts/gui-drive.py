#!/usr/bin/env python3
"""Drive Oino Desktop deterministically: click, type and read the UI over the automation socket.

The app serves a JSON-lines RPC on a Unix socket when it runs with
`OINO_DESKTOP_AUTOMATION=<path>` (see `apps/oino-ui/src/automation/` and
docs/dev/v1/gui-driver.md). This client talks to that socket locally or, with `--ti`, through
an SSH pipe to the `ti` GUI host.

Usage
    scripts/gui-drive.py [--ti | --host H --socket S] <command> [args]

Commands
    ping                           round trip
    state                          screen, session id, pending dialogs
    tree [--role R] [--label L]    semantic element list (JSON with --json)
    click ID|--label L|--x X --y Y click an element (also double-click / right-click / hover)
    type TEXT [--submit]           type into the focused input
    key SPEC                       one keystroke (`enter`, `secondary-k`, `shift-tab`, …)
    wait ID|--label L|--text T     wait for an element or text (--absent, --timeout MS)
    shot PATH                      window PNG (fetched to this machine with --ti)
    run SCENARIO.json [...]        run a scenario file (steps + assertions, screenshots)
    quit                           close the app

Examples
    scripts/gui-drive.py --ti tree --role switch
    scripts/gui-drive.py --ti run docs/dev/v1/gui/scenarios/approval-allow.json
"""

from __future__ import annotations

import argparse
import base64
import json
import shlex
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

TI_HOST = "ti@ti"
TI_SOCKET = "/home/ti/oino-test/run/automation.sock"
TI_SHOTS = "docs/dev/v1/design/shots/gui"
TI_REMOTE_SHOTS = "/home/ti/oino-test/run/gui-shots"

# Runs on the far side of the SSH pipe: stdin lines -> socket -> stdout lines. It travels as
# base64 because SSH joins the remote command into one shell line (no quoting survives).
PROXY = (
    "import socket, sys\n"
    "s = socket.socket(socket.AF_UNIX)\n"
    "s.connect(sys.argv[1])\n"
    "f = s.makefile('rwb', buffering=0)\n"
    "for line in sys.stdin.buffer:\n"
    "    f.write(line if line.endswith(b'\\n') else line + b'\\n')\n"
    "    r = f.readline()\n"
    "    if not r:\n"
    "        break\n"
    "    sys.stdout.buffer.write(r)\n"
    "    sys.stdout.buffer.flush()\n"
)
PROXY_B64 = base64.b64encode(PROXY.encode()).decode()
PROXY_CMD = f"import base64;exec(base64.b64decode('{PROXY_B64}'))"


class DriverError(RuntimeError):
    """A failed RPC or assertion."""


class Client:
    """One JSON-lines connection to the app's automation socket."""

    def __init__(self, host: str | None, path: str) -> None:
        self.host = host
        self.path = path
        self.next_id = 0
        self.proc: subprocess.Popen[bytes] | None = None
        self.sock: socket.socket | None = None
        if host:
            # SSH joins the remote command into one shell line, so quote it here.
            remote = f"python3 -u -c {shlex.quote(PROXY_CMD)} {shlex.quote(path)}"
            self.proc = subprocess.Popen(
                ["ssh", host, remote],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
            )
        else:
            self.sock = socket.socket(socket.AF_UNIX)
            self.sock.settimeout(30.0)
            self.sock.connect(path)

    def close(self) -> None:
        if self.proc:
            try:
                if self.proc.stdin:
                    self.proc.stdin.close()
                self.proc.wait(timeout=10)
            except (OSError, subprocess.TimeoutExpired):
                self.proc.kill()
        if self.sock:
            self.sock.close()

    def call(self, method: str, params: dict | None = None, timeout: float = 120.0) -> dict:
        self.next_id += 1
        request = {"id": self.next_id, "method": method, "params": params or {}}
        line = (json.dumps(request) + "\n").encode()
        reply = self._exchange(line, timeout)
        if not reply.get("ok"):
            message = reply.get("error", {}).get("message", "unknown error")
            raise DriverError(f"{method}: {message}")
        return reply.get("result", {})

    def _exchange(self, line: bytes, timeout: float) -> dict:  # noqa: ARG002
        if self.sock:
            self.sock.settimeout(timeout)
            self.sock.sendall(line)
            chunks = b""
            while not chunks.endswith(b"\n"):
                chunk = self.sock.recv(1 << 20)
                if not chunk:
                    raise DriverError("the app closed the automation socket")
                chunks += chunk
            return json.loads(chunks.decode())

        assert self.proc and self.proc.stdin and self.proc.stdout
        self.proc.stdin.write(line)
        self.proc.stdin.flush()
        reply = self.proc.stdout.readline()
        if not reply:
            raise DriverError("the SSH pipe to the app closed")
        return json.loads(reply.decode())


# ── selectors ────────────────────────────────────────────────────────────────────────────────


def selector(args: argparse.Namespace) -> dict:
    """`params` fields naming an element: id, role, label or coordinates."""
    params: dict = {}
    for key in ("id", "role", "label"):
        value = getattr(args, key, None)
        if value:
            params[key] = value
    for key in ("x", "y"):
        value = getattr(args, key, None)
        if value is not None:
            params[key] = value
    if getattr(args, "include_hidden", False):
        params["include_hidden"] = True
    return params


def require_selector(params: dict, command: str) -> dict:
    if not params:
        raise DriverError(f"{command} needs an id, --label, --role or --x/--y")
    return params


# ── commands ────────────────────────────────────────────────────────────────────────────────


def cmd_ping(client: Client, _: argparse.Namespace) -> int:
    print(json.dumps(client.call("ping")))
    return 0


def cmd_state(client: Client, _: argparse.Namespace) -> int:
    print(json.dumps(client.call("state"), indent=2))
    return 0


def cmd_tree(client: Client, args: argparse.Namespace) -> int:
    result = client.call("tree", selector(args))
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        for node in result["elements"]:
            marks = []
            if node.get("disabled") is False or node.get("enabled") is False:
                marks.append("disabled")
            if node.get("checked") is not None:
                marks.append(f"checked={node['checked']}")
            if node.get("selected"):
                marks.append("selected")
            if node.get("focused"):
                marks.append("focused")
            if not node.get("visible", True):
                marks.append("hidden")
            extra = f"  [{', '.join(marks)}]" if marks else ""
            print(f"{node['id']:<28} {node['role']:<11} {node['label']!r}{extra}")
    return 0


def cmd_click(client: Client, args: argparse.Namespace) -> int:
    method = {
        "click": "click",
        "double": "double_click",
        "right": "right_click",
        "hover": "hover",
        "scroll": "scroll",
    }[args.how]
    params = selector(args)
    if method == "scroll":
        params["dx"] = args.dx
        params["dy"] = args.dy
        if not params.get("x") and not params.get("id") and not params.get("label"):
            raise DriverError("scroll needs x/y or an element selector")
    else:
        require_selector(params, method)
    print(json.dumps(client.call(method, params)))
    return 0


def cmd_type(client: Client, args: argparse.Namespace) -> int:
    print(json.dumps(client.call("type_text", {"text": args.text, "submit": args.submit})))
    return 0


def cmd_key(client: Client, args: argparse.Namespace) -> int:
    print(json.dumps(client.call("key", {"key": args.spec})))
    return 0


def cmd_wait(client: Client, args: argparse.Namespace) -> int:
    params = selector(args)
    if args.text:
        params["text"] = args.text
    if not params:
        raise DriverError("wait needs an id, --label, --role or --text")
    params["timeout_ms"] = args.timeout
    params["absent"] = args.absent
    print(json.dumps(client.call("wait_for", params, timeout=args.timeout / 1000 + 30)))
    return 0


def cmd_shot(client: Client, args: argparse.Namespace) -> int:
    path = args.path
    remote = bool(client.host)
    target = path if not remote else args.remote_path
    result = client.call("screenshot", {"path": target if target else _remote_shot_path()})
    local = Path(path)
    if remote:
        local.parent.mkdir(parents=True, exist_ok=True)
        with local.open("wb") as handle:
            subprocess.run(
                ["ssh", client.host, "cat", result["path"]],
                stdout=handle,
                check=True,
            )
    print(json.dumps({**result, "local": str(local), "bytes": local.stat().st_size}))
    return 0


def _remote_shot_path() -> str:
    return f"/home/ti/oino-test/run/gui-shot-{int(time.time() * 1000)}.png"


def cmd_quit(client: Client, _: argparse.Namespace) -> int:
    print(json.dumps(client.call("quit")))
    return 0


# ── scenarios ───────────────────────────────────────────────────────────────────────────────


ROOT = Path(__file__).resolve().parents[1]
LOCAL_RUN = ROOT / "target/tmp/local-run"
RED_PNG = "attach-red.png"


def resolve_run_path(client: Client, path: str) -> str:
    """`$RUN/<file>` → the app host's run dir (ti: `~/oino-test/run`, the local charter
    fallback: `target/tmp/local-run`), creating the solid-red test PNG there when it is
    missing (QA-r4 H-4: a scenario must not hardcode one host's drop path, and the app-side
    `drop` now refuses missing files instead of silently dropping nothing)."""
    if not path.startswith("$RUN/"):
        return path
    name = path[len("$RUN/"):]
    if client.host:
        return f"/home/ti/oino-test/run/{name}"
    target = LOCAL_RUN / name
    if name == RED_PNG and not target.is_file():
        target.parent.mkdir(parents=True, exist_ok=True)
        try:
            from PIL import Image
        except ImportError as error:
            raise DriverError(f"creating {target}: {error} (python3-PIL is required)") from None
        Image.new("RGB", (96, 96), (224, 48, 24)).save(target)
    return str(target)


class Scenario:
    """A JSON scenario: `{name, scene, shots, steps}` (docs/dev/v1/gui-driver.md §4)."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.spec = json.loads(path.read_text())
        self.name = self.spec.get("name", path.stem)
        self.scene = self.spec.get("scene")
        # `demo` (scripted, free) | `faux` (real core, no network) | `real` (providers from
        # the copied auth.json, e.g. oino/deepseek/deepseek-flash).
        self.backend = self.spec.get("backend", "demo")
        self.env: list[str] = self.spec.get("env", [])
        # Settings merged into the scratch agent dir's settings.json before the app starts
        # (extension switches a scenario needs, e.g. `unipi.ask-user.enabled`).
        self.settings: dict = self.spec.get("settings", {})
        self.shots = Path(self.spec.get("shots", "target/gui-shots")) / self.name
        self.steps = self.spec.get("steps", [])
        self.step = 0

    def __len__(self) -> int:
        return len(self.steps)


def run_scenario(client: Client, args: argparse.Namespace) -> int:
    scenario = Scenario(Path(args.file))
    if args.scene:
        scenario.scene = args.scene
    print(f"# scenario {scenario.name} ({scenario.path})", flush=True)
    failures = 0
    for index, step in enumerate(scenario.steps, start=1):
        scenario.step = index
        method, params = next(iter(step.items()))
        params = params or {}
        started = time.time()
        try:
            detail = run_step(client, scenario, method, params, args)
            note = f"{method}" if not detail else f"{method} {detail}"
            print(f"[{index:02d}/{len(scenario)}] ok   {note} ({(time.time() - started) * 1000:.0f} ms)", flush=True)
        except DriverError as error:
            failures += 1
            print(f"[{index:02d}/{len(scenario)}] FAIL {method}: {error}", flush=True)
            _fail_shot(client, scenario, index, args)
            if args.stop_on_failure:
                break
    print(f"# {scenario.name}: {'PASS' if failures == 0 else f'FAIL ({failures})'}", flush=True)
    return 0 if failures == 0 else 1


def run_step(client: Client, scenario: Scenario, method: str, params: dict, args: argparse.Namespace) -> str:
    """Runs one scenario step and returns a short human summary."""
    if method in ("click", "double_click", "right_click", "hover"):
        result = client.call(method, params)
        return f"{result.get('id') or result.get('at')}"
    if method == "type_text":
        payload = dict(params)
        payload.setdefault("submit", False)
        result = client.call("type_text", payload, timeout=180)
        return f"{result.get('typed')} chars"
    if method in ("key", "focus", "scroll", "settle", "ping"):
        return json.dumps(client.call(method, params))[:120]
    if method == "wait_for":
        payload = dict(params)
        payload.setdefault("timeout_ms", 10000)
        result = client.call("wait_for", payload, timeout=payload["timeout_ms"] / 1000 + 30)
        return f"matched after {result.get('ms')} ms"
    if method == "sleep":
        time.sleep(float(params.get("ms", 250)) / 1000)
        return f"{params.get('ms', 250)} ms"
    if method == "screenshot":
        name = params.get("name", f"{scenario.step:02d}-{scenario.name}")
        path = scenario.shots / f"{name}.png"
        return _take_shot(client, path, args)
    if method == "tree":
        result = client.call("tree", params)
        visible = [n for n in result["elements"] if n.get("visible")]
        return f"{len(visible)} visible / {len(result['elements'])} elements"
    if method == "state":
        result = client.call("state")
        print("      " + json.dumps(result))
        return ""
    if method in ("drop", "clipboard_image"):
        # Platform input injections for attachments (docs/dev/v1/gui-driver.md §2); `$RUN`
        # placeholders resolve to the app host's run dir.
        payload = dict(params)
        if isinstance(payload.get("paths"), list):
            payload["paths"] = [resolve_run_path(client, p) for p in payload["paths"]]
        if isinstance(payload.get("path"), str):
            payload["path"] = resolve_run_path(client, payload["path"])
        return json.dumps(client.call(method, payload))[:160]
    if method == "toggle":
        # Click a switch/checkbox and prove its `checked` state flipped (the initial state is
        # whatever the app persisted, so the scenario cannot assume it).
        timeout_ms = float(params.get("timeout_ms", 10000))
        before = find_node(client.call("tree", params)["elements"], params)
        if before is None:
            raise DriverError(f"no element matches {params}")
        start = before.get("checked")
        client.call("click", params)
        deadline = time.time() + timeout_ms / 1000
        while time.time() < deadline:
            now = find_node(client.call("tree", params)["elements"], params)
            if now is not None and now.get("checked") != start:
                return f"{now['id']}: checked {start} -> {now.get('checked')}"
            time.sleep(0.1)
        raise DriverError(f"{params.get('id') or params}: checked stayed {start}")
    if method == "assert":
        return check_assertions(client, params, args)
    if method == "wait_state":
        timeout_ms = float(params.get("timeout_ms", 15000))
        deadline = time.time() + timeout_ms / 1000
        last = ""
        while True:
            try:
                return check_assertions(client, params, args)
            except DriverError as error:
                last = str(error)
                if time.time() >= deadline:
                    raise DriverError(f"{last} (after {int(timeout_ms)} ms)") from None
                time.sleep(0.1)
    raise DriverError(f"unknown step {method!r}")


def check_assertions(client: Client, params: dict, args: argparse.Namespace) -> str:
    """`{route, element, absent, text, dialogs, busy}` assertions against state + tree."""
    params = {k: v for k, v in params.items() if k != "timeout_ms"}
    state = client.call("state")
    tree = client.call("tree")
    checked = []
    problems = []
    if "route" in params:
        checked.append(f"route={state.get('route')}")
        if state.get("route") != params["route"]:
            problems.append(f"route is {state.get('route')!r}, expected {params['route']!r}")
    if "busy" in params:
        checked.append(f"busy={state.get('busy')}")
        if bool(state.get("busy")) != bool(params["busy"]):
            problems.append(f"busy is {state.get('busy')}, expected {params['busy']}")
    if "dialogs" in params:
        want = params["dialogs"]
        have = state.get("dialogs", [])
        checked.append(f"dialogs={len(have)}")
        if want == 0 and have:
            problems.append(f"{len(have)} dialogs are queued: {have}")
        elif isinstance(want, int) and len(have) != want:
            problems.append(f"{len(have)} dialogs queued, expected {want}: {have}")
        elif isinstance(want, str):
            methods = [d.get("method") for d in have]
            if want not in methods:
                problems.append(f"no {want!r} dialog queued: {methods}")
    if "session_id" in params:
        checked.append(f"session_id={state.get('session_id')}")
        if not state.get("session_id"):
            problems.append("no session is open")
    if "chrome" in params:
        # Extension chrome: statuses, above/below-composer widgets and frame segments.
        chrome = state.get("chrome") or {}
        texts = [s.get("text") or "" for s in chrome.get("statuses", [])]
        texts += [w.get("text") or "" for w in chrome.get("widgets", [])]
        frame = chrome.get("frame") or {}
        texts += [s.get("text") or "" for key in ("top", "bottom") for s in frame.get(key, [])]
        needle = params["chrome"]
        checked.append(f"chrome={needle!r}")
        if not any(needle.lower() in text.lower() for text in texts):
            problems.append(f"no status, widget or frame segment contains {needle!r}: {texts}")
    if "chrome_absent" in params:
        # The inverse of `chrome`: no status, widget or frame segment contains the needle
        # (e.g. `/footer` off drops the stats strip).
        chrome = state.get("chrome") or {}
        texts = [s.get("text") or "" for s in chrome.get("statuses", [])]
        texts += [w.get("text") or "" for w in chrome.get("widgets", [])]
        frame = chrome.get("frame") or {}
        texts += [s.get("text") or "" for key in ("top", "bottom") for s in frame.get(key, [])]
        needle = params["chrome_absent"]
        checked.append(f"chrome_absent={needle!r}")
        hits = [text for text in texts if needle.lower() in text.lower()]
        if hits:
            problems.append(f"chrome still shows {needle!r}: {hits}")
    if "open" in params:
        entries = state.get("open", [])
        checked.append(f"open={params['open']!r}")
        if not any(params["open"].lower() in entry.lower() for entry in entries):
            problems.append(f"no open surface matches {params['open']!r}: {entries}")
    if "text" in params:
        needle = params["text"]
        hit = any(needle.lower() in (n.get("label") or "").lower() for n in tree["elements"])
        hit = hit or any(
            needle.lower() in (n.get("value") or "").lower() for n in tree["elements"]
        )
        checked.append(f"text={needle!r}")
        if not hit:
            problems.append(f"no element contains {needle!r}")
    if "element" in params:
        node = find_node(tree["elements"], params["element"])
        checked.append(f"element={params['element']}")
        if node is None:
            problems.append(f"no element matches {params['element']}")
    if "absent" in params:
        node = find_node(tree["elements"], params["absent"])
        checked.append(f"absent={params['absent']}")
        if node is not None:
            problems.append(f"element {node['id']} is still there")
    if problems:
        raise DriverError("; ".join(problems))
    return f"{', '.join(checked)}" if checked else ""


def find_node(nodes: list[dict], want: dict) -> dict | None:
    """First node matching every given field (`id`, `role`, `label`/`value` substring,
    `checked`, `selected`, `enabled`, `focused`, `visible`)."""
    for node in nodes:
        if "id" in want and node.get("id") != want["id"]:
            continue
        if "role" in want and node.get("role") != want["role"]:
            continue
        if "label" in want:
            label = (node.get("label") or "").lower()
            if want["label"].lower() not in label:
                continue
        if "value" in want:
            value = (node.get("value") or "").lower()
            if want["value"].lower() not in value:
                continue
        if any(key in want and node.get(key) != want[key]
               for key in ("checked", "selected", "enabled", "focused", "visible")):
            continue
        return node
    return None


def _take_shot(client: Client, path: Path, args: argparse.Namespace) -> str:
    remote = bool(client.host)
    if remote:
        # The app writes on its own host; the client fetches the PNG below.
        folder = f"{args.remote_path_prefix.rstrip('/')}/{path.parent.name}"
        target = f"{folder}/{path.name}"
        subprocess.run(["ssh", client.host, "mkdir", "-p", folder], check=False)
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        target = str(path.resolve())
    result = client.call("screenshot", {"path": target})
    if remote:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("wb") as handle:
            subprocess.run(["ssh", client.host, "cat", result["path"]], stdout=handle, check=True)
    size = result.get("width"), result.get("height")
    return f"{path} ({size[0]}x{size[1]})"


def _fail_shot(client: Client, scenario: Scenario, index: int, args: argparse.Namespace) -> None:
    try:
        path = scenario.shots / f"FAIL-{index:02d}.png"
        print("      " + _take_shot(client, path, args))
        print("      state: " + json.dumps(client.call("state")))
    except (DriverError, OSError, subprocess.CalledProcessError) as error:
        print(f"      (no failure screenshot: {error})")


# ── app lifecycle on `ti` ───────────────────────────────────────────────────────────────────


def merge_settings(base: dict, extra: dict) -> dict:
    """Deep-merges `extra` into `base` (one level below `extensionSettings`)."""
    merged = dict(base)
    for key, value in extra.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            merged[key] = {**merged[key], **value}
        else:
            merged[key] = value
    return merged


def ti_restart(
    socket_path: str,
    scene: str | None,
    backend: str = "demo",
    extra_env: list[str] | None = None,
    settings: dict | None = None,
) -> None:
    """Restarts the app on `ti` with the automation socket (and demo scene) enabled."""
    env = ["--env", f"OINO_DESKTOP_AUTOMATION={socket_path}"]
    for pair in extra_env or []:
        env += ["--env", pair]
    if scene:
        env += ["--env", f"OINO_DESKTOP_DEMO_SCENE={scene}"]
    subprocess.run(["cargo", "xtask", "ti", "stop"], check=False)
    if backend == "real":
        # A real run needs a default model and a non-first-run agent dir (no onboarding).
        # Core settings are pi's `defaultProvider`/`defaultModel` pair (not `model:`); the
        # router needs its cached model catalog, copied from this PC (docs/dev/v1/ti.md §2).
        base = {"defaultProvider": "oino", "defaultModel": "deepseek/deepseek-flash"}
        merged = json.dumps(merge_settings(base, settings or {}))
        desktop = json.dumps({"onboarded": True})
        script = (
            "set -e; mkdir -p ~/oino-test/agent ~/oino-test/project\n"
            # A real scenario starts from a clean slate: no resumed session, no leftover
            # workspace state (plan/permission mode, long-horizon runs, kanboard db) from the
            # previous run, so assertions cannot pass on another scenario's output.
            "rm -rf ~/oino-test/agent/sessions ~/oino-test/agent/state "
            "~/oino-test/project/.oino ~/oino-test/project/docs\n"
            f"printf '%s' {shlex.quote(merged)} > ~/oino-test/agent/settings.json\n"
            f"printf '%s' {shlex.quote(desktop)} > ~/oino-test/agent/desktop.json"
        )
        # A solid-colour PNG for the attachment scenarios (drop / clipboard paste).
        script += (
            "\nconvert -size 96x96 xc:'#e03018' /home/ti/oino-test/run/attach-red.png"
            "\nidentify /home/ti/oino-test/run/attach-red.png"
        )
        subprocess.run(["ssh", TI_HOST, script], check=True)
        # The router's model list is a dynamic catalog: the app reads
        # `<agent>/cache/models/<provider>.json` until its provider answers, and a stale or
        # missing file makes `oino:deepseek/deepseek-flash` unresolvable (ti.md §2).
        catalog = Path.home() / ".oino/cache/models/oino.json"
        if catalog.is_file():
            subprocess.run(
                ["ssh", TI_HOST, "mkdir -p ~/oino-test/agent/cache/models"], check=True
            )
            subprocess.run(
                ["scp", "-q", str(catalog), f"{TI_HOST}:oino-test/agent/cache/models/oino.json"],
                check=True,
            )
    run = ["cargo", "xtask", "ti", "start"]
    if backend == "demo":
        run.append("--demo")
    elif backend == "faux":
        run.append("--faux")
    run += env
    subprocess.run(run, check=True)


def wait_for_app(client: Client, timeout_s: float = 40.0) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            client.call("wait_for", {"id": "composer", "timeout_ms": 2000}, timeout=15)
            return
        except DriverError:
            time.sleep(0.5)
    raise DriverError("the app did not come up with a composer")


# ── CLI ─────────────────────────────────────────────────────────────────────────────────────


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Drive Oino Desktop over its automation socket.")
    parser.add_argument("--ti", action="store_true", help=f"talk to {TI_HOST} over SSH")
    parser.add_argument("--host", help="SSH host of the machine running the app")
    parser.add_argument("--socket", help="automation socket path (default: per mode)")
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("ping")
    sub.add_parser("state")

    tree = sub.add_parser("tree")
    tree.add_argument("--role")
    tree.add_argument("--label")
    tree.add_argument("--id")
    tree.add_argument("--json", action="store_true")

    for name, how in (("click", "click"), ("double-click", "double"), ("right-click", "right"), ("hover", "hover")):
        click = sub.add_parser(name)
        click.add_argument("id", nargs="?")
        click.add_argument("--label")
        click.add_argument("--role")
        click.add_argument("--x", type=float)
        click.add_argument("--y", type=float)
        click.set_defaults(how=how)

    scroll_cmd = sub.add_parser("scroll")
    scroll_cmd.add_argument("id", nargs="?")
    scroll_cmd.add_argument("--label")
    scroll_cmd.add_argument("--role")
    scroll_cmd.add_argument("--x", type=float)
    scroll_cmd.add_argument("--y", type=float)
    scroll_cmd.add_argument("--dy", type=float, default=0.0)
    scroll_cmd.add_argument("--dx", type=float, default=0.0)
    scroll_cmd.set_defaults(how="scroll")

    type_cmd = sub.add_parser("type")
    type_cmd.add_argument("text")
    type_cmd.add_argument("--submit", action="store_true")

    key = sub.add_parser("key")
    key.add_argument("spec")

    wait = sub.add_parser("wait")
    wait.add_argument("id", nargs="?")
    wait.add_argument("--label")
    wait.add_argument("--role")
    wait.add_argument("--text")
    wait.add_argument("--absent", action="store_true")
    wait.add_argument("--timeout", type=int, default=10000)

    shot = sub.add_parser("shot")
    shot.add_argument("path")
    shot.add_argument("--remote-path")

    run = sub.add_parser("run")
    run.add_argument("file")
    run.add_argument("--scene", help="demo scene to launch with (overrides the scenario)")
    run.add_argument("--no-restart", action="store_true", help="use the app that is already up")
    run.add_argument(
        "--backend",
        choices=("demo", "faux", "real"),
        help="app backend (default: the scenario's `backend`, else demo)",
    )
    run.add_argument("--scene-demo", action="store_true", default=True, help=argparse.SUPPRESS)
    run.add_argument("--stop-on-failure", action="store_true", default=True, help=argparse.SUPPRESS)
    run.add_argument(
        "--remote-path-prefix",
        default=TI_REMOTE_SHOTS,
        help="where screenshots land on the app host (they are fetched back)",
    )

    sub.add_parser("quit")
    return parser


def main(argv: list[str]) -> int:
    args = build_parser().parse_args(argv)
    host = args.host or (TI_HOST if args.ti else None)
    path = args.socket or (TI_SOCKET if args.ti else os.environ.get("OINO_DESKTOP_AUTOMATION"))
    if not path:
        print(
            "no automation socket: pass --socket, --ti, or set OINO_DESKTOP_AUTOMATION",
            file=sys.stderr,
        )
        return 2

    if args.command == "run" and not args.no_restart:
        scenario = Scenario(Path(args.file))
        scene = args.scene or scenario.scene
        backend = args.backend or scenario.backend
        print(
            f"# restarting the app ({backend}, {'demo scene ' + scene if scene else 'default'})",
            flush=True,
        )
        ti_restart(path, scene, backend, scenario.env, scenario.settings)

    client = Client(host, path)
    try:
        if args.command == "run" and not args.no_restart:
            wait_for_app(client)
        handlers = {
            "ping": cmd_ping,
            "state": cmd_state,
            "tree": cmd_tree,
            "click": cmd_click,
            "double-click": cmd_click,
            "right-click": cmd_click,
            "hover": cmd_click,
            "scroll": cmd_click,
            "type": cmd_type,
            "key": cmd_key,
            "wait": cmd_wait,
            "shot": cmd_shot,
            "run": lambda c, a: run_scenario(c, a),
            "quit": cmd_quit,
        }
        return handlers[args.command](client, args)
    except DriverError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
