"""Фильтры ложных таблиц и гибрид (прототип-детектор + планарная структура).
python3 -I filters.py RESULTS_DIR
Ожидает в RESULTS_DIR: gt_{odl,icdar,pt1m}.json, out_{odl,icdar,pt1m}_proto/, out_*_planar/.
Разбиение: train = половина документов pt1m + половина icdar (чётные индексы в отсортированном списке),
test = остальные + весь ODL. Пишет RESULTS_DIR/filters_*.{json,tsv}."""
import json, os, sys, math, random, statistics
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import metric2 as M
R = sys.argv[1]
CORP = os.environ.get('CORP', 'odl,icdar,pt1m').split(',')
GT = {c: json.load(open(os.path.join(R, 'gt_%s.json' % c))) for c in CORP}
PR = {c: M.load_preds(os.path.join(R, 'out_%s_proto' % c)) for c in CORP}
PL = {c: M.load_preds(os.path.join(R, 'out_%s_planar' % c)) for c in CORP}
# для pt1m: таблицы, исключённые для detection, не считаем за эталон, но и страницы с ними не считаем "пустыми"
def gtf(t): return not t.get('excl_det')
def split(c):
    docs = sorted(GT[c])
    if c == 'odl': return set(), set(docs)
    return set(docs[0::2]), set(docs[1::2])
SPL = {c: split(c) for c in CORP}
TRAIN = {c: SPL[c][0] for c in CORP}; TEST = {c: SPL[c][1] for c in CORP}

# ---------- разметка кандидатов ----------
def label(c, preds):
    m, res, fps = M.evaluate(GT[c], preds, gt_filter=gtf)
    tp = {r['rid']: r for r in res if r['found']}; fp = {f['rid']: f for f in fps}
    return tp, fp
LAB = {c: label(c, PR[c]) for c in CORP}
def cands(c, docs):
    out = []
    for d in docs:
        for r in PR[c].get(d, []):
            y = 1 if r['_id'] in LAB[c][0] else (0 if r['_id'] in LAB[c][1] else None)
            if y is not None: out.append((r, y))
    return out

# ---------- признаки для модели ----------
FEATS = ['nrows', 'ncols', 'fill', 'mean_words', 'num_frac', 'numcol_frac', 'sparse_rows', 'line_w_mean', 'nlines', 'n_h', 'n_v',
         'h_len', 'v_len', 'n_paths', 'n_segs', 'n_diag', 'fill_area', 'paths_per_word', 'row_h_cv', 'align_cv', 'w_frac', 'h_frac',
         'fs_ratio', 'first_col_words', 'last_col_num', 'header_like', 'word_share', 'nw', 'short_frac', 'row_h_fs']
LOGF = {'nrows', 'ncols', 'nlines', 'n_h', 'n_v', 'n_paths', 'n_segs', 'n_diag', 'nw', 'paths_per_word'}
def vec(r):
    f = r['feat']; v = []
    for k in FEATS:
        x = float(f.get(k, 0) or 0)
        if k in LOGF: x = math.log1p(max(x, 0))
        v.append(x)
    v.append(1.0 if r['kind'] == 'T' else 0.0)
    return v

class Logit:
    def fit(s, X, y, l2=1.0, iters=3000, lr=0.05):
        n, d = len(X), len(X[0])
        s.mu = [statistics.mean(col) for col in zip(*X)]; s.sd = [statistics.pstdev(col) or 1.0 for col in zip(*X)]
        Z = [[(x - m) / sd for x, m, sd in zip(row, s.mu, s.sd)] for row in X]
        w = [0.0] * d; b = 0.0
        pos = sum(y); neg = n - pos; wp = neg / max(pos, 1)  # балансировка классов
        for it in range(iters):
            gw = [0.0] * d; gb = 0.0
            for z, t in zip(Z, y):
                p = 1 / (1 + math.exp(-max(-30, min(30, sum(a * c for a, c in zip(w, z)) + b))))
                e = (p - t) * (wp if t == 1 else 1.0)
                for j in range(d): gw[j] += e * z[j]
                gb += e
            for j in range(d): w[j] -= lr * (gw[j] / n + l2 * w[j] / n)
            b -= lr * gb / n
        s.w, s.b = w, b
    def p(s, x):
        z = [(a - m) / sd for a, m, sd in zip(x, s.mu, s.sd)]
        return 1 / (1 + math.exp(-max(-30, min(30, sum(a * c for a, c in zip(s.w, z)) + s.b))))

