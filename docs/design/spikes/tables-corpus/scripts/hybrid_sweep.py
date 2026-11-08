"""Перебор порога полноты сетки (edge_cov) для гибрида: прототип-детектор + планарная структура, train/test отдельно.
python3 -I hybrid_sweep.py RESULTS_DIR"""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import filters as F, metric2 as M
R = sys.argv[1]
out = []
GUARD = os.environ.get('GUARD', '1') == '1'; SUF = os.environ.get('SUF', '')
for ecov in [None, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]:
    for nonrect in ([False] if ecov is None else [False, True]):
        sel = None if ecov is None else F.make_select(hyb=dict(ecov=ecov, allow_nonrect=nonrect, guard=GUARD))
        for part, D in [('train', F.TRAIN), ('test', F.TEST)]:
            ev = F.run_eval(sel, D, B=200); sm = F.summarize(ev)
            line = 'ecov=%-4s nonrect=%-5s %-5s found %d/%d (%.3f) teds %.3f shape %d (%.3f)' % (ecov, nonrect, part, sm['found'], sm['N'], sm['found_rate'], sm['teds_all'], sm['shape'], sm['shape_rate'])
            for c in ev: line += ' | %s teds %.3f shape %d CI teds %s' % (c, ev[c][0]['teds_all'], ev[c][0]['shape'], ev[c][1]['teds_all'])
            print(line); out.append(dict(ecov=ecov, nonrect=nonrect, part=part, summary=sm, per={c: dict(m=ev[c][0], ci=ev[c][1]) for c in ev}))
json.dump(out, open(os.path.join(R, 'hybrid_sweep%s.json' % SUF), 'w'))
