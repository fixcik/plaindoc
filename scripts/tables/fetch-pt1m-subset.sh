#!/usr/bin/env bash
# Fetches a PubTables-1M subset: annotations from the first (PART+1)*250 MB of the Hugging Face stream
# (CDLA-Permissive-2.0) and the PDFs from the PMC Open Access bucket (per-article licences).
# Never commit the output (ground-truth text and articles keep their own licences).
# usage: fetch-pt1m-subset.sh [DEST] [--seed N] [--part K] [--count M]
#   default: the pinned 449 documents of lists/pt1m_docs.tsv (the D10 corpus)
#   --seed N: instead pick a fresh sample of M documents (default 300, deterministic for the seed) from
#             annotation files of part K (default 0) that are not in lists/pt1m_docs.tsv
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
HF=https://huggingface.co/datasets/bsmock/pubtables-1m/resolve/main/PubTables-1M-PDF_Annotations.tar.gz
S3=https://pmc-oa-opendata.s3.amazonaws.com
CHUNK=262144000
DEST="${TABLES_DATA:+$TABLES_DATA/pt1m}"; SEED=""; PART=0; COUNT=300
while [ $# -gt 0 ]; do
  case "$1" in
    --seed) SEED="$2"; shift 2;; --part) PART="$2"; shift 2;; --count) COUNT="$2"; shift 2;;
    *) DEST="$1"; shift;;
  esac
done
: "${DEST:?set TABLES_DATA or pass DEST}"
mkdir -p "$DEST/pdf" "$DEST/ann" "$DEST/raw"
# 1. annotation stream: a truncated gzip stream is fine, tar stops at the cut
if [ -z "$(ls "$DEST/raw" 2>/dev/null)" ]; then
  curl -fsSL -r "0-$(( (PART + 1) * CHUNK - 1 ))" "$HF" | tar -xzf - -C "$DEST/raw" 2>/dev/null || true
fi
# 2. document list
LIST="$DEST/selected.tsv"
if [ -z "$SEED" ]; then
  cut -f1 "$HERE/lists/pt1m_docs.tsv" > "$LIST.ids"
else
  python3 -I "$HERE/../../docs/design/spikes/tables-corpus/scripts/pt1m_select.py" \
    "$(dirname "$(find "$DEST/raw" -name '*_tables.json' | head -1)")" "$COUNT" "$LIST" "$SEED"
  cut -f1 "$LIST" | grep -vxFf <(cut -f1 "$HERE/lists/pt1m_docs.tsv") > "$LIST.ids" || true
fi
MISSING=0
# 3. annotations and PDFs
while read -r pmc; do
  f=$(find "$DEST/raw" -name "${pmc}_tables.json" | head -1)
  [ -n "$f" ] && cp "$f" "$DEST/ann/"
  [ -s "$DEST/pdf/$pmc.pdf" ] || curl -fs --retry 3 --max-time 120 -o "$DEST/pdf/$pmc.pdf" "$S3/$pmc.1/$pmc.1.pdf" \
    || { rm -f "$DEST/pdf/$pmc.pdf"; echo "missing: $pmc" >&2; MISSING=$((MISSING + 1)); }
done < "$LIST.ids"
# the pinned default list must be complete (the test split is positional); fresh samples may lose a few
[ -z "$SEED" ] && [ "$MISSING" -gt 0 ] && { echo "pt1m: $MISSING pinned PDFs missing" >&2; exit 1; }
# 4. manifest: checksums of what was fetched (the PMC files are not versioned, so this pins a sample)
(cd "$DEST/pdf" && shasum -a 256 *.pdf) > "$DEST/manifest.sha256"
echo "pt1m: $(ls "$DEST/pdf" | wc -l) pdf, manifest $DEST/manifest.sha256"
