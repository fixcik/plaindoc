"""Прототип детектора таблиц (копия render/mine_corpus_seg.py, режимы L,T, ROWFILL=0, TOCF=1) + признаки
каждого кандидата + планарная структура того же региона (для гибрида).
Использование: python3 -I proto.py GEOM_DIR OUT_DIR
Выход: OUT_DIR/<doc>.json = [{page, kind:'L'|'T', bbox:[x0,top,x1,bot], rows:[[{t,cs,rs}]], feat:{...},
        planar:{rows, nonrect, ncols, nrows, hcov, vcov} | null}]
Координаты: top-origin (top = H - y - h), как внутри прототипа. Ничего не фильтруется сверх базового прототипа:
фильтры применяются потом (filters.py) по признакам."""
import json, glob, os, sys, statistics, math, re, bisect
from collections import defaultdict

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
            if st:
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

def text_lines(ws):
    ws = sorted(ws, key=lambda w: (w['cy'], w['x0']))
    lines = []
    for w in ws:
        if lines and abs(lines[-1]['cy'] - w['cy']) <= 0.5 * max(w['fs'], 1):
            l = lines[-1]; l['w'].append(w); l['cy'] = (l['cy'] * (len(l['w']) - 1) + w['cy']) / len(l['w'])
        else: lines.append(dict(cy=w['cy'], w=[w]))
    for l in lines: l['w'].sort(key=lambda w: w['x0'])
    return lines

def valleys(ws, gmin, tol_cnt):
    xs = sorted(set([w['x0'] for w in ws] + [w['x1'] for w in ws]))
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

def row_bounds_from_text(lines, colb, y0, y1):
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

ROWFILL = False
TOCF = True
def finalize(g):
    g = [r for r in g if any(c.strip() for c in r)]
    if len(g) < 2 or not g: return None
    nc = len(g[0]); keep = [j for j in range(nc) if any(r[j].strip() for r in g)]
    if len(keep) < 2: return None
    g = [[r[j] for j in keep] for r in g]
    if TOCF and len(g[0]) == 2:
        last = [r[1].strip() for r in g if r[1].strip()]
        if last and sum(1 for x in last if re.fullmatch(r'[0-9ivxlc\-–]+', x.lower())) / len(last) > 0.8: return None
    filled = sum(1 for r in g for c in r if c.strip()); tot = len(g) * len(g[0])
    if filled / tot < 0.35: return None
    return [[(c, 1, 1) for c in r] for r in g]

def detect_lined(ws, hs, vs, rects, page_w):
    regs, boxes = regions(hs, vs, rects)
    out = []
    for x0, y0, x1, y1, comp in regs:
        rw = [w for w in ws if x0 - 2 <= w['cx'] <= x1 + 2 and y0 - 2 <= w['cy'] <= y1 + 2]
        if len(rw) < 4: continue
        W, Hh = x1 - x0, y1 - y0
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
        nvl = len(colb); nhl = len(rowb)
        if len(colb) < 3:
            v = valleys(rw, max(4.0, 0.7 * fs), max(0, int(0.06 * len(lines))))
            colb = [x0] + [c for c in v if x0 + 5 < c < x1 - 5] + [x1]
        else:
            colb = sorted(set([x0] + colb + [x1])) if colb[0] > x0 + 8 else colb
            if colb[-1] < x1 - 8: colb.append(x1)
        if len(rowb) >= 3:
            rb = sorted(rowb)
            if rb[0] > y0 + 8: rb = [y0] + rb
            if rb[-1] < y1 - 8: rb = rb + [y1]
            rowb = rb
        else:
            rowb = None
        if rowb is None or len(rowb) < 3:
            rowb = row_bounds_from_text(lines, colb, y0, y1)
        g = build_cells(rw, colb, rowb)
        t = finalize(g)
        if t: out.append(dict(rows=t, bbox=(x0, y0, x1, y1), kind='L', words=rw, colb=colb, rowb=rowb, nvl=nvl, nhl=nhl))
    return out

