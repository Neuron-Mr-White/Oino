#!/usr/bin/env bash
# Fake pi + unipi install for testing the bundled `unipi-migrate` skill.
#
#   scripts/unipi-migrate-fixture.sh make  <dir>   # writes <dir>/home (HOME) with ~/.pi, ~/.unipi, a project
#   scripts/unipi-migrate-fixture.sh check <dir>   # checks <dir>/home/.oino after a migration run
#
# End to end: make the fixture, give <dir>/home/.oino credentials for one model, then run the
# skill in two print-mode turns with HOME=<dir>/home OINO_AGENT_DIR=<dir>/home/.oino (cwd
# <dir>/home): `oino -p "/skill:unipi-migrate … phases 1-2, then stop"` and
# `oino -c -p "Yes, migrate everything … project <dir>/home/src/proj …"`, then `check`.
# UNIPI_MIGRATE_REAL=1 adds scrubbed copies of a few small files from the real ~/.unipi.
#
# Everything is synthetic (fake keys); nothing outside <dir> is read or written.
set -euo pipefail

cmd="${1:?make|check}"; root="$(realpath -m "${2:?dir}")"; H="$root/home"
PI="$H/.pi/agent"; U="$H/.unipi"; P="$H/src/proj"; O="$H/.oino"

j() { mkdir -p "$(dirname "$1")"; cat > "$1"; }

