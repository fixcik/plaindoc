#!/usr/bin/env bash
# Fetches the ICDAR 2013 Table Competition dataset (67 PDFs + ground-truth XML) and unpacks the PDFs.
# Licence not stated on the source page: research evaluation only, never commit the data.
# usage: fetch-icdar2013.sh [DEST]   (default: $TABLES_DATA/icdar)
set -euo pipefail
URL=https://www.tamirhassan.com/html/files/icdar2013-competition-dataset-with-gt.zip
SHA256=3c005732f5f829a0c1f5c50bf92cd13cbe935e5ad717a0631ca8d7a04b0e01e9
DEST="${1:-${TABLES_DATA:?set TABLES_DATA or pass DEST}/icdar}"
mkdir -p "$DEST/pdf" "$DEST/x"
ZIP="$DEST/icdar2013.zip"
[ -s "$ZIP" ] || curl -fsSL -o "$ZIP" "$URL"
echo "$SHA256  $ZIP" | shasum -a 256 -c -
unzip -qo "$ZIP" -d "$DEST/x"
# ground truth stays in $DEST/x (input of gt_convert.py icdar); PDFs go flat into pdf/
find "$DEST/x" -name '*.pdf' -exec cp {} "$DEST/pdf/" \;
echo "icdar: $(ls "$DEST/pdf" | wc -l) pdf in $DEST/pdf"
