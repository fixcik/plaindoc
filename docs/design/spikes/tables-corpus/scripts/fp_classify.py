"""Классификация причин ложных таблиц по признакам (эвристика; выборочно проверено глазами по рендеру).
python3 -I fp_classify.py GT.json PRED_DIR LABEL [GT_FILTER_EXCL_DET]
Классы: chart (диаграмма/оси/легенда), prose (многоколоночная проза/врезка), caption_box (подпись/заголовок в рамке, 2x2),
toc (оглавление), list (нумерованный список/ссылки), math (формула), frame (рамка страницы/крупная врезка), diagram (блок-схема),
header_footer (колонтитул), other."""
import json, sys, os, re, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import metric2 as M
gt = json.load(open(sys.argv[1])); pr = M.load_preds(sys.argv[2]); label = sys.argv[3]
gtf = (lambda t: not t.get('excl_det')) if len(sys.argv) > 4 else None
m, res, fps = M.evaluate(gt, pr, gt_filter=gtf)

ORD = re.compile(r'^\(?\d{1,3}[.)]?$|^[ivxlc]+[.)]$')
def classify(f, rows, kind):
    rows = [r for r in rows if r]
    if not rows: return 'other'
    first = [r[0][2].strip() for r in rows if r[0][2].strip()]
    last = [r[-1][2].strip() for r in rows if r[-1][2].strip()]
    txt = ' '.join(c[2] for r in rows for c in r)
    nr, nc = f['nrows'], f['ncols']
    if f['nw'] == 0 or f['fill'] == 0: return 'empty_grid'
    if nr * nc == 1: return 'boxed_text'
    if f['area_frac'] >= 0.5 and nr * nc <= 6: return 'frame'
    if re.search(r'(open access|address:|e-?mail|received:|keywords|corresponding author|published:)', txt, re.I) and f['y0_frac'] <= 0.5 and f['num_frac'] < 0.2: return 'article_header'
    if kind == 'L' and f['sparse_rows'] >= 0.6 and f['n_v'] == 0 and nc <= 2 and f['nw'] < 40 and f['num_frac'] < 0.3: return 'math'
    if re.search(r'[=+−∑∫√≤≥±×→]|\b(lim|log|exp|sin|cos)\b', txt) and f['nw'] < 40 and f['num_frac'] < 0.5 and nc <= 3: return 'math'
    if kind == 'L' and (f['n_paths'] >= 50 or f['n_segs'] >= 20) and f['num_frac'] < 0.2 and f['fill'] <= 0.85: return 'diagram'
    if kind == 'L' and f['n_segs'] >= 20 and f['num_frac'] >= 0.2: return 'chart'
    if f['n_diag'] >= 8: return 'chart' if f['num_frac'] >= 0.2 else 'diagram'
    if (f['num_frac'] >= 0.45 and (f['fill'] <= 0.7 or f['sparse_rows'] >= 0.3)) or (f['paths_per_word'] >= 1.0 and f['fill'] <= 0.7 and f['num_frac'] >= 0.2) or (f['fill_area'] >= 0.5 and f['line_w_mean'] <= 0.35 and f['num_frac'] >= 0.2): return 'chart'
    if f['fill_area'] >= 0.5 and f['n_v'] <= 3 and f['line_w_mean'] <= 0.35 and kind == 'L': return 'chart'
    if nr * nc <= 4 and kind == 'L' and re.search(r'(figure|diagram|table|fig\.|chart|source)', txt, re.I): return 'caption_box'
    if nr * nc <= 4 and kind == 'L': return 'caption_box'
    if nc <= 3 and len(last) >= 3 and sum(bool(re.fullmatch(r'[0-9]{1,4}|[ivxlc]+', x.lower())) for x in last) / len(last) >= 0.8 and f['first_col_words'] <= 1.5: return 'toc'
    if nc <= 3 and len(first) >= 3 and sum(bool(ORD.match(x)) for x in first) / len(first) >= 0.7 and f['mean_words'] >= 2: return 'list'
    if re.search(r'[=+−∑∫√≤≥±×→]|\b(lim|log|exp|sin|cos)\b', txt) and f['nw'] < 40 and kind == 'T': return 'math'
    if kind == 'T' and f['num_frac'] >= 0.6 and nr <= 4: return 'chart'
    if kind == 'L' and (f['fill_area'] >= 0.5 or f['n_paths'] >= 20) and f['num_frac'] < 0.5 and f['fill'] <= 0.9: return 'figure_labels'
    if kind == 'T' and (f['mean_words'] >= 3.5 or f['line_w_mean'] >= 0.85) and f['num_frac'] < 0.25: return 'prose'
    if f['y1_frac'] <= 0.12 or f['y0_frac'] >= 0.9: return 'header_footer'
    if f['num_frac'] >= 0.3 and f['sparse_rows'] >= 0.4: return 'chart'
    if kind == 'L' and f['fill'] <= 0.85 and f['n_v'] >= 4 and f['num_frac'] < 0.2 and f['mean_words'] >= 2: return 'diagram'
    if kind == 'L' and f['fill'] <= 0.5 and f['n_v'] <= 3: return 'diagram'
    return 'other'
cnt = collections.Counter(); ex = collections.defaultdict(list); byc = collections.Counter()
for f in fps:
    c = classify(f['feat'], f['rows'], f['kind']); cnt[c] += 1
    if len(ex[c]) < 6: ex[c].append('%s:p%d' % (f['doc'], f['page'] + 1))
    byc[(c, f['kind'])] += 1
tot = sum(cnt.values())
print('%s: FP всего %d (страниц без таблиц %d, из них с FP %d)' % (label, tot, m['neg_pages'], m['neg_pages_fp']))
for c, n in cnt.most_common():
    print('  %-14s %3d (%4.1f%%)  примеры: %s' % (c, n, 100 * n / tot, ', '.join(ex[c])))
# те же классы для найденных настоящих таблиц: сколько настоящих попадает под каждый класс (цена)
tp = collections.Counter()
for r in res:
    if r['found'] and r['feat']:
        rows = [[[1, 1, '']]]
        # восстановим строки из предсказания не храним; используем gt-строки только для текста
        tp[classify(r['feat'], r['gt']['rows'], r['kind'])] += 1
print('  найденные настоящие таблицы по тем же правилам: ' + ', '.join('%s=%d' % kv for kv in tp.most_common()))
json.dump(dict(label=label, counts=cnt, examples=ex, tp_counts=tp, by_kind={str(k): v for k, v in byc.items()}), open(os.path.join(os.path.dirname(sys.argv[2]), 'fp_classes_%s.json' % label), 'w'), ensure_ascii=False, indent=1)