def detect_text(ws, page_w, used_boxes):
    ws = [w for w in ws if not any(b[0] - 1 <= w['cx'] <= b[2] + 1 and b[1] - 1 <= w['cy'] <= b[3] + 1 for b in used_boxes)]
    lines = text_lines(ws)
    if len(lines) < 3: return []
    for l in lines:
        segs = []; fs = statistics.median(w['fs'] for w in l['w'])
        for w in l['w']:
            if segs and w['x0'] - segs[-1]['x1'] < 1.0 * fs: segs[-1].update(x1=w['x1'], ws=segs[-1]['ws'] + [w])
            else: segs.append(dict(x0=w['x0'], x1=w['x1'], ws=[w]))
        l['segs'] = segs; l['fs'] = fs
    cand = [len(l['segs']) >= 2 and all(len(s['ws']) <= 5 for s in l['segs']) for l in lines]
    out = []; i = 0
    while i < len(lines):
        if not cand[i]: i += 1; continue
        j = i
        while j + 1 < len(lines) and (cand[j + 1] or (j + 2 < len(lines) and cand[j + 2] and lines[j + 1]['cy'] - lines[j]['cy'] < 2.2 * lines[j]['fs'])) and lines[j + 1]['cy'] - lines[j]['cy'] < 3.0 * lines[j]['fs']: j += 1
        blk = lines[i:j + 1]; i = j + 1
        if sum(1 for l in blk if len(l['segs']) >= 2) < 3: continue
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
        txt = [c[0] for r in t for c in r if c[0].strip()]
        if statistics.mean(len(x.split()) for x in txt) > 6: continue
        bb = (bx0, min(w['top'] for w in bw), bx1, max(w['bot'] for w in bw))
        out.append(dict(rows=t, bbox=bb, kind='T', words=bw, colb=colb, rowb=rowb, nvl=0, nhl=0))
    return out

# ---------- планарное разбиение (копия render/cells_user.py, lattice) ----------
PTOL = 2.0
def merge_intervals(iv):
    iv = sorted(iv); out = []
    for a, b in iv:
        if out and a <= out[-1][1] + 1.0: out[-1][1] = max(out[-1][1], b)
        else: out.append([a, b])
    return out
def covered(intervals, a, b, tol=2.5):
    for s, e in intervals:
        if s <= a + tol and e >= b - tol: return True
    return False
