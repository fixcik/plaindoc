"""Кривая «ложные против найденных» по порогу логистической модели (модель из filters_logit.json, обучена на train),
отдельно train/test, с бутстрап-интервалами по документам; выбор рабочей точки на train (fp_page_rate < 5%).
python3 -I curve_fine.py RESULTS_DIR [ecov_for_hybrid]"""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import filters as F, metric2 as M
R = sys.argv[1]; ECOV = float(sys.argv[2]) if len(sys.argv) > 2 else 0.7
mj = json.load(open(os.path.join(R, 'filters_logit.json')))
model = F.Logit(); model.w, model.b, model.mu, model.sd = mj['w'], mj['b'], mj['mu'], mj['sd']
LONG = {c: {d for d, v in F.GT[c].items() if (v['pages'] or 0) > 30} for c in F.CORP}
def ev_part(sel, D, B=300):
    out = {}
    for c in F.CORP:
        docs = D[c]
        if not docs: continue
        m, ci = M.bootstrap(F.GT[c], F.PR[c], select=sel, gt_filter=F.gtf, docs=docs, B=B)
        ms = M.evaluate(F.GT[c], F.PR[c], select=sel, gt_filter=F.gtf, docs=docs - LONG[c])[0] if LONG[c] else m
        out[c] = dict(m=m, ci=ci, fp_page_rate_short=ms['fp_page_rate'], neg_pages_short=ms['neg_pages'], neg_pages_fp_short=ms['neg_pages_fp'])
    return out
def pooled(ev):
    N = sum(v['m']['N'] for v in ev.values()); found = sum(v['m']['found'] for v in ev.values())
    teds = sum(v['m']['teds_all'] * v['m']['N'] for v in ev.values()) / N if N else 0
    shape = sum(v['m']['shape'] for v in ev.values())
    negp = sum(v['m']['neg_pages'] for v in ev.values()); negpf = sum(v['m']['neg_pages_fp'] for v in ev.values())
    negps = sum(v['neg_pages_short'] for v in ev.values()); negpfs = sum(v['neg_pages_fp_short'] for v in ev.values())
    return dict(N=N, found=found, found_rate=found / N if N else 0, teds_all=teds, shape_rate=shape / N if N else 0,
                neg_pages=negp, neg_pages_fp=negpf, fp_page_rate=negpf / negp if negp else 0, fp_page_rate_short=negpfs / negps if negps else 0)
rows = []
thrs = [round(0.05 * i, 2) for i in range(0, 20)]
for hyb in [None, dict(ecov=ECOV)]:
    for thr in thrs:
        sel = F.make_select(model, thr, hyb=hyb)
        rec = dict(thr=thr, hybrid=bool(hyb))
        for part, D in [('train', F.TRAIN), ('test', F.TEST)]:
            ev = ev_part(sel, D); p = pooled(ev)
            rec[part] = dict(pooled=p, per=ev)
            odl = ev.get('odl')
            print('hyb=%d thr %.2f %-5s found %d/%d (%.3f) teds %.3f shape %.3f fp_page %d/%d (%.3f) short %.3f%s' % (
                bool(hyb), thr, part, p['found'], p['N'], p['found_rate'], p['teds_all'], p['shape_rate'], p['neg_pages_fp'], p['neg_pages'], p['fp_page_rate'], p['fp_page_rate_short'],
                ' | ODL found %d/%d CI %s teds %.3f CI %s fp_doc %d/%d (%.3f) CI %s' % (odl['m']['found'], odl['m']['N'], odl['ci']['found'], odl['m']['teds_all'], odl['ci']['teds_all'],
                                                                                        odl['m']['neg_docs_fp'], odl['m']['neg_docs'], odl['m']['fp_doc_rate'], odl['ci']['fp_doc_rate']) if odl else ''), flush=True)
            for c in ev:
                v = ev[c]; print('      %-5s %s | fp_page_short %.3f' % (c, M.fmt(v['m'], v['ci']), v['fp_page_rate_short']), flush=True)
        rows.append(rec)
json.dump(rows, open(os.path.join(R, 'curve_fine.json'), 'w'))
# рабочая точка: минимальный порог, при котором на train fp_page_rate < 0.05 (пул) и ни в одном корпусе train не выше 0.05
with open(os.path.join(R, 'curve_fine.tsv'), 'w') as fo:
    fo.write('hybrid\tthr\tpart\tN\tfound\tfound_rate\tteds_all\tshape_rate\tneg_pages\tneg_pages_fp\tfp_page_rate\tfp_page_rate_short\todl_found\todl_teds\todl_fp_docs\todl_fp_doc_rate\todl_fp_doc_ci\todl_found_ci\todl_teds_ci\ticdar_found\ticdar_teds\ticdar_fp_page\tpt1m_found\tpt1m_teds\tpt1m_fp_page\n')
    for rec in rows:
        for part in ('train', 'test'):
            p = rec[part]['pooled']; per = rec[part]['per']
            def g(c, k, d='-'):
                return per[c]['m'][k] if c in per else d
            def gc(c, k):
                return per[c]['ci'][k] if c in per else '-'
            fo.write('\t'.join(str(x) for x in [int(rec['hybrid']), rec['thr'], part, p['N'], p['found'], round(p['found_rate'], 4), round(p['teds_all'], 4), round(p['shape_rate'], 4), p['neg_pages'], p['neg_pages_fp'], round(p['fp_page_rate'], 4), round(p['fp_page_rate_short'], 4),
                                             g('odl', 'found'), round(g('odl', 'teds_all', 0), 4), g('odl', 'neg_docs_fp'), round(g('odl', 'fp_doc_rate', 0), 4), gc('odl', 'fp_doc_rate'), gc('odl', 'found'), gc('odl', 'teds_all'),
                                             g('icdar', 'found'), round(g('icdar', 'teds_all', 0), 4), round(g('icdar', 'fp_page_rate', 0), 4), g('pt1m', 'found'), round(g('pt1m', 'teds_all', 0), 4), round(g('pt1m', 'fp_page_rate', 0), 4)]) + '\n')
for hyb in (False, True):
    cand = [r for r in rows if r['hybrid'] == hyb and r['train']['pooled']['fp_page_rate'] < 0.05 and all(v['m']['fp_page_rate'] < 0.05 for v in r['train']['per'].values())]
    if cand:
        best = min(cand, key=lambda r: r['thr'])
        print('РАБОЧАЯ ТОЧКА hybrid=%s: thr=%.2f (выбрана на train) -> train found %.3f fp_page %.3f | test found %.3f teds %.3f fp_page %.3f | ODL(test) found %s teds %.3f fp_doc %.3f CI %s' % (
            hyb, best['thr'], best['train']['pooled']['found_rate'], best['train']['pooled']['fp_page_rate'], best['test']['pooled']['found_rate'], best['test']['pooled']['teds_all'], best['test']['pooled']['fp_page_rate'],
            best['test']['per']['odl']['m']['found'], best['test']['per']['odl']['m']['teds_all'], best['test']['per']['odl']['m']['fp_doc_rate'], best['test']['per']['odl']['ci']['fp_doc_rate']))
    else: print('РАБОЧАЯ ТОЧКА hybrid=%s: порог с fp_page<5%% на train не найден' % hyb)
