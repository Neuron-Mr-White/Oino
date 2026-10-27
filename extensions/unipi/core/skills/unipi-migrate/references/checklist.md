# Verification and rollback

## 1. Verification checklist

Run each check; report pass/fail per line in the final report.

1. **JSON parses**: `for f in "$OINO"/{settings,auth,models,trust,keybindings}.json "$OINO"/secrets.json; do [ -f "$f" ] && python3 -m json.tool "$f" >/dev/null && echo "ok $f"; done`
   (and every `<project>/.oino/settings.json` written).
2. **Permissions**: `stat -c '%a %n' "$OINO/auth.json" "$OINO/secrets.json"` → `600`.
3. **No secrets in settings**: no secret field key from `fields.json` appears with a literal
   value under `extensionSettings` in any `settings.json`.
4. **No dropped keys left**: `settings.json` has no `packages` entry for `@pi-unipi`, no
   `extensions` `*.ts`, no top-level `unipi.*` keys, and no legacy keys from settings-map §3.
5. **Oino starts**: `printf '%s\n' '{"type":"get_commands","id":"c"}' | timeout 60 oino --mode rpc`
   returns a `get_commands` response; no `Failed to load` / settings errors on stderr.
6. **Extension settings read back**: for each migrated namespace,
   `{"type":"_oino/get_extension_settings","id":"x","namespace":"<ns>","scope":"global"}`
   shows the migrated values; secrets show `••••••`.
7. **Models**: `oino --list-models <defaultModel>` finds the default model; every migrated
   provider with credentials lists models.
8. **Sessions**: source and target `*.jsonl` counts match (minus recorded skips); every
   copied file's first line is a `{"type":"session",…}` header whose `cwd` matches its dir
   name; tell the user `oino -r` in a project lists its sessions.
9. **Memory**: per project, `find … -name '*.md' | wc -l` matches the source.
10. **Kanboard** (when migrated): `OINO_KANBOARD_HOME="$STATE/unipi.kanboard" oino-kanboard --project <slug> validate`
    reports no format problems for every slug (pre-existing rule problems copied from unipi,
    such as dangling dependencies, are reported, not fixed) (skip if `oino-kanboard` is not installed; then
    `grep -l '^strategy:\|^plan:\|owner: system\|mode: direct' -r "$STATE/unipi.kanboard/projects"`
    must print nothing).
11. **Commands**: `grep -rn '/unipi:' <files of SKILL.md step 8>` prints nothing (or only
    `/unipi:cron`, reported as having no equivalent).
12. **Sources untouched**: `find "$PI_DIR" "$UNIPI" -newer "$MIG/discover.txt" -type f | head`
    lists only files written by other running programs (pi/unipi themselves), none by you.
13. In a fresh interactive `oino`: `/doctor`, `/memory status`, `/skills`, `/agents`,
    `/mcp-status`, `/kanboard status` — tell the user to run these.

## 2. Rollback

Everything the migration wrote is under `$OINO`, the projects' `.oino/` dirs and the files of
step 8. To roll back:

```bash
# global: restore the backup taken in phase 4 (move the migrated dir aside first)
mv "$OINO" "$OINO.migrated-$(date +%s)" && cp -a "$BACKUP" "$OINO"
# no previous Oino dir existed: mv "$OINO" "$OINO.migrated-$(date +%s)"
# projects: restore each .oino archive
rm -rf "<p>/.oino" && tar -C "<p>" -xzf "$MIG/<name>-dot-oino.tgz"   # or delete .oino if there was none
# command renames in git projects
git -C "<p>" checkout -- AGENTS.md CLAUDE.md   # only the files step 8 changed
```

Global files changed by step 8 (`$OINO/prompts`, `$OINO/skills`, …) are restored with the
backup. The pi and unipi sources were never changed, so pi + unipi keep working as before.
Keep `$MIG` (inventory, log) until the user is satisfied; it holds no secrets.
