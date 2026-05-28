#!/usr/bin/env sh
set -eu

expected='docs/architecture.md
docs/auth.md
docs/built-in-extensions.md
docs/command.md
docs/configurations.md
docs/dev/extension-dev.md
docs/dev/theme.md
docs/dev/tui.md
docs/extension.md
docs/keymap.md
docs/update.md'

actual=$(find docs -type f -name '*.md' | sort)
if [ "$actual" != "$expected" ]; then
  printf 'unexpected docs set\nexpected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
  exit 1
fi

term_provider_ui="$(printf 'deep\\163eek[[:space:]]+tui')"
term_legacy_catalog="j""code"
term_codex="codex""-like"
term_claude_code="claude[[:space:]]+code"
term_agents_file="AGENTS""\\.md"
term_claude_file="CLAUDE""\\.md"
stale_pattern="$term_provider_ui|$term_legacy_catalog|$term_codex|$term_claude_code|$term_agents_file|$term_claude_file"
if grep -RniE "$stale_pattern" README.md docs .github AGENT.md 2>/dev/null; then
  printf 'stale external/reference wording found in public docs/workflows\n' >&2
  exit 1
fi

if ! grep -q 'scripts/install.sh' README.md || ! grep -q 'scripts/install.ps1' README.md; then
  printf 'README must mention scripts/install.sh and scripts/install.ps1 for local install testing\n' >&2
  exit 1
fi

python3 - <<'PY'
import pathlib
import re
import sys

files = [pathlib.Path("README.md"), *sorted(pathlib.Path("docs").rglob("*.md"))]
link_re = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
errors = []
for path in files:
    text = path.read_text()
    for match in link_re.finditer(text):
        target = match.group(1).split("#", 1)[0].strip()
        if not target or re.match(r"[a-zA-Z][a-zA-Z0-9+.-]*:", target):
            continue
        target_path = (path.parent / target).resolve()
        if not target_path.exists():
            errors.append(f"{path}:{text[:match.start()].count(chr(10)) + 1}: broken link {target}")
if errors:
    print("\n".join(errors), file=sys.stderr)
    sys.exit(1)

try:
    import yaml  # type: ignore
except Exception:
    yaml = None
if yaml is not None:
    for workflow in pathlib.Path(".github/workflows").glob("*.yml"):
        yaml.safe_load(workflow.read_text())
PY

sh -n scripts/install.sh
sh -n scripts/install-interactive.sh
sh -n scripts/install-smoke.sh
sh -n scripts/docs-check.sh
bash -n scripts/install-all-builtins.sh
if command -v pwsh >/dev/null 2>&1; then
  pwsh -NoProfile -Command '$errors = $null; $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content -Raw scripts/install.ps1), [ref]$errors); if ($errors) { $errors | Format-List; exit 1 }'
elif command -v powershell >/dev/null 2>&1; then
  powershell -NoProfile -Command '$errors = $null; $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content -Raw scripts/install.ps1), [ref]$errors); if ($errors) { $errors | Format-List; exit 1 }'
fi
printf 'docs check passed\n'
