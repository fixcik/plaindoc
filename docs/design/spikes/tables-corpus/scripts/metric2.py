"""Метрика таблиц, обобщение tbl/metric.py на многостраничные корпуса и единый формат эталона (gt_convert.py).
Отличия от tbl/metric.py (оригинал не меняется):
  * сопоставление эталон<->вывод только внутри одной страницы (оригинал — внутри файла; для одностраничного ODL то же самое);
  * ложные срабатывания считаются по страницам без эталонных таблиц (neg pages) и по документам без таблиц (neg docs);
    "лишние" выводы на страницах с таблицами (excess) считаются отдельно и в FP не входят (как в оригинале);
  * TEDS — та же упрощённая 3-уровневая DP-метрика (НЕ Zhang-Shasha), код скопирован дословно;
  * select(rec) -> rows|None позволяет применять фильтры/гибрид к записям proto.py без перезапуска детектора;
  * бутстрап по документам (bootstrap()).
"""
import json, os, re, statistics, random, glob
from collections import Counter
norm = lambda s: re.sub(r'\s+', ' ', s).strip().lower()
def lev(a, b):
    if a == b: return 0
    if len(a) < len(b): a, b = b, a
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]
def shape(t): return (len(t), max((sum(c[0] for c in r) for r in t), default=0))
def cells(t): return [norm(c[2]) for r in t for c in r if norm(c[2])]
def f1(a, b):
    if not a or not b: return 0.0
    ca, cb = Counter(a), Counter(b); i = sum((ca & cb).values())
    p = i / len(b); r = i / len(a)
    return 2 * p * r / (p + r) if p + r else 0.0
def grid(t):
    g = {}; occ = set()
    for ri, r in enumerate(t):
        ci = 0
        for c in r:
            while (ri, ci) in occ: ci += 1
            for dr in range(min(c[1], 50)):
                for dc in range(min(c[0], 50)):
                    occ.add((ri + dr, ci + dc)); g[(ri + dr, ci + dc)] = norm(c[2])
            ci += c[0]
    return g
def gridsim(a, b):
    ga, gb = grid(a), grid(b); keys = set(ga) | set(gb)
    return sum(1 for k in keys if ga.get(k, '') == gb.get(k, '')) / len(keys) if keys else 0
def ccost(a, b):
    if a[0] != b[0] or a[1] != b[1]: return 1.0
    x, y = norm(a[2]), norm(b[2])
    if x == y: return 0.0
    m = max(len(x), len(y))
    return min(1.0, lev(x[:2000], y[:2000]) / m)
def rowdist(r1, r2):
    n, m = len(r1), len(r2)
    prev = [float(j) for j in range(m + 1)]
    for i in range(1, n + 1):
        cur = [float(i)]
        for j in range(1, m + 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + ccost(r1[i - 1], r2[j - 1])))
        prev = cur
    return prev[m]
def teds(a, b):
    na = 1 + len(a) + sum(len(r) for r in a); nb = 1 + len(b) + sum(len(r) for r in b)
    if sum(len(r) for r in a) * sum(len(r) for r in b) > 400000: return None
    n, m = len(a), len(b)
    prev = [0.0]
    for j in range(1, m + 1): prev.append(prev[-1] + 1 + len(b[j - 1]))
    for i in range(1, n + 1):
        cur = [prev[0] + 1 + len(a[i - 1])]
        for j in range(1, m + 1):
            cur.append(min(prev[j] + 1 + len(a[i - 1]), cur[j - 1] + 1 + len(b[j - 1]), prev[j - 1] + rowdist(a[i - 1], b[j - 1])))
        prev = cur
    return 1 - prev[m] / max(na, nb)
def iou(a, b):
    if not a or not b: return 0.0
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0])); iy = max(0, min(a[3], b[3]) - max(a[1], b[1]))
    i = ix * iy; u = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - i
    return i / u if u > 0 else 0.0

_MEMO = {}
def sig(rows): return (len(rows), tuple(len(r) for r in rows[:50]), sum(len(c[2]) for r in rows for c in r), sum(c[0] + c[1] for r in rows for c in r))
def teds_memo(key, a, b):
    if key not in _MEMO: _MEMO[key] = teds(a, b)
    return _MEMO[key]

def rows_of(rec): return [[[c['cs'], c['rs'], c['t']] for c in r] for r in rec['rows']]

def load_preds(d):
    out = {}
    for f in glob.glob(os.path.join(d, '*.json')):
        n = os.path.basename(f)[:-5]; recs = json.load(open(f))
        for i, r in enumerate(recs): r['_id'] = (n, i)
        out[n] = recs
    return out