# ---------- ручные фильтры (True = выбросить) ----------
def rule_chart(f, k): return (f['num_frac'] >= 0.5 and f['fill'] <= 0.55) or f['n_diag'] >= 8 or (f['fill_area'] >= 0.5 and f['line_w_mean'] <= 0.3 and f['n_v'] <= 2)
def rule_prose(f, k): return k == 'T' and (f['mean_words'] >= 3.5 and f['num_frac'] < 0.2) or (k == 'T' and f['line_w_mean'] >= 0.9 and f['numcol_frac'] == 0)
def rule_tiny(f, k): return f['nrows'] * f['ncols'] <= 4 or f['nw'] < 8
def rule_toc(f, k): return f['ncols'] <= 3 and f['last_col_num'] >= 0.8 and f['numcol_frac'] <= 1 / f['ncols'] + 1e-9 and f['first_col_words'] <= 1.5 and f['mean_words'] >= 2.5
def rule_list(f, k): return k == 'T' and f['ncols'] == 2 and f['num_frac'] <= 0.55 and f['numcols'] == 1 and f['mean_words'] >= 3
def rule_frame(f, k): return f['area_frac'] >= 0.5 and f['nrows'] * f['ncols'] <= 6
def rule_sparse(f, k): return f['sparse_rows'] >= 0.5
RULES = dict(chart=rule_chart, prose=rule_prose, tiny=rule_tiny, toc=rule_toc, list=rule_list, frame=rule_frame, sparse=rule_sparse)

# ---------- гибрид ----------
def hybrid_rows(r, ecov=1.0, allow_nonrect=False, guard=True):
    pl = r.get('planar')
    # guard: планарная сетка не должна терять строки прототипа (случай ODL-45: 8 строк -> 3)
    if r['kind'] == 'L' and pl and pl['edge_cov'] >= ecov and pl['ncells'] >= 4 and (allow_nonrect or pl['nonrect'] == 0) and pl['nrows'] >= 2 and pl['ncols'] >= 2 \
            and (not guard or pl['nrows'] >= 0.6 * len(r['rows'])):
        return [[[c['cs'], c['rs'], c['t']] for c in row] for row in pl['rows']]
    return M.rows_of(r)

def make_select(model=None, thr=0.0, rules=(), hyb=None):
    def sel(r):
        f = r['feat']; k = r['kind']
        for name in rules:
            if RULES[name](f, k): return None
        if model is not None and model.p(vec(r)) < thr: return None
        return hybrid_rows(r, **hyb) if hyb is not None else M.rows_of(r)
    return sel

def run_eval(sel, docsel, B=300):
    out = {}
    for c in CORP:
        docs = docsel[c]
        if not docs: continue
        m, ci = M.bootstrap(GT[c], PR[c], select=sel, gt_filter=gtf, docs=docs, B=B)
        out[c] = (m, ci)
    return out
def summarize(ev):
    N = sum(m['N'] for m, ci in ev.values()); found = sum(m['found'] for m, ci in ev.values())
    teds = sum(m['teds_all'] * m['N'] for m, ci in ev.values()) / N if N else 0
    shape = sum(m['shape'] for m, ci in ev.values())
    negp = sum(m['neg_pages'] for m, ci in ev.values()); negpf = sum(m['neg_pages_fp'] for m, ci in ev.values())
    return dict(N=N, found=found, found_rate=found / N if N else 0, teds_all=teds, shape=shape, shape_rate=shape / N if N else 0, neg_pages=negp, neg_pages_fp=negpf, fp_page_rate=negpf / negp if negp else 0)