def lattice(ws, hs, vs, region):
    x0, y0, x1, y1 = region[:4]
    hs = [h for h in hs if y0 - 3 <= h[2] <= y1 + 3 and h[1] >= x0 - 3 and h[0] <= x1 + 3]
    vs = [v for v in vs if x0 - 3 <= v[2] <= x1 + 3 and v[1] >= y0 - 3 and v[0] <= y1 + 3]
    xs = cluster([v[2] for v in vs] + [x0, x1], PTOL); ys = cluster([h[2] for h in hs] + [y0, y1], PTOL)
    nc, nr = len(xs) - 1, len(ys) - 1
    if nc < 1 or nr < 1: return None
    vcov = {}; hcov = {}
    for i, x in enumerate(xs): vcov[i] = merge_intervals([[v[0], v[1]] for v in vs if abs(v[2] - x) <= PTOL * 1.5])
    for j, y in enumerate(ys): hcov[j] = merge_intervals([[h[0], h[1]] for h in hs if abs(h[2] - y) <= PTOL * 1.5])
    d = DSU(nc * nr); idx = lambda i, j: j * nc + i
    nedge = 0; ncov = 0
    for j in range(nr):
        for i in range(nc):
            if i + 1 < nc:
                nedge += 1
                if not covered(vcov[i + 1], ys[j], ys[j + 1]): d.u(idx(i, j), idx(i + 1, j))
                else: ncov += 1
            if j + 1 < nr:
                nedge += 1
                if not covered(hcov[j + 1], xs[i], xs[i + 1]): d.u(idx(i, j), idx(i, j + 1))
                else: ncov += 1
    comps = {}
    for j in range(nr):
        for i in range(nc): comps.setdefault(d.f(idx(i, j)), []).append((i, j))
    cell_of = {}; cells = {}
    for k, members in comps.items():
        i0 = min(m[0] for m in members); i1 = max(m[0] for m in members)
        j0 = min(m[1] for m in members); j1 = max(m[1] for m in members)
        cells[k] = dict(i0=i0, j0=j0, cs=i1 - i0 + 1, rs=j1 - j0 + 1, words=[], rect=(len(members) == (i1 - i0 + 1) * (j1 - j0 + 1)))
        for m in members: cell_of[m] = k
    for w in ws:
        cx, cy = w['cx'], w['cy']
        if not (x0 - 2 <= cx <= x1 + 2 and y0 - 2 <= cy <= y1 + 2): continue
        i = min(max(bisect.bisect_right(xs, cx) - 1, 0), nc - 1)
        j = min(max(bisect.bisect_right(ys, cy) - 1, 0), nr - 1)
        cells[cell_of[(i, j)]]['words'].append(w)
    def text(words):
        words = sorted(words, key=lambda w: (round(w['cy'] / 4), w['x0']))
        lines = []; cur = []; last = None
        for w in words:
            if last is not None and abs(w['cy'] - last) > 4: lines.append(cur); cur = []
            cur.append(w['t']); last = w['cy']
        if cur: lines.append(cur)
        return ' '.join(' '.join(l) for l in lines)
    rows = [[] for _ in range(nr)]
    for c in sorted(cells.values(), key=lambda c: (c['j0'], c['i0'])):
        rows[c['j0']].append({'t': text(c['words']), 'cs': c['cs'], 'rs': c['rs']})
    # внешняя рамка: доля периметра, покрытая линиями
    per = 0.0
    per += sum(min(e, x1) - max(s, x0) for s, e in hcov[0] if e > x0 and s < x1) + sum(min(e, x1) - max(s, x0) for s, e in hcov[nr] if e > x0 and s < x1)
    per += sum(min(e, y1) - max(s, y0) for s, e in vcov[0] if e > y0 and s < y1) + sum(min(e, y1) - max(s, y0) for s, e in vcov[nc] if e > y0 and s < y1)
    per /= max(2 * (x1 - x0) + 2 * (y1 - y0), 1e-6)
    return dict(rows=rows, nonrect=sum(1 for c in cells.values() if not c['rect']), ncols=nc, nrows=nr,
                ncells=len(cells), edge_cov=ncov / max(nedge, 1), frame=min(per, 1.0))

# ---------- признаки ----------
NUM = re.compile(r'^[\(\[]?[-+−–]?[$€£¥]?\d[\d,.\s]*%?[\)\]]?[*†‡a-z]?$')
def is_num(s):
    s = s.strip()
    return bool(s) and bool(NUM.match(s))