def evaluate(gt, preds, select=None, gt_filter=None, docs=None):
    """gt: dict из gt_convert; preds: dict doc -> [rec]; select(rec)->rows|None; gt_filter(table)->bool.
    Возвращает (summary, table_records, fp_records)."""
    select = select or rows_of
    res = []; fps = []; excess = 0; ext_total = 0; neg_pages = 0; neg_pages_fp = 0; neg_docs = 0; neg_docs_fp = 0
    for n, g in gt.items():
        if docs is not None and n not in docs: continue
        recs = preds.get(n, [])
        sel = []
        for r in recs:
            rows = select(r)
            if rows: sel.append((r, rows))
        ext_total += len(sel)
        gts = [t for t in g['tables'] if (gt_filter is None or gt_filter(t))]
        gt_pages = {t['page'] for t in g['tables']}
        npg = g.get('pages') or (max(gt_pages | {r['page'] for r in recs}) + 1 if (gt_pages or recs) else 1)
        used = set()
        for ti, t in enumerate(gts):
            best = None
            for i, (r, rows) in enumerate(sel):
                if i in used or r['page'] != t['page']: continue
                s = f1(cells(t['rows']), cells(rows))
                if best is None or s > best[0]: best = (s, i)
            if best and best[0] > 0.3:
                used.add(best[1]); r, rows = sel[best[1]]
                tv = teds_memo((n, ti, r.get('_id'), sig(rows)), t['rows'], rows)
                res.append(dict(doc=n, page=t['page'], found=True, f1=best[0], shape=shape(t['rows']) == shape(rows), grid=gridsim(t['rows'], rows),
                                teds=tv if tv is not None else 0, gt_shape=shape(t['rows']), ex_shape=shape(rows), iou=iou(t.get('bbox'), r.get('bbox')),
                                kind=r.get('kind'), feat=r.get('feat'), gt=t, rid=r.get('_id')))
            else:
                res.append(dict(doc=n, page=t['page'], found=False, f1=0, shape=False, grid=0, teds=0, gt_shape=shape(t['rows']), ex_shape=None, iou=0, kind=None, feat=None, gt=t))
        for i, (r, rows) in enumerate(sel):
            if i in used: continue
            if r['page'] in gt_pages: excess += 1
            else: fps.append(dict(doc=n, page=r['page'], bbox=r.get('bbox'), kind=r.get('kind'), feat=r.get('feat'), rows=rows, rid=r.get('_id')))
        fp_pages = {f['page'] for f in fps if f['doc'] == n}
        neg_pages += npg - len(gt_pages); neg_pages_fp += len(fp_pages)
        if not g['tables']:
            neg_docs += 1; neg_docs_fp += 1 if sel else 0
    N = len(res); F = [r for r in res if r['found']]
    m = dict(N=N, found=len(F), shape=sum(r['shape'] for r in F),
             grid_found=statistics.mean(r['grid'] for r in F) if F else 0, grid_all=statistics.mean(r['grid'] for r in res) if res else 0,
             teds_found=statistics.mean(r['teds'] for r in F) if F else 0, teds_all=statistics.mean(r['teds'] for r in res) if res else 0,
             fp_tables=len(fps), neg_pages=neg_pages, neg_pages_fp=neg_pages_fp, fp_page_rate=neg_pages_fp / neg_pages if neg_pages else 0,
             neg_docs=neg_docs, neg_docs_fp=neg_docs_fp, fp_doc_rate=neg_docs_fp / neg_docs if neg_docs else 0,
             excess=excess, ext_total=ext_total)
    return m, res, fps

def bootstrap(gt, preds, select=None, gt_filter=None, docs=None, B=500, seed=0):
    """Бутстрап по документам: перцентили 2.5/97.5 для found-доли, teds_all, shape-доли, fp_page_rate, fp_doc_rate."""
    m, res, fps = evaluate(gt, preds, select, gt_filter, docs)
    # агрегаты по документам
    per = {}
    dl = [d for d in gt if docs is None or d in docs]
    for d in dl: per[d] = dict(N=0, found=0, teds=0.0, shape=0, negp=0, negp_fp=0, negd=0, negd_fp=0)
    for r in res:
        p = per[r['doc']]; p['N'] += 1; p['found'] += r['found']; p['teds'] += r['teds']; p['shape'] += r['shape']
    fp_pages = {}
    for f in fps: fp_pages.setdefault(f['doc'], set()).add(f['page'])
    for d in dl:
        g = gt[d]; gt_pages = {t['page'] for t in g['tables']}
        npg = g.get('pages') or 1
        per[d]['negp'] = npg - len(gt_pages); per[d]['negp_fp'] = len(fp_pages.get(d, ()))
        if not g['tables']:
            per[d]['negd'] = 1; per[d]['negd_fp'] = 1 if preds.get(d) and any((select or rows_of)(r) for r in preds[d]) else 0
    rng = random.Random(seed); stats = {k: [] for k in ('found', 'teds_all', 'shape', 'fp_page_rate', 'fp_doc_rate')}
    for _ in range(B):
        s = [per[rng.choice(dl)] for _ in dl]
        N = sum(p['N'] for p in s); negp = sum(p['negp'] for p in s); negd = sum(p['negd'] for p in s)
        stats['found'].append(sum(p['found'] for p in s) / N if N else 0)
        stats['teds_all'].append(sum(p['teds'] for p in s) / N if N else 0)
        stats['shape'].append(sum(p['shape'] for p in s) / N if N else 0)
        stats['fp_page_rate'].append(sum(p['negp_fp'] for p in s) / negp if negp else 0)
        stats['fp_doc_rate'].append(sum(p['negd_fp'] for p in s) / negd if negd else 0)
    ci = {}
    for k, v in stats.items():
        v.sort(); ci[k] = (round(v[int(0.025 * B)], 3), round(v[min(int(0.975 * B), B - 1)], 3))
    return m, ci

def fmt(m, ci=None):
    s = 'found %d/%d (%.2f) teds_all %.3f shape %d (%.2f) fp_tables %d fp_pages %d/%d (%.3f) fp_docs %d/%d (%.3f) excess %d' % (
        m['found'], m['N'], m['found'] / m['N'] if m['N'] else 0, m['teds_all'], m['shape'], m['shape'] / m['N'] if m['N'] else 0,
        m['fp_tables'], m['neg_pages_fp'], m['neg_pages'], m['fp_page_rate'], m['neg_docs_fp'], m['neg_docs'], m['fp_doc_rate'], m['excess'])
    if ci: s += ' | CI found %s teds %s shape %s fp_page %s fp_doc %s' % (ci['found'], ci['teds_all'], ci['shape'], ci['fp_page_rate'], ci['fp_doc_rate'])
    return s
