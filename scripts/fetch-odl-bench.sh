#!/usr/bin/env bash
# Fetches opendataloader-bench PDFs and Markdown ground truth at a pinned commit (no Git LFS).
set -euo pipefail
REV=7af1d8f4d0c09f51ea1a5c6ba5f66e993286d109
DEST="${1:-$(git rev-parse --show-toplevel)/target/odl-bench}"
if [ ! -d "$DEST/.git" ]; then
  git clone --filter=blob:none --no-checkout https://github.com/opendataloader-project/opendataloader-bench "$DEST"
fi
git -C "$DEST" sparse-checkout set pdfs ground-truth/markdown
git -C "$DEST" checkout --quiet "$REV"
echo "export ODL_BENCH_PDFS=$DEST/pdfs ODL_BENCH_GT=$DEST/ground-truth/markdown"
