#!/usr/bin/env bash
# Build a publication tree without checking out files or changing the real index.
# Usage: scripts/release/make-public-commit.sh [source-ref [public-branch [version]]]
# Refresh origin/main separately, then audit the resulting commit before any push.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
source_ref=${1:-perf-v3}
branch=${2:-public/main}
version=${3:-1.0.13}
git check-ref-format "refs/heads/$branch"
source_sha=$(git rev-parse --verify "$source_ref^{commit}")
parent=$(git rev-parse --verify 'origin/main^{commit}')
old=$(git rev-parse --verify "refs/heads/$branch" 2>/dev/null || printf '%040d' 0)
# Do not move a branch checked out in any worktree.
if git worktree list --porcelain | grep -Fxq "branch refs/heads/$branch"; then
  echo "publication branch is checked out: $branch" >&2; exit 1
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export GIT_INDEX_FILE="$tmp/index"
git read-tree "$source_sha"
git show "$source_sha:scripts/release/public-exclusions.txt" > "$tmp/exclusions"
python3 - "$tmp/exclusions" <<'PY'
import fnmatch
import subprocess
import sys
patterns = [s.strip() for s in open(sys.argv[1]) if s.strip() and not s.startswith('#')]
paths = subprocess.check_output(['git', 'ls-files', '-z']).split(b'\0')
removed = [p for p in paths if p and any(fnmatch.fnmatchcase(p.decode(), pat) for pat in patterns)]
subprocess.run(['git', 'update-index', '--force-remove', '-z', '--stdin'],
               input=b'\0'.join(removed) + b'\0', check=True)
print(f'Excluded {len(removed)} paths', file=sys.stderr)
PY
tree=$(git write-tree)
commit=$(printf 'UniPi Oino %s\n' "$version" | git commit-tree "$tree" -p "$parent")
git update-ref "refs/heads/$branch" "$commit" "$old"
printf '%s\n' "$commit"
# This creates exactly one commit above origin/main, NOT a merge of private history.
# It makes no network request and never pushes, tags, dispatches or publishes.
