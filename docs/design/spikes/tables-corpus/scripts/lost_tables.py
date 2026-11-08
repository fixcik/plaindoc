"""Какие настоящие таблицы теряет фильтр при пороге THR (и какие FP остаются): по корпусам, типу линий, классу.
python3 -I lost_tables.py RESULTS_DIR THR [rules,comma,separated]"""
import sys, os, json, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import filters as F, metric2 as M
R = sys.argv[1]; THRS = [float(x) for x in sys.argv[2].split(',')]; RULES = tuple(x for x in (sys.argv[3].split(',') if len(sys.argv) > 3 else []) if x)
mj = json.load(open(os.path.join(R, 'filters_logit.json')))
model = F.Logit(); model.w, model.b, model.mu, model.sd = mj['w'], mj['b'], mj['mu'], mj['sd']
src = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'fp_classify.py')).read()
ns = {'__file__': os.path.join(os.path.dirname(os.path.abspath(__file__)), 'fp_classify.py')}
exec(src.split("gt = json.load")[0] + "\n" + src.split("m, res, fps = M.evaluate(gt, pr, gt_filter=gtf)")[1].split("cnt = collections.Counter()")[0], ns)
classify = ns['classify']
GEOM = {'odl': os.path.join(os.path.dirname(R), 'tbl', 'geomseg'), 'icdar': os.path.join(os.path.dirname(R), 'corpus', 'icdar2013', 'geom'), 'pt1m': os.path.join(os.path.dirname(R), 'corpus', 'pubtables1m', 'geom')}
for THR in THRS:
  sel = F.make_select(model, THR, rules=RULES)
  for part, D in [('train', F.TRAIN), ('test', F.TEST)]:
    print('== %s: порог %.2f, правила %s' % (part, THR, RULES or '-'))
    for c in F.CORP:
        docs = D[c]
        if not docs: continue
        m0, res0, fps0 = M.evaluate(F.GT[c], F.PR[c], gt_filter=F.gtf, docs=docs)
        m1, res1, fps1 = M.evaluate(F.GT[c], F.PR[c], select=sel, gt_filter=F.gtf, docs=docs)
        f0 = {(r['doc'], r['page'], r['gt']['rows'][0][0][2][:30] if r['gt']['rows'] and r['gt']['rows'][0] else '', id(r['gt'])): r for r in res0 if r['found']}
        f1 = {(r['doc'], r['page'], r['gt']['rows'][0][0][2][:30] if r['gt']['rows'] and r['gt']['rows'][0] else '', id(r['gt'])): r for r in res1 if r['found']}
        lost = [r for k, r in f0.items() if k not in f1]
        print('  %-5s found %d -> %d (потеряно %d), FP %d -> %d, teds_all %.3f -> %.3f' % (c, m0['found'], m1['found'], len(lost), m0['fp_tables'], m1['fp_tables'], m0['teds_all'], m1['teds_all']))
        cnt = collections.Counter(); ex = collections.defaultdict(list)
        for r in lost:
            byid = {rr['_id']: rr for rr in F.PR[c].get(r['doc'], [])}
            rec = byid.get(r['rid']); f = rec['feat']
            key = '%s kind=%s %dx%d' % (classify(f, M.rows_of(rec), rec['kind']), rec['kind'], f['nrows'], f['ncols'])
            cls = classify(f, M.rows_of(rec), rec['kind']); cnt[(cls, rec['kind'])] += 1
            if len(ex[(cls, rec['kind'])]) < 4: ex[(cls, rec['kind'])].append('%s:p%d(%dx%d,p=%.2f,teds=%.2f)' % (r['doc'], r['page'] + 1, f['nrows'], f['ncols'], model.p(F.vec(rec)), r['teds']))
        for k, n in cnt.most_common(): print('      потеряно %-22s %3d  %s' % (k, n, ', '.join(ex[k])))
        cnt = collections.Counter()
        for fp in fps1: cnt[classify(fp['feat'], fp['rows'], fp['kind'])] += 1
        print('      остались FP: ' + ', '.join('%s=%d' % kv for kv in cnt.most_common()))
