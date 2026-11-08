"""Прототип собственного детектора таблиц поверх words + table-primitives (дамп pdf-spike-diag dump).
Режимы: lines/bands (по ограничивающим линиям/заливкам) и text (выравнивание колонок без линий).
Использование: python3 mine.py OUTDIR [MODES]  (MODES: например L,T  — по умолчанию L,T)"""
import json, glob, os, sys, statistics, math
from collections import defaultdict
B = os.environ.get('SPIKE_DIR', '.') + '/'

def mk_words(p):
    H = p['h']; out = []
    for x, y, w, h, t, fs in p['words']:
        out.append(dict(x0=x, x1=x + w, top=H - (y + h), bot=H - y, t=t, fs=fs or h, cx=x + w / 2, cy=H - y - h / 2))
    return out

def mk_prims(p):
    H = p['h']; hs = []; vs = []; rects = []
    for x, y, w, h, st, fi, sw in p['paths']:
        top = H - (y + h)
        if h <= 2.5 and w > 5: hs.append((x, x + w, top + h / 2))
        elif w <= 2.5 and h > 5: vs.append((top, top + h, x + w / 2))
        else:
            rects.append((x, top, x + w, top + h, st, fi))
            if st:  # обведённый прямоугольник = 4 линии
                hs += [(x, x + w, top), (x, x + w, top + h)]; vs += [(top, top + h, x), (top, top + h, x + w)]
    for x1, y1, x2, y2, sw in p.get('segs', []):
        if abs(y1 - y2) <= 2.5 and abs(x1 - x2) > 5: hs.append((min(x1, x2), max(x1, x2), H - (y1 + y2) / 2))
        elif abs(x1 - x2) <= 2.5 and abs(y1 - y2) > 5: vs.append((H - max(y1, y2), H - min(y1, y2), (x1 + x2) / 2))
    return hs, vs, rects

def cluster(vals, tol):
    vals = sorted(vals); out = []
    for v in vals:
        if out and v - out[-1][-1] <= tol: out[-1].append(v)
        else: out.append([v])
    return [sum(c) / len(c) for c in out]

class DSU:
    def __init__(s, n): s.p = list(range(n))
    def f(s, i):
        while s.p[i] != i: s.p[i] = s.p[s.p[i]]; i = s.p[i]
        return i
    def u(s, a, b): s.p[s.f(a)] = s.f(b)

def regions(hs, vs, rects, tol=3.0):
    """Связные компоненты примитивов (линии + заливки), ограничивающие рамки."""
    boxes = [(a, y - .5, b, y + .5) for a, b, y in hs] + [(x, a, x + .5, b) for a, b, x in vs]
    boxes += [(x0, t, x1, b) for x0, t, x1, b, st, fi in rects if (x1 - x0) < 560 and (b - t) < 700]
    n = len(boxes); d = DSU(n)
    order = sorted(range(n), key=lambda i: boxes[i][0])
    for ii, i in enumerate(order):
        a = boxes[i]
        for j in order[ii + 1:]:
            b = boxes[j]
            if b[0] > a[2] + tol: break
            if b[1] <= a[3] + tol and b[3] >= a[1] - tol: d.u(i, j)
    comps = defaultdict(list)
    for i in range(n): comps[d.f(i)].append(i)
    res = []
    for c in comps.values():
        x0 = min(boxes[i][0] for i in c); y0 = min(boxes[i][1] for i in c)
        x1 = max(boxes[i][2] for i in c); y1 = max(boxes[i][3] for i in c)
        if len(c) >= 3 and x1 - x0 > 60 and y1 - y0 > 20: res.append((x0, y0, x1, y1, c))
    return res, boxes