def features(t, page, ws_all, hs, vs, rects, paths_raw):
    rows = t['rows']; x0, y0, x1, y1 = t['bbox']; W = max(x1 - x0, 1e-6); Hh = max(y1 - y0, 1e-6)
    cells = [c[0] for r in rows for c in r]; ne = [c for c in cells if c.strip()]
    nr = len(rows); nc = max(sum(c[1] if len(c) > 1 else 1 for c in r) for r in rows)
    nc = max(nc, 1)
    rows = [list(r) + [('', 1, 1)] * (nc - len(r)) for r in rows]  # ragged (planar) -> прямоугольник
    wc = [len(c.split()) for c in ne]; chars = [len(c) for c in ne]
    rw = t['words']
    f = dict(nrows=nr, ncols=nc, ncells=nr * nc, fill=len(ne) / max(nr * nc, 1),
             mean_words=statistics.mean(wc) if wc else 0, max_words=max(wc) if wc else 0,
             mean_chars=statistics.mean(chars) if chars else 0,
             num_frac=sum(1 for c in ne if is_num(c)) / max(len(ne), 1),
             short_frac=sum(1 for w in wc if w <= 3) / max(len(wc), 1),
             nw=len(rw), w_frac=W / page['w'], h_frac=Hh / page['h'], area_frac=W * Hh / (page['w'] * page['h']),
             y0_frac=y0 / page['h'], y1_frac=y1 / page['h'], kind=t['kind'], nvl=t['nvl'], nhl=t['nhl'])
    # числовые колонки
    numcols = 0
    for j in range(nc):
        col = [r[j][0] for r in rows if r[j][0].strip()]
        if col and sum(is_num(c) for c in col) / len(col) >= 0.6: numcols += 1
    f['numcols'] = numcols; f['numcol_frac'] = numcols / nc
    # строки с <=1 непустой ячейкой
    f['sparse_rows'] = sum(1 for r in rows if sum(1 for c in r if c[0].strip()) <= 1) / nr
    # текстовые строки региона
    lines = text_lines(rw)
    f['nlines'] = len(lines)
    lw = [(max(w['x1'] for w in l['w']) - min(w['x0'] for w in l['w'])) / W for l in lines]
    f['line_w_mean'] = statistics.mean(lw) if lw else 0
    f['line_words_mean'] = statistics.mean(len(l['w']) for l in lines) if lines else 0
    fs = [w['fs'] for w in rw]; f['fs_med'] = statistics.median(fs) if fs else 0
    pfs = statistics.median(w['fs'] for w in ws_all) if ws_all else f['fs_med']
    f['fs_ratio'] = f['fs_med'] / pfs if pfs else 1
    f['fs_std'] = statistics.pstdev(fs) if len(fs) > 1 else 0
    # линии и графика в регионе
    inh = [(a, b, y) for a, b, y in hs if y0 - 3 <= y <= y1 + 3 and b > x0 - 3 and a < x1 + 3]
    inv = [(a, b, x) for a, b, x in vs if x0 - 3 <= x <= x1 + 3 and b > y0 - 3 and a < y1 + 3]
    f['n_h'] = len(cluster([y for a, b, y in inh], 2.0)); f['n_v'] = len(cluster([x for a, b, x in inv], 2.0))
    f['h_len'] = sum(min(b, x1) - max(a, x0) for a, b, y in inh) / (W * max(nr + 1, 1))
    f['v_len'] = sum(min(b, y1) - max(a, y0) for a, b, x in inv) / (Hh * max(nc + 1, 1))
    # векторная графика: все пути (сырые) внутри региона, кроме длинных линий
    H = page['h']; np_ = 0; nsmall = 0; fill_area = 0.0; nfill = 0
    for x, y, w, h, st, fi, sw in paths_raw:
        top = H - (y + h)
        if x + w < x0 - 3 or x > x1 + 3 or top + h < y0 - 3 or top > y1 + 3: continue
        np_ += 1
        if w < 8 and h < 8: nsmall += 1
        if fi and w > 2.5 and h > 2.5:
            nfill += 1
            fill_area += (min(x + w, x1) - max(x, x0)) * (min(top + h, y1) - max(top, y0))
    nseg = sum(1 for x1_, y1_, x2_, y2_, sw in page.get('segs', [])
               if x0 - 3 <= min(x1_, x2_) and max(x1_, x2_) <= x1 + 3 and y0 - 3 <= H - max(y1_, y2_) and H - min(y1_, y2_) <= y1 + 3)
    f['n_paths'] = np_; f['n_small_paths'] = nsmall; f['n_fill'] = nfill; f['fill_area'] = min(fill_area / (W * Hh), 1.0); f['n_segs'] = nseg
    f['paths_per_word'] = (np_ + nseg) / max(len(rw), 1)
    # диагональные/короткие сегменты (диаграммы): сегменты не горизонтальные и не вертикальные
    ndiag = sum(1 for x1_, y1_, x2_, y2_, sw in page.get('segs', [])
                if x0 - 3 <= min(x1_, x2_) and max(x1_, x2_) <= x1 + 3 and y0 - 3 <= H - max(y1_, y2_) and H - min(y1_, y2_) <= y1 + 3
                and abs(x1_ - x2_) > 1 and abs(y1_ - y2_) > 1)
    f['n_diag'] = ndiag
    # регулярность строк/колонок
    rb = t['rowb']; cb = t['colb']
    rh = [b - a for a, b in zip(rb, rb[1:])]; cw = [b - a for a, b in zip(cb, cb[1:])]
    f['row_h_cv'] = statistics.pstdev(rh) / statistics.mean(rh) if len(rh) > 1 and statistics.mean(rh) else 0
    f['col_w_cv'] = statistics.pstdev(cw) / statistics.mean(cw) if len(cw) > 1 and statistics.mean(cw) else 0
    f['row_h_med'] = statistics.median(rh) if rh else 0
    f['row_h_fs'] = f['row_h_med'] / f['fs_med'] if f['fs_med'] else 0
    # выравнивание: для каждой колонки std левых краёв ячеек / ширина колонки
    al = []
    for j in range(min(nc, len(cb) - 1)):
        xs = []
        for r in range(min(nr, len(rb) - 1)):
            cws = [w for w in rw if cb[j] <= w['cx'] < cb[j + 1] and rb[r] <= w['cy'] < rb[r + 1]]
            if cws: xs.append(min(w['x0'] for w in cws))
        if len(xs) > 1 and cw[j] > 0: al.append(statistics.pstdev(xs) / cw[j])
    f['align_cv'] = statistics.mean(al) if al else 0
    # слова на странице вне региона (для контекста) и доля слов страницы в регионе
    f['page_words'] = len(ws_all); f['word_share'] = len(rw) / max(len(ws_all), 1)
    # первая колонка: доля длинных ячеек (TOC/списки)
    first = [r[0][0] for r in rows if r[0][0].strip()]
    f['first_col_words'] = statistics.mean(len(c.split()) for c in first) if first else 0
    last = [r[-1][0] for r in rows if r[-1][0].strip()]
    f['last_col_num'] = sum(is_num(c) for c in last) / max(len(last), 1)
    # заголовочная строка: все ячейки первой строки непустые и не числа
    f['header_like'] = int(all(c[0].strip() and not is_num(c[0]) for c in rows[0]))
    # доля ячеек с полной рамкой (4 линии) — грубо через покрытие границ
    return f

