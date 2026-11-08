#!/bin/bash
# Builds the synthetic corpus from generated tables only (source=generated, fixed seeds; no ruwiki/RWT text).
# Usage: make_all.sh OUTDIR   (env: CHROME=<chrome binary>, DUMPER=<pdf-spike-segs binary>)
# Produces OUTDIR/syn/{pdf,kit,ox,out}; then: python3 -I scripts/tables/synth_gt.py OUTDIR/syn gt.json
set -e
D="$(cd "$(dirname "$0")" && pwd)"; B="${1:?usage: make_all.sh OUTDIR}"
mkdir -p "$B/work" "$B/syn/kit" "$B/syn/ox"
python3 -I "$D/pool.py" "$B/work/pool.jsonl"
rm -rf "$B/work/html"; python3 -I "$D/compose.py" "$B/work/pool.jsonl" "$B/work/html" "$B/work/plan.jsonl" --generated-only
"$D/render.sh" "$B/work/html" "$B/syn/pdf" 8
cp "$B/work/plan.jsonl" "$B/syn/plan.jsonl"
# text layers: PDFKit (independent of pdf_oxide, for the ground truth) and a pdf_oxide geometry dump
[ -x "$B/pdftext" ] || swiftc -O "$D/pdftext.swift" -o "$B/pdftext"
"$B/pdftext" "$B/syn/kit" "$B"/syn/pdf/*.pdf
"${DUMPER:?set DUMPER to the pdf-spike-segs binary}" dump "$B/syn/ox" "$B"/syn/pdf/*.pdf
python3 -I "$D/gt.py" "$B/syn/plan.jsonl" "$B/syn/kit" "$B/syn/ox" "$B/syn/out"
