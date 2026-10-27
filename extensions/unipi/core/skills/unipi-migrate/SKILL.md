---
name: unipi-migrate
description: Migrate an existing pi + unipi (@pi-unipi/unipi) install into Oino — settings, credentials, models, sessions, skills, prompts, themes, keybindings and every unipi module's config and state. Dry-run inventory first, explicit confirmation, backup of ~/.oino, verification and rollback. Opt-in; use only when the user asks to migrate from pi/unipi.
default-enabled: false
---

# Migrate pi + unipi into Oino

Oino ships no legacy unipi support in code: nothing reads `~/.pi` or `~/.unipi`. This skill
is the one-time, user-driven migration. You run every step with your normal tools (`bash`,
`read`, `write`, `edit`). The reference files hold the exact mappings:

| File | Holds |
|---|---|
| `references/settings-map.md` | pi `settings.json` keys, old `unipi.*` keys, per-namespace unipi config → `extensionSettings`, legacy key rewrites, secrets |
| `references/state-map.md` | every unipi module's files and dirs → Oino paths, sessions, resources, not-migrated list |
| `references/providers.md` | `auth.json` / `models.json` provider and API mapping |
| `references/commands.tsv` | old `/unipi:*` → new command names (tab separated, longest first) |
| `references/checklist.md` | verification checklist and rollback |

Read the reference for a phase before you start that phase.

## Ground rules

1. **Never modify the source.** `~/.pi`, `~/.unipi`, project `.pi/` and `.unipi/` dirs and
   `~/.mempalace` are read-only for you. Copy, never move. No `rm` outside the Oino target.
2. **Dry run first.** Phases 1–2 write nothing (except files under a scratch dir in `$TMPDIR`).
3. **Ask before writing.** Get an explicit "yes" to the inventory (use `ask_user` when
   available, else ask in chat and stop). The user may exclude items; respect that.
4. **Back up before writing** (phase 4). Abort if the backup fails.
5. **Never print secret values.** Report keys as `set` / `empty`; mask with `***`. Do not
   paste `auth.json`, `secrets.json` or tokens into chat, logs or the report.
6. **Do not overwrite** existing Oino data silently: on conflict keep the Oino value, record
   the conflict, and offer to overwrite in the final report.
7. Anything without an Oino equivalent is listed as **not migrated** with the reason.

## Variables

Resolve these once and show them in the inventory:

```bash
PI_DIR="${PI_CODING_AGENT_DIR:-$HOME/.pi/agent}"     # pi agent dir
UNIPI="$HOME/.unipi"                                   # unipi root
OINO="${OINO_AGENT_DIR:-$HOME/.oino}"                  # Oino agent dir (target)
STATE="$OINO/state"                                    # Oino extension state root
MIG="${TMPDIR:-/tmp}/oino-migrate-$(date +%Y%m%d-%H%M%S)"; mkdir -p "$MIG"   # scratch
```

Oino extension state lives at `$STATE/<extension-id>/global/…` (guest `/ext-global`) and
`$STATE/<extension-id>/workspace/<wsid>/…` (guest `/ext-workspace`). The Oino workspace id of
a project is the first 16 hex chars of the sha256 of its canonical git root (or the canonical
dir when it is not a git repo):

```bash
wsid() { r=$(cd "$1" && (git rev-parse --show-toplevel 2>/dev/null || pwd -P)); printf '%s' "$(realpath "$r")" | sha256sum | cut -c1-16; }
```

## Phase 1 — Discover (read-only)

Run these and keep the output in `$MIG/discover.txt`:

```bash
ls -la "$PI_DIR" "$UNIPI" 2>&1
ls "$PI_DIR"/{skills,prompts,themes,extensions} 2>/dev/null
ls "$PI_DIR"/sessions | wc -l; du -sh "$PI_DIR"/sessions 2>/dev/null
ls "$PI_DIR"/npm/node_modules/@pi-unipi 2>/dev/null; cat "$PI_DIR"/npm/package.json 2>/dev/null
python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print(sorted(d))' "$PI_DIR/settings.json"
python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print({k:v.get("type") for k,v in d.items()})' "$PI_DIR/auth.json"
python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print({k:(v.get("api"),len(v.get("models",[]))) for k,v in d.get("providers",{}).items()})' "$PI_DIR/models.json"
ls "$UNIPI"/config "$UNIPI"/global 2>/dev/null; ls "$UNIPI"/config/*/ 2>/dev/null
ls "$UNIPI"/memory | head; find "$UNIPI"/memory -name '*.md' | wc -l
ls "$UNIPI"/kanboard/projects 2>/dev/null; ls "$UNIPI"/skill-vault 2>/dev/null
ls "$UNIPI"/workspace | wc -l
cat ~/.mempalace/config.json 2>/dev/null | head -c 400; ls ~/.mempalace 2>/dev/null
ls -la "$OINO" 2>&1; cat "$OINO"/settings.json 2>/dev/null | head -c 2000
```

Projects: collect candidate project dirs from (a) the session dir names under
`$PI_DIR/sessions/` (`--home-u-src-app--` = `/home/u/src/app`; the name is ambiguous when a
path contains `-`, so confirm the dir exists), (b) the keys of `$PI_DIR/trust.json`, (c)
`.unipi-workspace.json` markers:

```bash
find "$HOME" -maxdepth 6 -name .unipi-workspace.json -not -path '*/node_modules/*' 2>/dev/null
```

Each marker holds `{"schemaVersion":1,"workspaceId":"<uuid>"}`; that uuid names
`$UNIPI/workspace/<uuid>/`. For each project record: path, unipi uuid (if any), Oino wsid,
presence of `.pi/`, `.unipi/`, `.oino/`, `AGENTS.md`/`CLAUDE.md`.

Get the live Oino settings schema (namespaces, field keys, which fields are secrets); you
need it in phase 5 to drop unknown keys:

```bash
cd "$MIG" && printf '%s\n' '{"type":"_oino/get_settings_schema","id":"s"}' | timeout 60 oino --mode rpc 2>/dev/null \
  | python3 -c 'import sys,json
for l in sys.stdin:
  try: d=json.loads(l)
  except Exception: continue
  if d.get("id")=="s": json.dump(d["data"]["namespaces"],open("schema.json","w"),indent=1)'
python3 - <<'PY'
import json
def keys(o,acc):
    if isinstance(o,dict):
        if "key" in o and "type" in o: acc.append((o["key"],o["type"]))
        for v in o.values(): keys(v,acc)
    elif isinstance(o,list):
        for v in o: keys(v,acc)
out={}
for n in json.load(open("schema.json")):
    acc=[]; keys(n["sections"],acc)
    out[n["namespace"]]={"owner":n["extensionId"],"fields":dict(acc),"defaults":n.get("defaults",{})}
json.dump(out,open("fields.json","w"),indent=1)
print({k:(v["owner"],len(v["fields"])) for k,v in out.items()})
PY
```

If `oino --mode rpc` hangs, kill it after 60 s; it is safe to rerun.

## Phase 2 — Dry-run inventory report

Write `$MIG/inventory.md` and show it to the user. One table per area (settings, credentials,
models, sessions, resources, each unipi module, projects). Columns:
`item | source path | target path / key | action (copy, merge, rewrite, skip) | notes`.
Include: counts (sessions, memory files, kanboard tasks, skills), conflicts with existing Oino
data, every **not migrated** item with its reason (see `references/state-map.md` §9), and
secrets as names only. End with the exact backup path you will create.

## Phase 3 — Confirmation

Ask: "Migrate everything listed (yes / no / exclude …)?" Proceed only on an explicit yes.
Record exclusions in `$MIG/inventory.md`.

## Phase 4 — Backup the target

```bash
BACKUP="$OINO.pre-unipi-migrate-$(date +%Y%m%d-%H%M%S)"
[ -d "$OINO" ] && cp -a "$OINO" "$BACKUP" && echo "backup: $BACKUP"
for p in <each project with .oino/>; do tar -C "$p" -czf "$MIG/$(basename "$p")-dot-oino.tgz" .oino; done
```