def text_lines(ws, ytol=None):
    ws = sorted(ws, key=lambda w: (w['cy'], w['x0']))
    lines = []
    for w in ws:
        if lines and abs(lines[-1]['cy'] - w['cy']) <= 0.5 * max(w['fs'], 1):
            l = lines[-1]; l['w'].append(w); l['cy'] = (l['cy'] * (len(l['w']) - 1) + w['cy']) / len(l['w'])
        else: lines.append(dict(cy=w['cy'], w=[w]))
    for l in lines: l['w'].sort(key=lambda w: w['x0'])
    return lines

def valleys(ws, gmin, tol_cnt):
    """Границы колонок: x, где пересекается <= tol_cnt слов, и пустой зазор >= gmin."""
    ev = sorted([(w['x0'], 1) for w in ws] + [(w['x1'], -1) for w in ws], key=lambda e: (e[0], e[1]))
    xs = sorted(set([w['x0'] for w in ws] + [w['x1'] for w in ws]))
    # покрытие на интервалах между соседними краями
    cov = []
    for a, b in zip(xs, xs[1:]):
        c = sum(1 for w in ws if w['x0'] < b - 1e-6 and w['x1'] > a + 1e-6)
        cov.append((a, b, c))
    gaps = []; cur = None
    for a, b, c in cov:
        if c <= tol_cnt:
            if cur and abs(cur[1] - a) < 1e-6: cur[1] = b
            else:
                cur = [a, b]; gaps.append(cur)
    return [(a + b) / 2 for a, b in gaps if b - a >= gmin]

def build_cells(ws, colb, rowb):
    """colb, rowb - границы (отсортированные, длины ncol+1, nrow+1). Возврат сетка текстов."""
    nc, nr = len(colb) - 1, len(rowb) - 1
    cell = [[[] for _ in range(nc)] for _ in range(nr)]
    for w in ws:
        ci = next((i for i in range(nc) if colb[i] <= w['cx'] < colb[i + 1]), None)
        ri = next((i for i in range(nr) if rowb[i] <= w['cy'] < rowb[i + 1]), None)
        if ci is not None and ri is not None: cell[ri][ci].append(w)
    g = []
    for r in cell:
        row = []
        for c in r:
            c.sort(key=lambda w: (round(w['cy'] / 3), w['x0']))
            row.append(' '.join(w['t'] for w in c))
        g.append(row)
    return g

def cover_h(hs, y, a, b, tol=2.0):
    """Доля [a,b] по x, покрытая горизонтальными линиями на высоте y."""
    segs = sorted((max(a, s0), min(b, s1)) for s0, s1, yy in hs if abs(yy - y) <= tol and s1 > a and s0 < b)
    tot = 0; cur = a
    for s0, s1 in segs:
        if s1 > cur: tot += s1 - max(cur, s0); cur = s1
    return tot / max(b - a, 1e-6)
def cover_v(vs, x, a, b, tol=2.0):
    segs = sorted((max(a, s0), min(b, s1)) for s0, s1, xx in vs if abs(xx - x) <= tol and s1 > a and s0 < b)
    tot = 0; cur = a
    for s0, s1 in segs:
        if s1 > cur: tot += s1 - max(cur, s0); cur = s1
    return tot / max(b - a, 1e-6)