if __name__ == '__main__':
    log = open(os.path.join(R, 'filters_log.txt'), 'w')
    def P(*a):
        s = ' '.join(str(x) for x in a); print(s); log.write(s + '\n'); log.flush()
    for c in CORP: P(c, 'docs', len(GT[c]), 'train', len(TRAIN[c]), 'test', len(TEST[c]), 'tables', sum(1 for d in GT[c].values() for t in d['tables'] if gtf(t)))
    # базовые
    for name, sel in [('proto', None), ('hybrid_ecov1.0', make_select(hyb=dict(ecov=1.0))), ('hybrid_ecov0.9', make_select(hyb=dict(ecov=0.9))),
                      ('hybrid_ecov0.8', make_select(hyb=dict(ecov=0.8))), ('hybrid_ecov1.0_nonrect', make_select(hyb=dict(ecov=1.0, allow_nonrect=True)))]:
        for c in CORP:
            m, ci = M.bootstrap(GT[c], PR[c], select=sel, gt_filter=gtf, B=300); P('%-22s %-6s %s' % (name, c, M.fmt(m, ci)))
    for c in CORP:
        m, ci = M.bootstrap(GT[c], PL[c], gt_filter=gtf, B=300); P('%-22s %-6s %s' % ('planar', c, M.fmt(m, ci)))
    # ручные фильтры по одному: сколько FP убирает / сколько TP теряет, train и test
    P('\n== ручные фильтры (кандидаты: TP = сопоставлен с эталоном, FP = на странице без эталонных таблиц)')
    for part, D in [('train', TRAIN), ('test', TEST)]:
        allc = {c: cands(c, D[c]) for c in CORP}
        P('[%s] кандидаты: ' % part + ' '.join('%s TP=%d FP=%d' % (c, sum(y for r, y in allc[c]), sum(1 - y for r, y in allc[c])) for c in CORP))
        for name, rule in RULES.items():
            s = []
            for c in CORP:
                tp_lost = sum(1 for r, y in allc[c] if y == 1 and rule(r['feat'], r['kind']))
                fp_rm = sum(1 for r, y in allc[c] if y == 0 and rule(r['feat'], r['kind']))
                s.append('%s -FP %d -TP %d' % (c, fp_rm, tp_lost))
            P('  %-7s %s' % (name, ' | '.join(s)))
    # логистическая модель: обучение на train, кривая порогов
    Xtr = []; ytr = []
    for c in CORP:
        for r, y in cands(c, TRAIN[c]): Xtr.append(vec(r)); ytr.append(y)
    model = Logit(); model.fit(Xtr, ytr)
    P('\n== логистическая модель: веса (стандартизованные признаки)')
    for k, w in sorted(zip(FEATS + ['kind_T'], model.w), key=lambda x: -abs(x[1])): P('  %-16s %+.3f' % (k, w))
    json.dump(dict(feats=FEATS + ['kind_T'], w=model.w, b=model.b, mu=model.mu, sd=model.sd), open(os.path.join(R, 'filters_logit.json'), 'w'))
    P('\n== кривая порогов (train / test): порог -> found, teds_all, shape, fp_page_rate (все корпуса части), fp_doc_rate ODL (только test)')
    curve = []
    for thr in [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]:
        row = dict(thr=thr)
        for part, D in [('train', TRAIN), ('test', TEST)]:
            ev = run_eval(make_select(model, thr), D, B=200); sm = summarize(ev)
            row[part] = dict(sm, per={c: dict(m=ev[c][0], ci=ev[c][1]) for c in ev})
            odl = ev.get('odl')
            P('thr %.1f %-5s found %d/%d (%.3f) teds %.3f shape %.3f fp_page %d/%d (%.3f)%s' % (
                thr, part, sm['found'], sm['N'], sm['found_rate'], sm['teds_all'], sm['shape_rate'], sm['neg_pages_fp'], sm['neg_pages'], sm['fp_page_rate'],
                (' | ODL fp_doc %d/%d (%.3f) CI %s found %d/%d CI %s' % (odl[0]['neg_docs_fp'], odl[0]['neg_docs'], odl[0]['fp_doc_rate'], odl[1]['fp_doc_rate'], odl[0]['found'], odl[0]['N'], odl[1]['found'])) if odl else ''))
            for c in ev: P('      %-5s %s' % (c, M.fmt(ev[c][0], ev[c][1])))
        curve.append(row)
    json.dump(curve, open(os.path.join(R, 'filters_curve.json'), 'w'))
    # ручные правила вместе + гибрид
    P('\n== наборы правил (без модели) и гибрид')
    for name, rules in [('chart', ('chart',)), ('chart+prose', ('chart', 'prose')), ('chart+prose+tiny', ('chart', 'prose', 'tiny')),
                        ('all_rules', tuple(RULES)), ('chart+prose+tiny+toc+list', ('chart', 'prose', 'tiny', 'toc', 'list'))]:
        for hyb in [None, dict(ecov=1.0)]:
            for part, D in [('train', TRAIN), ('test', TEST)]:
                ev = run_eval(make_select(rules=rules, hyb=hyb), D, B=200); sm = summarize(ev); odl = ev.get('odl')
                P('%-28s hyb=%-5s %-5s found %d/%d (%.3f) teds %.3f shape %.3f fp_page %d/%d (%.3f)%s' % (
                    name, bool(hyb), part, sm['found'], sm['N'], sm['found_rate'], sm['teds_all'], sm['shape_rate'], sm['neg_pages_fp'], sm['neg_pages'], sm['fp_page_rate'],
                    (' | ODL fp_doc %d/%d CI %s found %d/%d teds %.3f' % (odl[0]['neg_docs_fp'], odl[0]['neg_docs'], odl[1]['fp_doc_rate'], odl[0]['found'], odl[0]['N'], odl[0]['teds_all'])) if odl else ''))
                for c in ev: P('      %-5s %s' % (c, M.fmt(ev[c][0], ev[c][1])))
    log.close()
