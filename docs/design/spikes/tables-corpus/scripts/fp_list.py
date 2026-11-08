"""Список ложных срабатываний (на страницах без эталонных таблиц) с признаками.
python3 -I fp_list.py GT.json PRED_DIR OUT.json [--excess]"""
import json, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import metric2 as M
gt = json.load(open(sys.argv[1])); pr = M.load_preds(sys.argv[2])
m, res, fps = M.evaluate(gt, pr)
print(M.fmt(m))
KEYS = ['nrows', 'ncols', 'fill', 'mean_words', 'num_frac', 'numcols', 'nw', 'nlines', 'line_w_mean', 'n_h', 'n_v', 'h_len', 'v_len',
        'n_paths', 'n_small_paths', 'n_segs', 'n_diag', 'fill_area', 'paths_per_word', 'row_h_cv', 'align_cv', 'w_frac', 'h_frac', 'y0_frac', 'y1_frac', 'fs_ratio', 'sparse_rows', 'first_col_words', 'last_col_num', 'word_share']
out = []
for f in fps:
    ft = f['feat'] or {}
    rows = f['rows']
    txt = [' | '.join(c[2][:25] for c in r) for r in rows[:3]]
    rec = dict(doc=f['doc'], page=f['page'], kind=f['kind'], bbox=f['bbox'], feat={k: ft.get(k) for k in KEYS}, sample=txt)
    out.append(rec)
    print('%s p%d %s bbox=%s %dx%d fill=%.2f mw=%.1f num=%.2f nw=%d lines=%d lw=%.2f nh=%d nv=%d paths=%d small=%d segs=%d diag=%d fa=%.2f ppw=%.2f rhcv=%.2f al=%.2f y0=%.2f' % (
        f['doc'], f['page'], f['kind'], [round(v) for v in f['bbox']] if f['bbox'] else None, ft.get('nrows', 0), ft.get('ncols', 0), ft.get('fill', 0), ft.get('mean_words', 0),
        ft.get('num_frac', 0), ft.get('nw', 0), ft.get('nlines', 0), ft.get('line_w_mean', 0), ft.get('n_h', 0), ft.get('n_v', 0), ft.get('n_paths', 0), ft.get('n_small_paths', 0),
        ft.get('n_segs', 0), ft.get('n_diag', 0), ft.get('fill_area', 0), ft.get('paths_per_word', 0), ft.get('row_h_cv', 0), ft.get('align_cv', 0), ft.get('y0_frac', 0)))
    for t in txt: print('     ', t[:140])
json.dump(out, open(sys.argv[3], 'w'), ensure_ascii=False, indent=0)
