"""Метрики таблиц. Копия логики gen/cmp_tables.py + TEDS-вариант.
TEDS здесь = 1 - d / max(|A|,|B|), где A,B - деревья table->tr->td (узлы: 1 + строки + ячейки),
d - ОГРАНИЧЕННОЕ (3-уровневое) редакционное расстояние: строки выравниваются DP (вставка/удаление строки = 1 + число ячеек),
внутри пары строк ячейки выравниваются DP (вставка/удаление ячейки = 1; замена = 1, если colspan/rowspan различаются,
иначе нормализованный Левенштейн текстов). Это не полный Zhang-Shasha (не допускает "всплытия" ячеек при удалении tr),
но по смыслу PubTabNet-TEDS (с текстом в ячейках, без thead/tbody). Для объёмных таблиц (>60000 пар ячеек) считается со срезом.
"""
import json, glob, os, re, sys, statistics
from html.parser import HTMLParser
from collections import Counter
base = os.environ.get('SPIKE_DIR', '.') + '/'
norm = lambda s: re.sub(r'\s+', ' ', s).strip().lower()
try:
    from rapidfuzz.distance import Levenshtein as _L
    def lev(a, b): return _L.distance(a, b)
except Exception:
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

class P(HTMLParser):
    def __init__(s):
        super().__init__(); s.tables = []; s.t = None; s.r = None; s.c = None; s.depth = 0
    def handle_starttag(s, tag, a):
        a = dict(a)
        if tag == 'table': s.t = []
        elif tag == 'tr' and s.t is not None: s.r = []
        elif tag in ('td', 'th') and s.r is not None:
            s.c = [int(a.get('colspan', 1) or 1), int(a.get('rowspan', 1) or 1), '']
    def handle_data(s, d):
        if s.c is not None: s.c[2] += d
    def handle_endtag(s, tag):
        if tag in ('td', 'th') and s.c is not None: s.r.append(s.c); s.c = None
        elif tag == 'tr' and s.r is not None: s.t.append(s.r); s.r = None
        elif tag == 'table' and s.t is not None: s.tables.append(s.t); s.t = None

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
    if a[0] != b[0] or a[1] != b[1]:
        return 1.0
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
    if sum(len(r) for r in a) * sum(len(r) for r in b) > 400000:
        return None
    n, m = len(a), len(b)
    prev = [0.0]
    for j in range(1, m + 1): prev.append(prev[-1] + 1 + len(b[j - 1]))
    for i in range(1, n + 1):
        cur = [prev[0] + 1 + len(a[i - 1])]
        for j in range(1, m + 1):
            cur.append(min(prev[j] + 1 + len(a[i - 1]), cur[j - 1] + 1 + len(b[j - 1]),
                           prev[j - 1] + rowdist(a[i - 1], b[j - 1]) + 0))
        prev = cur
    return 1 - prev[m] / max(na, nb)

def load_gt():
    out = {}
    for g in sorted(glob.glob(base + 'samples/odl-ground-truth/*.md')):
        n = os.path.basename(g)[:-3]
        p = P(); p.feed(open(g).read()); out[n] = p.tables
    return out
_GT = None
def evaluate(d, detail=False):
    global _GT
    if _GT is None: _GT = load_gt()
    res = []; fp = 0; fp_files = 0; ext_total = 0; excess = 0
    for n, gt in _GT.items():
        try: ex = json.load(open('%s/%s.json' % (d, n)))
        except Exception: ex = []
        ex = [[[[c['cs'], c['rs'], c['t']] for c in r] for r in t['rows']] for t in ex]
        ext_total += len(ex)
        if not gt:
            fp += len(ex); fp_files += 1 if ex else 0; continue
        used = set()
        for t in gt:
            best = None
            for i, e in enumerate(ex):
                if i in used: continue
                s = f1(cells(t), cells(e))
                if best is None or s > best[0]: best = (s, i)
            if best and best[0] > 0.3:
                used.add(best[1]); e = ex[best[1]]
                tv = teds(t, e)
                res.append(dict(file=n, found=True, f1=best[0], shape=shape(t) == shape(e), grid=gridsim(t, e),
                                teds=tv if tv is not None else 0, gt_shape=shape(t), ex_shape=shape(e)))
            else:
                res.append(dict(file=n, found=False, f1=0, shape=False, grid=0, teds=0, gt_shape=shape(t), ex_shape=None))
        excess += len(ex) - len(used)
    N = len(res); F = [r for r in res if r['found']]
    m = dict(found=len(F), N=N, shape=sum(r['shape'] for r in F),
             grid_found=statistics.mean(r['grid'] for r in F) if F else 0,
             grid_all=statistics.mean(r['grid'] for r in res),
             teds_found=statistics.mean(r['teds'] for r in F) if F else 0,
             teds_all=statistics.mean(r['teds'] for r in res),
             f1_found=statistics.mean(r['f1'] for r in F) if F else 0,
             fp_tables=fp, fp_files=fp_files, excess_in_gt_files=excess, ext_total=ext_total)
    return (m, res) if detail else m
if __name__ == '__main__':
    m = evaluate(sys.argv[1]); print(json.dumps(m))
