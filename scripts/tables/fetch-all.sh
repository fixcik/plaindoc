#!/usr/bin/env bash
# Builds $TABLES_DATA = <root>/{odl,icdar,pt1m,synth}/{pdf/,gt.json} for tests/pdf_tables_eval.rs.
# env: TABLES_DATA (root), DUMPER (spike geometry dumper `pdf-spike-segs`, used for the page counts of
#      gt_convert.py), SYNTH (optional: output dir of synth/make_all.sh, adds the synthetic corpus).
# Nothing built here may be committed (licences, see docs/tables-eval.md).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
R="${TABLES_DATA:?set TABLES_DATA}"; : "${DUMPER:?set DUMPER to the pdf-spike-segs binary}"
GT="$ROOT/docs/design/spikes/tables-corpus/scripts/gt_convert.py"
mkdir -p "$R"/{odl,icdar,pt1m}
# ODL: pinned bench commit, PDFs flat, markdown ground truth
ODL="$R/odl/src"; bash "$ROOT/scripts/fetch-odl-bench.sh" "$ODL" > /dev/null
ln -sfn "$ODL/pdfs" "$R/odl/pdf"
bash "$HERE/fetch-icdar2013.sh" "$R/icdar"
bash "$HERE/fetch-pt1m-subset.sh" "$R/pt1m"
for k in odl icdar pt1m; do
  mkdir -p "$R/$k/geom"; "$DUMPER" dump "$R/$k/geom" "$R/$k"/pdf/*.pdf
done
python3 -I "$GT" odl "$ODL/ground-truth/markdown" "$R/odl/geom" "$R/odl/gt.json"
python3 -I "$GT" icdar "$R/icdar/x" "$R/icdar/geom" "$R/icdar/gt.json"
python3 -I "$GT" pt1m "$R/pt1m/ann" "$R/pt1m/geom" "$R/pt1m/gt.json"
# the split is pinned by the lists: a missing document would silently shift the odd/even test set
for k in icdar pt1m; do
  python3 -I - "$R/$k/gt.json" "$HERE/lists/${k}_docs.tsv" <<'PY'
import json, sys
got = sorted(json.load(open(sys.argv[1])))
want = sorted(l.split('\t')[0] for l in open(sys.argv[2]) if l.strip())
if got != want:
    sys.exit('gt.json differs from %s: missing %s, extra %s' % (sys.argv[2], sorted(set(want) - set(got)), sorted(set(got) - set(want))))
PY
done
if [ -n "${SYNTH:-}" ]; then
  mkdir -p "$R/synth"; ln -sfn "$SYNTH/syn/pdf" "$R/synth/pdf"
  python3 -I "$HERE/synth_gt.py" "$SYNTH/syn" "$R/synth/gt.json"
fi
echo "TABLES_DATA=$R ready"
