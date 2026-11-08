#!/usr/bin/env bash
# Explicit download of the pinned OCR models (URLs and sha256 in crates/ocr/model-manifest.txt,
# shared with crates/ocr/build.rs).
# Usage: scripts/fetch-ocr-models.sh [DIR]
# DIR defaults to crates/ocr/models (git-ignored), which feature `bundled-models` embeds at build
# time. Usually not needed: a plain `cargo build` downloads the same files itself into the user
# models dir (see docs/building.md). Use it for offline builds (fetch once, then build with
# PLAINDOC_NO_FETCH=1 or PLAINDOC_OCR_BUNDLE_DIR=DIR) and for model-less binaries at run time
# (--ocr-models DIR / $PLAINDOC_OCR_MODELS=DIR).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${1:-$ROOT/crates/ocr/models}"
if command -v sha256sum >/dev/null; then SHA=(sha256sum); else SHA=(shasum -a 256); fi
check() { [ "$("${SHA[@]}" "$2" | cut -d" " -f1)" = "$1" ]; }
fetch() {
  local id=$1 sha=$2 url=$3 f="$DIR/$1/inference.onnx"
  mkdir -p "$DIR/$id"
  if [ -f "$f" ] && check "$sha" "$f"; then echo "ok   $id"; return; fi
  curl -fsSL --retry 3 -o "$f.part" "$url"
  if ! check "$sha" "$f.part"; then rm -f "$f.part"; echo "sha256 mismatch: $id ($url)" >&2; exit 1; fi
  mv "$f.part" "$f"; echo "got  $id"
}
while read -r id sha _size url; do
  case "$id" in ''|'#'*) continue ;; esac
  fetch "$id" "$sha" "$url"
done < "$ROOT/crates/ocr/model-manifest.txt"
echo "export PLAINDOC_OCR_MODELS=$DIR"