def detect_lined(ws, hs, vs, rects, page_w):
    """Таблицы, ограниченные линиями/заливками. Возврат список таблиц [[(text,cs,rs)...]...]."""
    regs, boxes = regions(hs, vs, rects)
    out = []
    for x0, y0, x1, y1, comp in regs:
        rw = [w for w in ws if x0 - 2 <= w['cx'] <= x1 + 2 and y0 - 2 <= w['cy'] <= y1 + 2]
        if len(rw) < 4: continue
        W, Hh = x1 - x0, y1 - y0
        # границы строк: горизонтальные линии/края заливок
        yl = []; yf = []
        for a, b, y in hs:
            if x0 - 3 <= a and b <= x1 + 3 and y0 - 3 <= y <= y1 + 3: yl.append((y, b - a))
        for rx0, rt, rx1, rb, st, fi in rects:
            if fi and rx0 >= x0 - 3 and rx1 <= x1 + 3 and rt >= y0 - 3 and rb <= y1 + 3 and (rx1 - rx0) > 20 and (rb - rt) > 4:
                yf.append((rt, rx1 - rx0)); yf.append((rb, rx1 - rx0))
        def ycl_f(ys):
            ycl = []
            for y, l in sorted(ys):
                if ycl and y - ycl[-1][0] <= 2.0: ycl[-1][1] += l; ycl[-1][2] += 1; ycl[-1][0] = (ycl[-1][0] + y) / 2
                else: ycl.append([y, l, 1])
            return [y for y, l, n in ycl if l >= 0.35 * W]
        rowb = ycl_f(yl)
        if len(rowb) < 3 and ROWFILL: rowb = ycl_f(yl + yf)
        xs = []
        for a, b, x in vs:
            if x0 - 3 <= x <= x1 + 3 and a >= y0 - 3 and b <= y1 + 3: xs.append((x, b - a))
        xcl = []
        for x, l in sorted(xs):
            if xcl and x - xcl[-1][0] <= 2.0: xcl[-1][1] += l; xcl[-1][0] = (xcl[-1][0] + x) / 2
            else: xcl.append([x, l])
        colb = [x for x, l in xcl if l >= 0.3 * Hh]
        lines = text_lines(rw)
        fs = statistics.median(w['fs'] for w in rw)
        if len(colb) < 3:
            # колонки по пустым вертикальным "долинам" текста
            v = valleys(rw, max(4.0, 0.7 * fs), max(0, int(0.06 * len(lines))))
            colb = [x0] + [c for c in v if x0 + 5 < c < x1 - 5] + [x1]
        else:
            colb = sorted(set([x0] + colb + [x1])) if colb[0] > x0 + 8 else colb
            if colb[-1] < x1 - 8: colb.append(x1)
        # строки
        if len(rowb) >= 3:
            rb = sorted(rowb)
            if rb[0] > y0 + 8: rb = [y0] + rb
            if rb[-1] < y1 - 8: rb = rb + [y1]
            rowb = rb
        else:
            rowb = None
        if rowb is None or len(rowb) < 3:
            rowb = row_bounds_from_text(lines, colb, y0, y1)
        # если слов вне полос (заголовок над таблицей без линии) - игнорируем
        g = build_cells(rw, colb, rowb)
        t = finalize(g)
        if t: out.append(t)
    return out

def row_bounds_from_text(lines, colb, y0, y1):
    """Строки таблицы без горизонтальных линий: новая строка начинается, когда в первой колонке есть текст
    (продолжение = строка, где первая колонка пуста, а зазор небольшой)."""
    starts = []
    fc = colb[1]
    for i, l in enumerate(lines):
        first = [w for w in l['w'] if w['cx'] < fc]
        gap = l['cy'] - lines[i - 1]['cy'] if i else 99
        lh = max(w['fs'] for w in l['w'])
        if i == 0 or first or gap > 2.2 * lh: starts.append(i)
    b = [y0]
    for s in starts[1:]: b.append((lines[s - 1]['cy'] + lines[s]['cy']) / 2)
    b.append(y1 + 1)
    return b

ROWFILL = True
TOCF = False
def finalize(g):
    g = [r for r in g if any(c.strip() for c in r)]
    if len(g) < 2 or not g: return None
    nc = len(g[0]); keep = [j for j in range(nc) if any(r[j].strip() for r in g)]
    if len(keep) < 2: return None
    g = [[r[j] for j in keep] for r in g]
    if TOCF and len(g[0]) == 2:
        import re
        last = [r[1].strip() for r in g if r[1].strip()]
        if last and sum(1 for x in last if re.fullmatch(r'[0-9ivxlc\-–]+', x.lower())) / len(last) > 0.8: return None
    filled = sum(1 for r in g for c in r if c.strip()); tot = len(g) * len(g[0])
    if filled / tot < 0.35: return None
    return [[(c, 1, 1) for c in r] for r in g]

