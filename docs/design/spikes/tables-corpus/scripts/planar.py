"""Планарный построитель как самостоятельный детектор (копия логики render/cells_user.py main: регионы с >=8
примитивами, до 3 самых больших на страницу). Использование: python3 -I planar.py GEOM_DIR OUT_DIR
Выход в формате proto.py (kind='P', feat — те же признаки, planar=None)."""
import json, glob, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import proto as P

def run(geom_dir, outdir):
    os.makedirs(outdir, exist_ok=True)
    for fpath in sorted(glob.glob(os.path.join(geom_dir, '*.json'))):
        n = os.path.basename(fpath)[:-5]
        try: pages = json.load(open(fpath))['pages']
        except Exception: continue
        res = []
        for pi, p in enumerate(pages):
            ws = P.mk_words(p); hs, vs, rects = P.mk_prims(p)
            regs, _ = P.regions(hs, vs, rects)
            regs = [r for r in regs if len(r[4]) >= 8]
            for r in sorted(regs, key=lambda r: -(r[2] - r[0]) * (r[3] - r[1]))[:3]:
                L = P.lattice(ws, hs, vs, r)
                if not L: continue
                rows = [[(c['t'], c['cs'], c['rs']) for c in row] for row in L['rows']]
                rw = [w for w in ws if r[0] - 2 <= w['cx'] <= r[2] + 2 and r[1] - 2 <= w['cy'] <= r[3] + 2]
                # границы для признаков: сетка планарного разбиения
                xs = P.cluster([v[2] for v in vs if r[0] - 3 <= v[2] <= r[2] + 3] + [r[0], r[2]], P.PTOL)
                ys = P.cluster([h[2] for h in hs if r[1] - 3 <= h[2] <= r[3] + 3] + [r[1], r[3]], P.PTOL)
                t = dict(rows=rows, bbox=r[:4], kind='P', words=rw, colb=xs, rowb=ys, nvl=len(xs) - 2, nhl=len(ys) - 2)
                f = P.features(t, p, ws, hs, vs, rects, p['paths'])
                f.update(nonrect=L['nonrect'], edge_cov=round(L['edge_cov'], 4), frame=round(L['frame'], 4), pcells=L['ncells'])
                res.append({'page': pi, 'kind': 'P', 'bbox': [round(v, 1) for v in r[:4]], 'rows': L['rows'],
                            'feat': {k: (round(v, 4) if isinstance(v, float) else v) for k, v in f.items()}, 'planar': None})
        json.dump(res, open(os.path.join(outdir, n + '.json'), 'w'), ensure_ascii=False)

if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
