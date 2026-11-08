#!/usr/bin/env bash
# Fails if a tracked .rs file is longer than the limit (default 600 lines, `wc -l`).
# Usage: scripts/check-file-size.sh [LIMIT]
set -euo pipefail
LIMIT="${1:-600}"
cd "$(git rev-parse --show-toplevel)"
bad=$(git ls-files '*.rs' | while read -r f; do
  [ -f "$f" ] || continue
  n=$(wc -l < "$f"); [ "$n" -gt "$LIMIT" ] && echo "$n $f"
done | sort -rn || true)
if [ -n "$bad" ]; then
  echo "files longer than $LIMIT lines:" >&2
  echo "$bad" >&2
  exit 1
fi
echo "ok: no .rs file longer than $LIMIT lines"
