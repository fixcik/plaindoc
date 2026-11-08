"""Python reference for the Rust table detector (SP4): runs the spike prototype (proto.py) and the D11 selection
on pdf_oxide geometry dumps and writes, per document, the candidates with features, planar partition and the
D11 decision. Usage:
  python3 -I table_golden.py SPIKE_SCRIPTS_DIR GEOM_DIR OUT_DIR [DOC ...]          # one JSON per document
  python3 -I table_golden.py SPIKE_SCRIPTS_DIR GEOM_DIR OUT_FILE --fixture DOC:PAGE ...  # one fixture file
D11: rules chart+tiny+sparse+twocol, logit (filters_logit.json) threshold 0.10, hybrid ecov>=0.7, nonrect=0,
ncells>=4, planar rows within 0.6-1.4x of prototype rows. kv is NOT applied."""
import json, math, os, sys, glob
SPIKE = sys.argv[1]; sys.path.insert(0, SPIKE)
import proto as P
LOGIT = json.load(open(os.path.join(SPIKE, '..', 'results', 'filters_logit.json')))
LOGF = {'nrows', 'ncols', 'nlines', 'n_h', 'n_v', 'n_paths', 'n_segs', 'n_diag', 'nw', 'paths_per_word'}
def prob(r):
    v = []
    for k in LOGIT['feats'][:-1]:
        x = float(r['feat'].get(k, 0) or 0)
        v.append(math.log1p(max(x, 0)) if k in LOGF else x)
    v.append(1.0 if r['kind'] == 'T' else 0.0)
    z = [(a - m) / sd for a, m, sd in zip(v, LOGIT['mu'], LOGIT['sd'])]
    return 1 / (1 + math.exp(-max(-30, min(30, sum(a * c for a, c in zip(LOGIT['w'], z)) + LOGIT['b']))))
def reject(f, k, p):
    if (f['num_frac'] >= 0.5 and f['fill'] <= 0.55) or f['n_diag'] >= 8 or (f['fill_area'] >= 0.5 and f['line_w_mean'] <= 0.3 and f['n_v'] <= 2): return 'chart'
    if f['nrows'] * f['ncols'] <= 4 or f['nw'] < 8: return 'tiny'
    if f['sparse_rows'] >= 0.5: return 'sparse'
    if k == 'T' and f['ncols'] == 2 and f['numcols'] == 0 and f['col_w_cv'] <= 0.05 and f['line_w_mean'] >= 0.85 and f['mean_words'] >= 3.5: return 'twocol'
    if p < 0.10: return 'logit'
    return None
def planar_ok(r):
    pl = r.get('planar'); n = len(r['rows'])
    return bool(r['kind'] == 'L' and pl and pl['edge_cov'] >= 0.7 and pl['ncells'] >= 4 and pl['nonrect'] == 0 and pl['nrows'] >= 2
                and pl['ncols'] >= 2 and 0.6 * n <= pl['nrows'] <= 1.4 * n)
def page_records(p):
    ws = P.mk_words(p); hs, vs, rects = P.mk_prims(p)
    regs, _ = P.regions(hs, vs, rects); used = [r[:4] for r in regs]
    ts = P.detect_lined(ws, hs, vs, rects, p['w']) + P.detect_text(ws, p['w'], used)
    out = []
    for t in ts:
        f = P.features(t, p, ws, hs, vs, rects, p['paths'])
        pl = P.lattice(ws, hs, vs, t['bbox']) if t['kind'] == 'L' else None
        if pl: pl.pop('frame', None)
        r = {'kind': t['kind'], 'bbox': list(t['bbox']), 'rows': [[c[0] for c in row] for row in t['rows']],
             'feat': {k: (round(v, 4) if isinstance(v, float) else v) for k, v in f.items()}, 'planar': pl}
        r['p'] = prob(r); r['reject'] = reject(r['feat'], r['kind'], r['p']); r['use_planar'] = planar_ok(r)
        out.append(r)
    return out
if __name__ == '__main__':
    geom, out = sys.argv[2], sys.argv[3]
    if '--fixture' in sys.argv:
        pages = []
        for spec in sys.argv[sys.argv.index('--fixture') + 1:]:
            doc, pg = spec.rsplit(':', 1)
            p = json.load(open(os.path.join(geom, doc + '.json')))['pages'][int(pg)]
            pages.append({'doc': doc, 'page': int(pg), 'input': p, 'candidates': page_records(p)})
        os.makedirs(os.path.dirname(out) or '.', exist_ok=True)
        json.dump(pages, open(out, 'w'), ensure_ascii=False, separators=(',', ':'))
    else:
        os.makedirs(out, exist_ok=True)
        docs = sys.argv[4:] or sorted(os.path.basename(f)[:-5] for f in glob.glob(os.path.join(geom, '*.json')))
        for d in docs:
            try: pages = json.load(open(os.path.join(geom, d + '.json')))['pages']
            except Exception: continue
            json.dump([page_records(p) for p in pages], open(os.path.join(out, d + '.json'), 'w'), ensure_ascii=False)