make_fixture() {
  if [ -e "$root" ] && [ -n "$(ls -A "$root")" ] && [ ! -f "$root/.unipi-migrate-fixture" ]; then
    echo "refusing to replace $root: not empty and not a fixture dir" >&2; exit 2
  fi
  rm -rf "$root"; mkdir -p "$PI" "$U" "$P"; touch "$root/.unipi-migrate-fixture"
  git -C "$P" init -q
  local wsuuid=11111111-2222-4333-8444-555555555555
  printf '{"schemaVersion":1,"workspaceId":"%s"}\n' "$wsuuid" > "$P/.unipi-workspace.json"

  # ── pi ──
  j "$PI/settings.json" <<EOF
{"lastChangelogVersion":"1.4.0","defaultProvider":"oino","defaultModel":"deepseek/deepseek-flash",
 "defaultThinkingLevel":"low","steeringMode":"all","theme":"mytheme","quietStartup":true,
 "enableInstallTelemetry":true,"compaction":{"enabled":true},
 "packages":["npm:@pi-unipi/unipi","npm:pi-some-ts-package"],"extensions":["extensions/foo.ts"],
 "prompts":["prompts-extra"],
 "unipi.info.bootMode":"off","unipi.longHorizon.defaultMode":"goal","unipi.skills.discovery":false}
EOF
  j "$PI/auth.json" <<'EOF'
{"openrouter":{"type":"api_key","key":"sk-or-FAKE"},
 "anthropic":{"type":"oauth","access":"FAKE-A","refresh":"FAKE-R","expires":1},
 "antigravity":{"type":"oauth","access":"x","refresh":"y","expires":1},
 "mygw":{"type":"api_key","key":"FAKE-GW"},
 "orphan":{"type":"api_key","key":"FAKE-ORPHAN"}}
EOF
  chmod 600 "$PI/auth.json"
  j "$PI/models.json" <<'EOF'
{"providers":{
 "mygw":{"baseUrl":"https://gw.example.invalid/v1","api":"openai-completions","apiKey":"MYGW_KEY","models":[{"id":"m1","name":"M1","contextWindow":8192,"maxTokens":1024}]},
 "gemcli":{"baseUrl":"https://x.invalid","api":"google-gemini-cli","models":[{"id":"g"}]},
 "openrouter":{"baseUrl":"https://proxy.example.invalid/api/v1"}}}
EOF
  printf '{"%s": true}\n' "$P" > "$PI/trust.json"
  j "$PI/keybindings.json" <<<'{"app.model.select":"ctrl+l"}'
  j "$PI/themes/mytheme.json" <<<'{"name":"mytheme","vars":{},"colors":{}}'
  j "$PI/prompts/review.md" <<'EOF'
---
description: Review then plan
---
Run /unipi:plan first, then /unipi:memory-search auth and /unipi:model. Not /unipi:modelx.
EOF
  mkdir -p "$PI/prompts-extra"; j "$PI/prompts-extra/extra.md" <<<'Extra prompt, see /unipi:goal.'
  j "$PI/skills/my-skill/SKILL.md" <<'EOF'
---
name: my-skill
description: A user skill that mentions /unipi:goal.
---
Use /unipi:goal to finish, /unipi:settings to configure, /unipi:cron never.
EOF
  j "$PI/AGENTS.md" <<<'Global rules: open /unipi:settings, then /unipi:kanboard-add.'
  j "$PI/extensions/foo.ts" <<<'export default () => {}'
  j "$PI/mcp.json" <<<'{"mcpServers":{"fs":{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem","/tmp"]}}}'
  enc() { printf -- '--%s--' "$(printf '%s' "${1#/}" | tr / -)"; }
  local sdir; sdir="$PI/sessions/$(enc "$P")"
  mkdir -p "$sdir" "$PI/sessions/$(enc "$H")"
  j "$sdir/2026-10-01T00-00-00-000Z_aaaaaaaa.jsonl" <<EOF
{"type":"session","version":3,"id":"aaaaaaaa-0000-4000-8000-000000000001","timestamp":"2026-10-01T00:00:00.000Z","cwd":"$P"}
{"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2026-10-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"/unipi:plan hello"}],"timestamp":1}}
EOF
  j "$PI/sessions/$(enc "$H")/2026-10-02T00-00-00-000Z_bbbbbbbb.jsonl" <<EOF
{"type":"session","version":3,"id":"bbbbbbbb-0000-4000-8000-000000000002","timestamp":"2026-10-02T00:00:00.000Z","cwd":"$H"}
EOF
  j "$PI/herdr-agent-state.v8.json" <<<'{}'
  mkdir -p "$PI/npm/node_modules/@pi-unipi/unipi"

  # ── unipi config ──
  local C="$U/config"
  j "$C/compactor/config.json" <<<'{"overrideDefaultCompaction":false,"autoCompaction":{"enabled":true,"thresholdPercent":70,"cooldownMs":5},"sections":{"files":false},"fts5Index":{"enabled":true}}'
  j "$C/footer/config.json" <<<'{"colorMode":"mono","preset":"classic","separator":"|"}'
  j "$C/info-screen/config.json" <<<'{"showOnBoot":false}'
  j "$C/utility/config.json" <<<'{"badge":{"generationModel":"openrouter/x/y","autoGen":false},"skills":{"mode":"all","maxSkills":20}}'
  j "$C/skills/config.json" <<<'{"proxy":true,"states":{"alpha":"on","beta":"unlisted","gamma":{"discoverable":false}}}'
  j "$C/kanboard/config.json" <<<'{"doCredits":3,"blocking":true,"defaultStrategy":"goal","reminders":false}'
  j "$C/long-horizon/config.json" <<<'{"judge":{"enabled":true,"provider":"typesafe","model":"typesafe/jev-1.13","baseUrl":"","threshold":0.7,"timeoutMs":0,"apiKey":"FAKE-JUDGE"},"verifierModel":"openrouter/a/b"}'
  j "$C/permission/config.json" <<<'{"mode":"auto"}'
  j "$C/notify/config.json" <<<'{"defaultPlatforms":["telegram"],"telegram":{"enabled":true,"botToken":"FAKE-TG","chatId":"FAKE-CHAT"},"events":{"agent_end":{"enabled":true,"platforms":[]},"workflow_end":{"enabled":true}}}'
  j "$C/notify/ntfy.json" <<<'{"enabled":true,"priority":4,"serverUrl":"https://ntfy.example.invalid","topic":"mytopic"}'
  j "$C/notify/app-endpoints.json" <<<'{"endpoints":[]}'
  j "$C/web-api/config.json" <<<'{"providers":{"tavily":{"enabled":true,"apiKey":"FAKE-TAVILY"}}}'
  j "$C/web-api/auth.json" <<<'{"serpapi":"FAKE-SERP"}'
  j "$C/decision-model/config.json" <<<'{"provider":"openrouter","model":"typesafe/jev-1.13","apiKey":"FAKE-DM"}'
  j "$C/mcp/config.json" <<<'{"servers":{"fs":{"enabled":false,"addedAt":"2026-01-01"}},"sync":{"enabled":true}}'
  j "$C/mcp/mcp-config.json" <<<'{"mcpServers":{"git":{"command":"uvx","args":["mcp-server-git"]}}}'
  j "$C/mcp/servers.json" <<<'{"servers":[],"totalServers":0}'
  j "$C/fusion/config.json" <<<'{"startup":{"model":"openrouter/a/b","thinking":"low"}}'
  j "$C/fusion/preset.json" <<<'{"schema_version":1,"default":{"lead":"openrouter/a/b","sidekick":"openrouter/c/d"},"recent":[]}'
  j "$C/cron/config.json" <<<'{"concurrency":1}'
  j "$C/tips/config.json" <<<'{"enabled":true}'
  j "$C/subagents.json" <<<'{"maxConcurrent":3,"types":{"explore":{"enabled":false}}}'
  j "$C/background-tasks.json" <<<'{"maxFinishedTasks":7,"defaultTimeoutSeconds":60}'
  j "$C/watchdog/config.json" <<<'{"sessionCaps":{"maxTurns":50},"enabled":true}'
  j "$C/input-shortcuts.json" <<<'{"stash":"half typed prompt"}'
  j "$C/settings-version.json" <<<'{"migrated_version":"3.0.0"}'
  j "$C/agents/helper.md" <<'EOF'
---
display_name: Helper
description: Helper agent; reports with /unipi:memory.
tools: read, bash
model: openrouter/a/b
enabled: true
---
Call /unipi:memory-process when done.
EOF
  cp "$C/agents/helper.md" "$C/agents/helper.md.bak"

  # ── unipi state ──
  j "$U/memory/proj/decision/use_rust.md" <<'EOF'
---
id: use_rust
title: use_rust
tags:
  - rust
project: proj
created: '2026-09-01T00:00:00.000Z'
updated: '2026-09-01T00:00:00.000Z'
type: decision
---

We use Rust.
EOF
  j "$U/memory/proj/memory.db" <<<''
  j "$U/global/hints/hints.json" <<<'{"counts":{"cmd.settings":{"count":2,"last":1}}}'
  mkdir -p "$U/global/image"; printf 'PNG' > "$U/global/image/a.png"
  j "$U/kanboard/settings.json" <<<'{"piCommand":["pi"],"agentCommand":["pi"],"models":["openrouter/a/b"],"summaryModel":"openrouter/a/b","summaryInstruction":"be short"}'
  j "$U/kanboard/projects/proj-abc123/project.json" <<EOF
{"slug":"proj-abc123","name":"proj","root":"$P","prefix":"PRO","nextId":3,"createdAt":"2026-09-28T15:25:32Z","archived":false}
EOF
  j "$U/kanboard/projects/proj-abc123/tasks/PRO-1.md" <<'EOF'
---
id: PRO-1
title: First task
status: in_progress
priority: none
order: 1000
deps: []
labels: []
strategy: goal
plan: true
created: 2026-09-28T13:24:28Z
updated: 2026-09-30T11:17:26Z
run:
  session: s1
  pid: 1
  host: h
  mode: direct
  owner: system
  started: 2026-09-30T11:17:26Z
---

Do it with /unipi:goal.

## Activity
- 2026-09-28T13:24:28Z [user] created in backlog
EOF
  j "$U/kanboard/projects/proj-abc123/tasks/PRO-2.md" <<'EOF'
---
id: PRO-2
title: Second task
status: backlog
priority: none
order: 2000
deps: []
labels: []
created: 2026-09-28T13:24:28Z
updated: 2026-09-28T13:24:28Z
run:
---

Body.

## Activity
- 2026-09-28T13:24:28Z [user] created in backlog
EOF
  j "$U/kanboard/projects/proj-abc123/queues/pi-1.json" <<<'{}'
  j "$U/kanboard/daemon.lock" <<<''
  j "$U/skill-vault/vaulted/SKILL.md" <<<$'---\nname: vaulted\ndescription: A vault skill.\n---\nbody'
  local W="$U/workspace/$wsuuid"
  j "$W/state/long-horizon/goal.json" <<<'{"version":1,"goal":{"goalId":"g1","objective":"Ship v2","objectiveDigest":"d","status":"active","tokenBudget":null,"tokensBaselinePending":false,"tokensAtStart":10,"tokensNow":20,"tokensEstimated":false,"turn":3,"maxTurns":50,"noProgressStreak":0,"stallCap":8,"notMetStreak":0,"inconclusiveStreak":0,"blockedProposalStreak":0,"lease":{"sid":"x"},"revision":4,"updatedAt":"2026-10-01T00:00:00Z","kickoffDelivered":true,"reason":""}}'
  j "$W/state/long-horizon/state.json" <<<'{"version":1,"history":[]}'
  j "$W/state/long-horizon/ralph/chores.md" <<<'- [ ] tidy with /unipi:ralph'
  j "$W/config/footer/config.json" <<<'{"enabled":false}'
  j "$U/app-host/config.json" <<<'{}'
  j "$U/cache/usage-stats.json" <<<'{}'
  j "$U/config/models-cache.json" <<<'{}'

  # ── project ──
  j "$P/.pi/settings.json" <<<'{"defaultThinkingLevel":"high","extensions":["./x.ts"]}'
  j "$P/.pi/prompts/p.md" <<<'Project prompt: /unipi:btw why?'
  j "$P/.unipi/config/permission/config.json" <<<'{"mode":"full"}'
  j "$P/.unipi/ralph/task.md" <<<'- [ ] project task, see /unipi:visualize-progress'
  j "$P/AGENTS.md" <<<'Project rules: /unipi:kanboard and /unipi:compact-then summarize.'
  j "$H/.mempalace/config.json" <<<'{"palace_path":"~/.mempalace/palace"}'
  [ "${UNIPI_MIGRATE_REAL:-0}" = 1 ] && add_real
  echo "fixture: HOME=$H  project=$P  (pi $PI, unipi $U)"
}

# Optional (UNIPI_MIGRATE_REAL=1): scrubbed copies of a few small real files from the
# caller's ~/.unipi (read-only; values of keys named like key/token/secret are replaced).
add_real() {
  local R="${REAL_HOME:-$HOME}/.unipi"
  scrub() { python3 - "$1" "$2" <<'PY'
import json,re,sys
d=json.load(open(sys.argv[1]))
def s(o):
    if isinstance(o,dict): return {k:("SCRUBBED" if re.search("key|token|secret|pass",k,re.I) and isinstance(v,str) and v else s(v)) for k,v in o.items()}
    if isinstance(o,list): return [s(x) for x in o]
    return o
json.dump(s(d),open(sys.argv[2],"w"),indent=1)
PY
  }
  for ns in ask-user hints image memory subagents; do
    [ -f "$R/config/$ns/config.json" ] && mkdir -p "$U/config/$ns" && scrub "$R/config/$ns/config.json" "$U/config/$ns/config.json"
  done
  for a in explore.md glm-worker.md; do [ -f "$R/config/agents/$a" ] && cp "$R/config/agents/$a" "$U/config/agents/real-$a"; done
  [ -f "$R/global/hints/hints.json" ] && cp "$R/global/hints/hints.json" "$U/global/hints/hints.json"
  local m; m=$(find "$R/memory" -mindepth 3 -maxdepth 3 -path '*/preference/*.md' -print -quit 2>/dev/null || true)
  [ -n "$m" ] && mkdir -p "$U/memory/proj/preference" && cp "$m" "$U/memory/proj/preference/"
  local k; k=$(find "$R/kanboard/projects" -path '*/tasks/*.md' -print -quit 2>/dev/null || true)
  [ -n "$k" ] && sed -e 's/^id: .*/id: PRO-9/' "$k" > "$U/kanboard/projects/proj-abc123/tasks/PRO-9.md"
  echo "added scrubbed real files from $R"
}

fail=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; fail=1; }
t() { if eval "$2" >/dev/null 2>&1; then ok "$1"; else bad "$1"; fi; }

check_fixture() {
  local S="$O/state" ws; ws=$(printf '%s' "$(realpath "$P")" | sha256sum | cut -c1-16)
  py() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); $2" "$1"; }
  t "settings: theme copied" "py '$O/settings.json' 'assert d[\"theme\"]==\"mytheme\"'"
  t "settings: packages/extensions dropped" "py '$O/settings.json' 'assert \"npm:@pi-unipi/unipi\" not in d.get(\"packages\",[]) and not any(e.endswith(\".ts\") for e in d.get(\"extensions\",[]))'"
  t "settings: no unipi.* keys" "py '$O/settings.json' 'assert not [k for k in d if k.startswith(\"unipi.\")]'"
  t "settings: prompts path absolute" "py '$O/settings.json' 'assert all(p.startswith(\"/\") or p.startswith(\"~\") for p in d.get(\"prompts\",[\"/\"]))'"
  local E='d["extensionSettings"]'
  t "compactor rewrite" "py '$O/settings.json' 'c=$E[\"compactor\"]; assert c[\"method\"]==\"llm\" and c[\"trigger\"]==\"percent\" and c[\"thresholdPercent\"]==70 and \"files\" not in c[\"summarySections\"] and \"autoCompaction\" not in c and \"sections\" not in c'"
  t "footer mono->none, classic dropped" "py '$O/settings.json' 'f=$E[\"footer\"]; assert f[\"colorMode\"]==\"none\" and \"preset\" not in f'"
  t "info-screen bootMode" "py '$O/settings.json' 'assert $E[\"info-screen\"][\"bootMode\"]==\"off\"'"
  t "utility badge -> rename" "py '$O/settings.json' 'u=$E[\"utility\"]; assert u[\"rename\"][\"model\"]==\"openrouter/x/y\" and u[\"rename\"][\"auto\"] is False and \"badge\" not in u'"
  t "skills exposure from utility + states" "py '$O/settings.json' 's=$E[\"skills\"]; assert s[\"exposure\"][\"mode\"] in (\"all\",\"off\") and s[\"states\"][\"alpha\"]=={\"enabled\":True} and s[\"states\"][\"beta\"]=={\"enabled\":False} and s[\"states\"][\"gamma\"]=={\"enabled\":False}'"
  t "kanboard doCredits->doWrites" "py '$O/settings.json' 'k=$E[\"kanboard\"]; assert k[\"doWrites\"]==3 and \"blocking\" not in k and \"doCredits\" not in k'"
  t "permission global mode->defaultMode" "py '$O/settings.json' 'p=$E[\"permission\"]; assert p[\"defaultMode\"]==\"auto\" and \"mode\" not in p'"
  t "project permission mode" "py '$P/.oino/settings.json' 'assert d[\"extensionSettings\"][\"permission\"][\"mode\"]==\"full\"'"
  t "project core setting" "py '$P/.oino/settings.json' 'assert d[\"defaultThinkingLevel\"]==\"high\" and not d.get(\"extensions\")'"
  t "notify settings, no secret in settings" "py '$O/settings.json' 'n=$E[\"notify\"]; assert n[\"telegram\"][\"enabled\"] is True and \"botToken\" not in n[\"telegram\"] and \"workflow_end\" not in n.get(\"events\",{})'"
  t "no secret literal in settings.json" "! grep -q FAKE '$O/settings.json'"
  t "secrets.json has notify/web-api/decision-model" "py '$O/secrets.json' 'assert d[\"unipi.notify/notify/telegram.botToken\"]==\"FAKE-TG\" and d[\"unipi.web-api/web-api/providers.tavily.apiKey\"]==\"FAKE-TAVILY\" and d[\"unipi.web-api/web-api/providers.serpapi.apiKey\"]==\"FAKE-SERP\" and d[\"unipi.core/decision-model/apiKey\"]==\"FAKE-DM\" and d[\"unipi.long-horizon/long-horizon/decisionModel.apiKey\"]==\"FAKE-JUDGE\" and d[\"unipi.notify/notify/telegram.chatId\"]==\"FAKE-CHAT\"'"
  t "secrets.json mode 600" "[ \$(stat -c %a '$O/secrets.json') = 600 ]"
  t "auth: openrouter+anthropic+mygw kept, antigravity/orphan dropped" "py '$O/auth.json' 'assert \"openrouter\" in d and \"anthropic\" in d and \"mygw\" in d and \"antigravity\" not in d and \"orphan\" not in d'"
  t "auth mode 600" "[ \$(stat -c %a '$O/auth.json') = 600 ]"
  t "models: mygw kept, gemcli dropped" "py '$O/models.json' 'p=d[\"providers\"]; assert \"mygw\" in p and \"gemcli\" not in p and \"openrouter\" in p'"
  t "trust merged" "py '$O/trust.json' 'assert d[\"$P\"] is True'"
  t "keybindings" "[ -f '$O/keybindings.json' ]"
  t "theme" "[ -f '$O/themes/mytheme.json' ]"
  t "prompt renamed" "grep -q '/plan first, then /memory-search auth and /fusion. Not /unipi:modelx' '$O/prompts/review.md'"
  t "skill copied+renamed" "grep -q 'Use /goal to finish, /settings to configure' '$O/skills/my-skill/SKILL.md'"
  t "AGENTS.md renamed" "grep -q '/settings, then /kanboard-add' '$O/AGENTS.md'"
  t "project AGENTS.md renamed" "grep -q '/kanboard and /compact-then' '$P/AGENTS.md'"
  t "sessions copied" "[ \$(find '$O/sessions' -name '*.jsonl' | wc -l) -ge 2 ]"
  t "session history untouched" "grep -rq '/unipi:plan hello' '$O/sessions'"
  t "memory copied, db skipped" "[ -f '$S/unipi.memory/global/memory/proj/decision/use_rust.md' ] && [ ! -e '$S/unipi.memory/global/memory/proj/memory.db' ]"
  t "skill vault" "[ -f '$O/skill-vault/vaulted/SKILL.md' ]"
  t "subagent profile renamed, .bak skipped" "grep -q 'reports with /memory' '$S/unipi.subagents/global/profiles/helper.md' && [ ! -e '$S/unipi.subagents/global/profiles/helper.md.bak' ]"
  t "kanboard tasks transformed" "[ -f '$S/unipi.kanboard/projects/proj-abc123/tasks/PRO-1.md' ] && ! grep -Eq '^(strategy|plan):|owner: system|mode: direct' '$S/unipi.kanboard/projects/proj-abc123/tasks/PRO-1.md'"
  t "kanboard task body renamed" "grep -q 'Do it with /goal.' '$S/unipi.kanboard/projects/proj-abc123/tasks/PRO-1.md'"
  if command -v oino-kanboard >/dev/null; then
    t "kanboard canonical format" "! OINO_KANBOARD_HOME='$S/unipi.kanboard' oino-kanboard --project proj-abc123 validate 2>&1 | grep -q 'canonical'"
  fi
  t "kanboard daemon settings" "py '$S/unipi.kanboard/settings.json' 'assert d[\"piCommand\"]==[\"oino\"] and \"agentCommand\" not in d and d[\"summaryModel\"]'"
  t "kanboard queues not copied" "[ ! -e '$S/unipi.kanboard/projects/proj-abc123/queues' ]"
  t "ralph files" "[ -f '$P/.oino/ralph/task.md' ] && [ -f '$P/.oino/ralph/chores.md' ]"
  t "goal paused, parseable (generation added)" "py '$S/unipi.long-horizon/workspace/$ws/goal.json' 'g=d[\"goal\"]; assert g[\"status\"]==\"paused\" and \"lease\" not in g and g[\"generation\"]==0 and g[\"objective\"]==\"Ship v2\"'"
  t "fusion preset" "[ -f '$S/unipi.fusion/global/preset.json' ]"
  t "mcp servers merged" "py '$S/unipi.mcp/global/mcp.json' 'assert set(d[\"mcpServers\"])>={\"fs\",\"git\"}'"
  t "mcp enabled states" "py '$S/unipi.mcp/global/config.json' 'assert d[\"servers\"][\"fs\"][\"enabled\"] is False'"
  t "hints" "[ -f '$S/unipi.core/global/hints.json' ]"
  t "input-shortcuts stash" "py '$S/unipi.input-shortcuts/workspace/$ws/input-shortcuts.json' 'assert d[\"stash\"]'"
  t "images" "[ -f '$S/unipi.utility/global/images/a.png' ]"
  t "no setting points into ~/.unipi" "! grep -q '\.unipi' '$O/settings.json'"
  t "backup exists" "ls -d '$H'/.oino.pre-unipi-migrate-* "
  t "sources untouched (pi settings)" "grep -q 'npm:@pi-unipi/unipi' '$PI/settings.json' && grep -q '/unipi:plan' '$PI/prompts/review.md'"
  t "no cron/app-host migrated" "[ ! -e '$O/cron' ] && ! grep -q cron '$O/settings.json'"
  [ "$fail" = 0 ] && echo "ALL CHECKS PASSED" || { echo "SOME CHECKS FAILED"; exit 1; }
}

case "$cmd" in
  make) make_fixture ;;
  check) check_fixture ;;
  *) echo "usage: $0 make|check <dir>" >&2; exit 2 ;;
esac