Exclude nothing from the `cp -a` (it is the rollback). If the Oino dir does not exist yet, note
"no previous Oino dir; rollback = delete $OINO". Tell the user the backup path.

## Phase 5 — Migrate

Do the steps in this order; after each, append a line to `$MIG/log.md`
(`step | done/skipped/conflict | detail`). Close running Oino sessions first (settings are
re-read on start).

1. **Core settings** — `references/settings-map.md` §1: `$PI_DIR/settings.json` →
   `$OINO/settings.json` (merge, Oino keys win on conflict); drop/rewrite the listed keys.
2. **Credentials and models** — `references/providers.md`: `auth.json`, `models.json`.
3. **unipi module settings** — `references/settings-map.md` §2–§4: every
   `$UNIPI/config/<ns>/config.json` (+ old `unipi.*` keys from pi settings) →
   `extensionSettings.<ns>`; secret fields through the RPC (§4) into `$OINO/secrets.json`.
4. **Resources** — `references/state-map.md` §2: skills, prompts, themes, keybindings,
   AGENTS.md / SYSTEM.md / APPEND_SYSTEM.md, trust.
5. **Sessions** — `references/state-map.md` §1.
6. **unipi module state** — `references/state-map.md` §3–§8: memory, skill vault and states,
   subagent profiles, kanboard, long-horizon/ralph, fusion presets, MCP servers, hints,
   input-shortcut stash, notify, web-api, updater, compactor, watchdog, background tasks.
7. **Projects** — for each project: `.pi/` → `.oino/`, `.unipi/` → `.oino/` and project
   `extensionSettings` (`references/state-map.md` §7).
8. **Command names** — rewrite `/unipi:*` in user files with `references/commands.tsv`
   (below).

Write JSON with a script (python3 `json.load` → modify → `json.dump(indent=2)`), never with
hand-made string edits. Write files atomically (`file.tmp` then `mv`). Keep `auth.json` and
`secrets.json` at mode `0600`.

### Command rename (step 8)

Files to scan (only these, and only text files): `$OINO/prompts/**/*.md`,
`$OINO/skills/**/*.md`, `$OINO/AGENTS.md`, `$OINO/SYSTEM.md`, `$OINO/APPEND_SYSTEM.md`,
`$STATE/unipi.subagents/global/profiles/*.md`, `$STATE/unipi.kanboard/projects/*/tasks/*.md`
(task bodies; leave the `## Activity` log lines as they are), and per project `AGENTS.md`, `CLAUDE.md`,
`.oino/{prompts,skills,agents}/**/*.md`, `.oino/ralph/*.md`, `.oino/SYSTEM.md`,
`.oino/APPEND_SYSTEM.md`. Do not touch session files (history stays as it was) or any file
outside these sets. Project files under git: show `git diff` to the user afterwards.

```bash
python3 - "$SKILL_DIR/references/commands.tsv" FILE... <<'PY'
import re,sys
rows=[l.rstrip("\n").split("\t") for l in open(sys.argv[1]) if l.strip() and not l.startswith("#")]
for f in sys.argv[2:]:
    t=open(f,encoding="utf-8").read(); n=t
    for old,new,*_ in rows:
        if new=="—": continue
        n=re.sub(re.escape(old)+r"(?![A-Za-z0-9-])", new, n)
    if n!=t: open(f,"w",encoding="utf-8").write(n); print("rewrote",f)
PY
```

(`$SKILL_DIR` is this skill's directory.) After it, `grep -rn '/unipi:' <same files>` must
only show `/unipi:cron` (no equivalent: flag it to the user) or nothing.

## Phase 6 — Verify and report

Run `references/checklist.md` §1 and give the user the final report: what was migrated
(counts), conflicts kept, not-migrated items with reasons, backup path, and the rollback
command (`references/checklist.md` §2). Suggest a fresh `oino` start and `/doctor`.
