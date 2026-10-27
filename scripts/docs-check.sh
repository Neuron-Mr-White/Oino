#!/usr/bin/env sh
set -eu

# Docs named by the system prompt docs section (runtime §9.4) plus the other user guides.
required='acp.md cron.md desktop.md environment-variables.md extension-api.md extensions.md install.md
keybindings.md migrate-from-unipi.md models.md packages.md prompt-templates.md providers.md rpc.md sdk.md sessions.md
settings.md skills.md themes.md tui.md unipi.md'
for doc in $required; do
  test -f "docs/$doc" || { printf 'missing docs/%s\n' "$doc" >&2; exit 1; }
done
extra=$(find docs -maxdepth 1 -type f -name '*.md' | sed 's#docs/##' | sort | while read -r f; do
  case " $(echo $required) " in *" $f "*) ;; *) printf '%s ' "$f" ;; esac
done)
if [ -n "$extra" ]; then
  printf 'unexpected user docs (add them to scripts/docs-check.sh): %s\n' "$extra" >&2
  exit 1
fi

# Pre-v1 product surface that must not come back into user docs.
stale_pattern='oino-app|extension-packages|/router setup|OmniRoute|install-all-builtins|extensions/built-in'
if grep -RniE "$stale_pattern" README.md docs/*.md .github 2>/dev/null; then
  printf 'stale pre-v1 wording found in public docs/workflows\n' >&2
  exit 1
fi

if ! grep -q 'scripts/install.sh' README.md || ! grep -q 'scripts/install.ps1' README.md; then
  printf 'README must mention scripts/install.sh and scripts/install.ps1\n' >&2
  exit 1
fi

python3 - <<'PY'
import pathlib
import re
import sys

files = [pathlib.Path("README.md"), *sorted(pathlib.Path("docs").glob("*.md")), pathlib.Path("docs/dev/v1/STATUS.md")]
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
sh -n scripts/install-smoke.sh
sh -n scripts/docs-check.sh
if command -v pwsh >/dev/null 2>&1; then
  pwsh -NoProfile -Command '$errors = $null; $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content -Raw scripts/install.ps1), [ref]$errors); if ($errors) { $errors | Format-List; exit 1 }'
elif command -v powershell >/dev/null 2>&1; then
  powershell -NoProfile -Command '$errors = $null; $null = [System.Management.Automation.PSParser]::Tokenize((Get-Content -Raw scripts/install.ps1), [ref]$errors); if ($errors) { $errors | Format-List; exit 1 }'
fi
printf 'docs check passed\n'