def detect_text(ws, page_w, used_boxes):
    """Таблицы без линий: серии строк, где >=3 из ячеек выравнены по колонкам."""
    ws = [w for w in ws if not any(b[0] - 1 <= w['cx'] <= b[2] + 1 and b[1] - 1 <= w['cy'] <= b[3] + 1 for b in used_boxes)]
    lines = text_lines(ws)
    if len(lines) < 3: return []
    # сегменты строки: слова с зазором < 1.0*fs - одна ячейка
    for l in lines:
        segs = []; fs = statistics.median(w['fs'] for w in l['w'])
        for w in l['w']:
            if segs and w['x0'] - segs[-1]['x1'] < 1.0 * fs: segs[-1].update(x1=w['x1'], ws=segs[-1]['ws'] + [w])
            else: segs.append(dict(x0=w['x0'], x1=w['x1'], ws=[w]))
        l['segs'] = segs; l['fs'] = fs
    # кандидаты: строки с >=2 сегментами и короткими ячейками (<=5 слов)
    cand = [len(l['segs']) >= 2 and all(len(s['ws']) <= 5 for s in l['segs']) for l in lines]
    out = []; i = 0
    while i < len(lines):
        if not cand[i]: i += 1; continue
        j = i
        while j + 1 < len(lines) and (cand[j + 1] or (j + 2 < len(lines) and cand[j + 2] and lines[j + 1]['cy'] - lines[j]['cy'] < 2.2 * lines[j]['fs'])) and lines[j + 1]['cy'] - lines[j]['cy'] < 3.0 * lines[j]['fs']: j += 1
        blk = lines[i:j + 1]; i = j + 1
        if sum(1 for l in blk if len(l['segs']) >= 2) < 3: continue
        # колонки: долины по всем словам блока
        bw = [w for l in blk for w in l['w']]
        fs = statistics.median(l['fs'] for l in blk)
        v = valleys(bw, max(4.0, 0.7 * fs), max(0, int(0.1 * len(blk))))
        bx0 = min(w['x0'] for w in bw); bx1 = max(w['x1'] for w in bw)
        colb = [bx0 - 1] + [c for c in v if bx0 + 3 < c < bx1 - 3] + [bx1 + 1]
        if len(colb) < 3 or len(colb) - 1 > 15: continue
        by0 = blk[0]['cy'] - 20; by1 = blk[-1]['cy'] + 20
        rowb = row_bounds_from_text(blk, colb, by0, by1)
        g = build_cells(bw, colb, rowb)
        t = finalize(g)
        if not t or len(t) < 3: continue
        # отсев "прозы": средняя длина ячейки
        txt = [c[0] for r in t for c in r if c[0].strip()]
        if statistics.mean(len(x.split()) for x in txt) > 6: continue
        out.append(t)
    return out

def run(outdir, modes):
    os.makedirs(outdir, exist_ok=True)
    for f in sorted(glob.glob(B + 'user-run/geom2/*.json')):
        n = os.path.basename(f)[:-5]; pages = json.load(open(f))['pages']; res = []
        for pi, p in enumerate(pages):
            ws = mk_words(p); hs, vs, rects = mk_prims(p)
            ts = []
            used = []
            if 'L' in modes:
                lt = detect_lined(ws, hs, vs, rects, p['w'])
                ts += lt
                regs, _ = regions(hs, vs, rects); used = [r[:4] for r in regs]
            if 'T' in modes: ts += detect_text(ws, p['w'], used)
            for t in ts: res.append({'page': pi, 'rows': [[{'t': c[0], 'cs': c[1], 'rs': c[2]} for c in r] for r in t]})
        json.dump(res, open('%s/%s.json' % (outdir, n), 'w'))

if __name__ == '__main__':
    import os as _o
    ROWFILL = _o.environ.get('ROWFILL', '0') == '1'; TOCF = _o.environ.get('TOCF', '1') == '1'
    run(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else 'L,T')
