"""Разбор пропусков: тип линий у эталонных таблиц (по bbox эталона и дампу геометрии) и found/TEDS по типам.
python3 -I miss_analysis.py GT.json GEOM_DIR PRED_DIR [excl]"""
import json, sys, os, collections, statistics
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import metric2 as M, proto as P
gt = json.load(open(sys.argv[1])); geom = sys.argv[2]; pr = M.load_preds(sys.argv[3])
gtf = (lambda t: not t.get('excl_det')) if len(sys.argv) > 4 else None
m, res, fps = M.evaluate(gt, pr, gt_filter=gtf)
cache = {}
def page(doc, pi):
    if doc not in cache:
        try: cache[doc] = json.load(open(os.path.join(geom, doc + '.json')))['pages']
        except Exception: cache[doc] = []
    pg = cache[doc]
    return pg[pi] if pi < len(pg) else None
def linetype(doc, t):
    bb = t.get('bbox')
    if not bb: return 'nobbox'
    p = page(doc, t['page'])
    if not p: return 'nopage'
    hs, vs, rects = P.mk_prims(p)
    x0, y0, x1, y1 = bb; W = x1 - x0; H = y1 - y0
    nh = len(P.cluster([y for a, b, y in hs if y0 - 3 <= y <= y1 + 3 and min(b, x1) - max(a, x0) >= 0.5 * W], 2.0))
    nv = len(P.cluster([x for a, b, x in vs if x0 - 3 <= x <= x1 + 3 and min(b, y1) - max(a, y0) >= 0.5 * H], 2.0))
    ws = [w for w in P.mk_words(p) if x0 <= w['cx'] <= x1 and y0 <= w['cy'] <= y1]
    if not ws: return 'no_text(raster?)'
    if nv >= 2 and nh >= 2: return 'grid'
    if nh >= 2: return 'hlines_only'
    if nh == 1 or nv == 1: return 'one_line'
    return 'no_lines'
by = collections.defaultdict(list)
for r in res:
    by[linetype(r['doc'], r['gt'])].append(r)
print('%-18s %5s %6s %6s %7s %7s' % ('тип линий', 'N', 'found', 'rate', 'teds', 'shape'))
for k, v in sorted(by.items(), key=lambda kv: -len(kv[1])):
    F = [r for r in v if r['found']]
    print('%-18s %5d %6d %6.2f %7.3f %7.2f' % (k, len(v), len(F), len(F) / len(v), statistics.mean(r['teds'] for r in v), sum(r['shape'] for r in v) / len(v)))
# размер пропущенных
miss = [r for r in res if not r['found']]
print('пропущено', len(miss), 'из них строк<=3:', sum(1 for r in miss if r['gt_shape'][0] <= 3), 'колонок<=2:', sum(1 for r in miss if r['gt_shape'][1] <= 2))
# excess: сколько лишних выводов на страницах с таблицами
print('excess', m['excess'])
