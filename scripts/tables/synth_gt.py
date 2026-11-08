"""Synthetic corpus ground truth (corpus-synth/syn/out) -> unified format of gt_convert.py.
python3 -I synth_gt.py SYN_DIR OUT.json [--docs-from DOCLIST]
Drops bad_docs (verify.json), inner tables of nested tables (scored separately in the spike) and, per D11,
the aligned_list decoys (key-value blocks stay tables)."""
import json, os, sys, glob
B = sys.argv[1]; out = sys.argv[2]
bad = set(json.load(open(os.path.join(B, 'out', 'verify.json')))['bad_docs'])
man = [json.loads(l) for l in open(os.path.join(B, 'out', 'manifest.jsonl'))]
decoy = {m['doc']: m['decoy'] for m in man if m['kind'] == 'decoy'}
only = set(open(sys.argv[sys.argv.index('--docs-from') + 1]).read().split()) if '--docs-from' in sys.argv else None
res = {}
for f in sorted(glob.glob(os.path.join(B, 'out', 'gt', '*.json'))):
    d = os.path.basename(f)[:-5]
    if d in bad or decoy.get(d) == 'aligned_list' or (only is not None and d not in only): continue
    pages = len(json.load(open(os.path.join(B, 'ox', d + '.json')))['pages'])
    tables = [dict(page=t['page'], rows=[[[c['cs'], c['rs'], c['t']] for c in r] for r in t['rows']])
              for t in json.load(open(f)) if not t.get('nested_inner')]
    res[d] = dict(pages=pages, tables=tables)
json.dump(res, open(out, 'w'), ensure_ascii=False)
print(len(res), 'docs', sum(len(v['tables']) for v in res.values()), 'tables')