def run(geom_dir, outdir, modes='L,T', planar=True):
    os.makedirs(outdir, exist_ok=True)
    for fpath in sorted(glob.glob(os.path.join(geom_dir, '*.json'))):
        n = os.path.basename(fpath)[:-5]
        try: pages = json.load(open(fpath))['pages']
        except Exception: continue
        res = []
        for pi, p in enumerate(pages):
            ws = mk_words(p); hs, vs, rects = mk_prims(p)
            ts = []; used = []
            if 'L' in modes:
                ts += detect_lined(ws, hs, vs, rects, p['w'])
                regs, _ = regions(hs, vs, rects); used = [r[:4] for r in regs]
            if 'T' in modes: ts += detect_text(ws, p['w'], used)
            for t in ts:
                f = features(t, p, ws, hs, vs, rects, p['paths'])
                pl = None
                if planar and t['kind'] == 'L':
                    L = lattice(ws, hs, vs, t['bbox'])
                    if L: pl = L
                res.append({'page': pi, 'kind': t['kind'], 'bbox': [round(v, 1) for v in t['bbox']],
                            'rows': [[{'t': c[0], 'cs': c[1], 'rs': c[2]} for c in r] for r in t['rows']],
                            'feat': {k: (round(v, 4) if isinstance(v, float) else v) for k, v in f.items()}, 'planar': pl})
        json.dump(res, open(os.path.join(outdir, n + '.json'), 'w'), ensure_ascii=False)

if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else 'L,T')
