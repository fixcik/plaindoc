"""Наборы ручных правил (с моделью и без) — train/test по корпусам, с бутстрап-интервалами.
python3 -I rules_eval.py RESULTS_DIR"""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import filters as F, metric2 as M
R = sys.argv[1]
mj = json.load(open(os.path.join(R, 'filters_logit.json')))
model = F.Logit(); model.w, model.b, model.mu, model.sd = mj['w'], mj['b'], mj['mu'], mj['sd']
SETS = [('none', ()), ('chart', ('chart',)), ('chart+tiny', ('chart', 'tiny')), ('chart+tiny+sparse', ('chart', 'tiny', 'sparse')),
        ('chart+tiny+sparse+toc+list+frame', ('chart', 'tiny', 'sparse', 'toc', 'list', 'frame')), ('all(+prose)', tuple(F.RULES))]
out = []
for name, rules in SETS:
    for thr in (None, 0.10, 0.20):
        for hyb in (None, dict(ecov=0.7)):
            sel = F.make_select(model if thr is not None else None, thr or 0.0, rules=rules, hyb=hyb)
            for part, D in [('train', F.TRAIN), ('test', F.TEST)]:
                ev = F.run_eval(sel, D, B=300); sm = F.summarize(ev); odl = ev.get('odl')
                line = '%-34s thr=%-4s hyb=%d %-5s found %d/%d (%.3f) teds %.3f shape %.3f fp_page %d/%d (%.3f)' % (
                    name, thr, bool(hyb), part, sm['found'], sm['N'], sm['found_rate'], sm['teds_all'], sm['shape_rate'], sm['neg_pages_fp'], sm['neg_pages'], sm['fp_page_rate'])
                for c in ev:
                    m, ci = ev[c]
                    line += ' | %s found %d teds %.3f %s %.3f CI %s' % (c, m['found'], m['teds_all'], 'fp_doc' if c == 'odl' else 'fp_page', m['fp_doc_rate'] if c == 'odl' else m['fp_page_rate'], ci['fp_doc_rate'] if c == 'odl' else ci['fp_page_rate'])
                print(line, flush=True)
                out.append(dict(rules=name, thr=thr, hybrid=bool(hyb), part=part, summary=sm, per={c: dict(m=ev[c][0], ci=ev[c][1]) for c in ev}))
json.dump(out, open(os.path.join(R, 'rules_eval.json'), 'w'))
