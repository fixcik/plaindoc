"""Ячейки по планарному разбиению линий (lattice с объединением ячеек).
Использование: python3 cells_user.py GEOM_JSON OUT_JSON
Вход: дамп pdf-spike-segs (words + paths + segs). Выход: [{page, rows:[[{t,cs,rs}]]}], ячейки объединённых
областей записываются один раз в левом верхнем углу (как в HTML)."""
import json, sys, bisect, importlib.util, os

B = os.environ.get('SPIKE_DIR', '.') + '/'
spec = importlib.util.spec_from_file_location('mu2', B + 'render/mine_user2.py')
mu2 = importlib.util.module_from_spec(spec); spec.loader.exec_module(mu2)

TOL = 2.0


def cluster(vals, tol=TOL):
    return mu2.cluster(vals, tol)


def merge_intervals(iv):
    iv = sorted(iv); out = []
    for a, b in iv:
        if out and a <= out[-1][1] + 1.0: out[-1][1] = max(out[-1][1], b)
        else: out.append([a, b])
    return out


def covered(intervals, a, b, tol=2.5):
    """Отрезок [a,b] покрыт линиями (с допуском)."""
    for s, e in intervals:
        if s <= a + tol and e >= b - tol: return True
    return False


class DSU:
    def __init__(s, n): s.p = list(range(n))
    def f(s, i):
        while s.p[i] != i: s.p[i] = s.p[s.p[i]]; i = s.p[i]
        return i
    def u(s, a, b): s.p[s.f(a)] = s.f(b)


def lattice(ws, hs, vs, region):
    x0, y0, x1, y1 = region[:4]
    hs = [h for h in hs if y0 - 3 <= h[2] <= y1 + 3 and h[1] >= x0 - 3 and h[0] <= x1 + 3]
    vs = [v for v in vs if x0 - 3 <= v[2] <= x1 + 3 and v[1] >= y0 - 3 and v[0] <= y1 + 3]
    xs = cluster([v[2] for v in vs] + [x0, x1]); ys = cluster([h[2] for h in hs] + [y0, y1])
    nc, nr = len(xs) - 1, len(ys) - 1
    if nc < 1 or nr < 1: return None

    def near(c, vals, tol=TOL * 1.5): return [i for i, v in enumerate(vals) if abs(v - c) <= tol]

    # покрытие вертикальных границ: для каждой x-кластеры — объединение интервалов по y
    vcov = {}; hcov = {}
    for i, x in enumerate(xs):
        vcov[i] = merge_intervals([[v[0], v[1]] for v in vs if abs(v[2] - x) <= TOL * 1.5])
    for j, y in enumerate(ys):
        hcov[j] = merge_intervals([[h[0], h[1]] for h in hs if abs(h[2] - y) <= TOL * 1.5])

    d = DSU(nc * nr); idx = lambda i, j: j * nc + i
    for j in range(nr):
        for i in range(nc):
            if i + 1 < nc and not covered(vcov[i + 1], ys[j], ys[j + 1]): d.u(idx(i, j), idx(i + 1, j))
            if j + 1 < nr and not covered(hcov[j + 1], xs[i], xs[i + 1]): d.u(idx(i, j), idx(i, j + 1))

    comps = {}
    for j in range(nr):
        for i in range(nc): comps.setdefault(d.f(idx(i, j)), []).append((i, j))
    cell_of = {}; cells = {}
    for k, members in comps.items():
        i0 = min(m[0] for m in members); i1 = max(m[0] for m in members)
        j0 = min(m[1] for m in members); j1 = max(m[1] for m in members)
        cells[k] = dict(i0=i0, j0=j0, cs=i1 - i0 + 1, rs=j1 - j0 + 1, words=[],
                        rect=(len(members) == (i1 - i0 + 1) * (j1 - j0 + 1)))
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
        rows[c['j0']].append({'t': text(c['words']), 'cs': c['cs'], 'rs': c['rs'], 'col': c['i0']})
    rows = [r for r in rows]
    return dict(xs=xs, ys=ys, rows=rows, nonrect=sum(1 for c in cells.values() if not c['rect']))


def main(src, dst):
    pages = json.load(open(src))['pages']; res = []
    for pi, p in enumerate(pages):
        ws = mu2.mk_words(p); hs, vs, rects = mu2.mk_prims(p)
        regs, _ = mu2.regions(hs, vs, rects)
        regs = [r for r in regs if len(r[4]) >= 8]
        for r in sorted(regs, key=lambda r: -(r[2] - r[0]) * (r[3] - r[1]))[:3]:
            t = lattice(ws, hs, vs, r)
            if not t: continue
            res.append({'page': pi, 'bbox': [round(v, 1) for v in r[:4]], 'rows': t['rows'], 'nonrect': t['nonrect'],
                        'ncols': len(t['xs']) - 1, 'nrows': len(t['ys']) - 1})
    json.dump(res, open(dst, 'w'), ensure_ascii=False)
    return res


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
