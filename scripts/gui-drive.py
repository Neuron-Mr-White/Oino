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
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

TI_HOST = "ti@ti"
TI_SOCKET = "/home/ti/oino-test/run/automation.sock"
TI_SHOTS = "docs/dev/v1/design/shots/gui"

# Runs on the far side of the SSH pipe: stdin lines -> socket -> stdout lines.
PROXY = r"""
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
f = s.makefile('rwb', buffering=0)
for line in sys.stdin.buffer:
    f.write(line if line.endswith(b'\n') else line + b'\n')
    r = f.readline()
    if not r:
        break
    sys.stdout.buffer.write(r)
    sys.stdout.buffer.flush()
"""


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
            self.proc = subprocess.Popen(
                ["ssh", host, "python3", "-u", "-c", PROXY, path],
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

    def _exchange(self, line: bytes, timeout: float) -> dict:
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
        deadline = time.time() + timeout
        out = b""
        while not out.endswith(b"\n"):
            if time.time() > deadline:
                raise DriverError("timed out waiting for the app (SSH pipe)")
            chunk = self.proc.stdout.read(1 << 20)
            if not chunk:
                raise DriverError("the SSH pipe to the app closed")
            out += chunk
        return json.loads(out.decode())


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
    method = {"click": "click", "double": "double_click", "right": "right_click", "hover": "hover"}[
        args.how
    ]
    params = require_selector(selector(args), method)
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


class Scenario:
    """A JSON scenario: `{name, scene, shots, steps}` (docs/dev/v1/gui-driver.md §4)."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.spec = json.loads(path.read_text())
        self.name = self.spec.get("name", path.stem)
        self.scene = self.spec.get("scene")
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
        (method, params), *_ = [(item) for item in step.items()][0]
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
    if method == "assert":
        return check_assertions(client, params, args)
    raise DriverError(f"unknown step {method!r}")


def check_assertions(client: Client, params: dict, args: argparse.Namespace) -> str:
    """`{route, element, absent, text, dialogs, busy}` assertions against state + tree."""
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
    for node in nodes:
        if "id" in want and node.get("id") != want["id"]:
            continue
        if "role" in want and node.get("role") != want["role"]:
            continue
        if "label" in want:
            label = (node.get("label") or "").lower()
            if want["label"].lower() not in label:
                continue
        return node
    return None


def _take_shot(client: Client, path: Path, args: argparse.Namespace) -> str:
    remote = bool(client.host)
    target = args.remote_path_prefix.rstrip("/") + "/" + path.name if remote else str(path)
    if remote:
        subprocess.run(["ssh", client.host, "mkdir", "-p", args.remote_path_prefix], check=False)
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


def ti_restart(socket_path: str, scene: str | None, demo: bool = True) -> None:
    """Restarts the app on `ti` with the automation socket (and demo scene) enabled."""
    env = ["--env", f"OINO_DESKTOP_AUTOMATION={socket_path}"]
    if scene:
        env += ["--env", f"OINO_DESKTOP_DEMO_SCENE={scene}"]
    subprocess.run(["cargo", "xtask", "ti", "stop"], check=False)
    run = ["cargo", "xtask", "ti", "start"]
    if demo:
        run.append("--demo")
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
    run.add_argument("--scene-demo", action="store_true", default=True, help=argparse.SUPPRESS)
    run.add_argument("--stop-on-failure", action="store_true", default=True, help=argparse.SUPPRESS)
    run.add_argument(
        "--remote-path-prefix",
        default=TI_SHOTS,
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
        print(f"# restarting the app ({'demo scene ' + scene if scene else 'default'})", flush=True)
        ti_restart(path, scene)

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
