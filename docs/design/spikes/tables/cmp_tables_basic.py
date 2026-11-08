import json, glob, os, re, statistics
from html.parser import HTMLParser
from collections import Counter

base = os.environ.get('SPIKE_DIR', '.') + '/'
norm = lambda s: re.sub(r'\s+', ' ', s).strip().lower()


class P(HTMLParser):
    def __init__(s):
        super().__init__()
        s.tables = []; s.t = None; s.r = None; s.c = None

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


res = []; fp = 0; fp_files = 0; ext_total = 0
for g in sorted(glob.glob(base + 'samples/odl-ground-truth/*.md')):
    n = os.path.basename(g)[:-3]
    p = P(); p.feed(open(g).read()); gt = p.tables
    ex = json.load(open(base + 'out-tables/%s.json' % n))
    ex = [[[[c['cs'], c['rs'], c['t']] for c in r] for r in t['rows']] for t in ex]
    ext_total += len(ex)
    if not gt:
        fp += len(ex); fp_files += 1 if ex else 0
        continue
    used = set()
    for t in gt:
        best = None
        for i, e in enumerate(ex):
            if i in used: continue
            s = f1(cells(t), cells(e))
            if best is None or s > best[0]: best = (s, i)
        spans = any(c[0] > 1 or c[1] > 1 for r in t for c in r)
        if best and best[0] > 0.3:
            used.add(best[1]); e = ex[best[1]]
            res.append(dict(file=n, found=True, f1=best[0], shape=shape(t) == shape(e), grid=gridsim(t, e),
                            gt_shape=shape(t), ex_shape=shape(e), spans=spans))
        else:
            res.append(dict(file=n, found=False, f1=0, shape=False, grid=0, gt_shape=shape(t), ex_shape=None, spans=spans))

N = len(res); F = [r for r in res if r['found']]
print('GT tables: %d in %d files; extracted tables total %d (in files without GT tables: %d in %d files)'
      % (N, len({r['file'] for r in res}), ext_total, fp, fp_files))
print('found (cell-text F1>0.3): %d/%d = %.0f%%' % (len(F), N, 100 * len(F) / N))
print('of found: shape exact %d/%d = %.0f%%; mean cell F1 %.2f; mean positional grid similarity %.2f'
      % (sum(r['shape'] for r in F), len(F), 100 * sum(r['shape'] for r in F) / len(F),
         statistics.mean(r['f1'] for r in F), statistics.mean(r['grid'] for r in F)))
S = [r for r in res if r['spans']]; Sf = [r for r in S if r['found']]
print('GT tables with merged cells: %d, found %d, shape exact %d' % (len(S), len(Sf), sum(r['shape'] for r in Sf)))
print('end-to-end (not found = 0): mean grid similarity over all GT tables %.2f' % statistics.mean(r['grid'] for r in res))
print('worst found:')
for r in sorted(F, key=lambda r: r['grid'])[:5]:
    print('  ', r['file'], 'f1 %.2f grid %.2f gt %s ex %s' % (r['f1'], r['grid'], r['gt_shape'], r['ex_shape']))

# --- вариант: объединить все извлечённые таблицы файла, смотреть покрытие текста ячеек и фрагментацию
print('\n== покрытие текста ячеек (все извлечённые таблицы файла как одно множество) и фрагментация ==')
cov = []; frag = []; zero = 0
for g in sorted(glob.glob(base + 'samples/odl-ground-truth/*.md')):
    n = os.path.basename(g)[:-3]
    p = P(); p.feed(open(g).read()); gt = p.tables
    if not gt: continue
    ex = json.load(open(base + 'out-tables/%s.json' % n))
    ex = [[[[c['cs'], c['rs'], c['t']] for c in r] for r in t['rows']] for t in ex]
    allex = Counter(c for e in ex for c in cells(e))
    for t in gt:
        gc = Counter(cells(t)); tot = sum(gc.values())
        cov.append(sum((gc & allex).values()) / tot if tot else 0)
    frag.append((len(gt), len(ex)))
    if not ex: zero += 1
print('GT-таблиц %d: среднее покрытие текста ячеек %.2f; медиана %.2f; таблиц с покрытием >=0.8: %d'
      % (len(cov), statistics.mean(cov), statistics.median(cov), sum(1 for c in cov if c >= 0.8)))
print('файлов с GT-таблицами, где извлечено 0 таблиц: %d из %d' % (zero, len(frag)))
print('файлов, где извлечено БОЛЬШЕ таблиц, чем в эталоне (фрагментация): %d; МЕНЬШЕ: %d; поровну: %d'
      % (sum(1 for a, b in frag if b > a), sum(1 for a, b in frag if b < a), sum(1 for a, b in frag if a == b)))
