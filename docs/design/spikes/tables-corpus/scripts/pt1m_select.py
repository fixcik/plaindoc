"""Выбор подмножества документов PubTables-1M (из потока аннотаций) и список PDF для скачивания из AWS-бакета PMC.
python3 -I pt1m_select.py ANN_DIR N OUT_LIST  -> пишет OUT_LIST (pmc_id<TAB>n_tables<TAB>n_pages_with_tables<TAB>split)
Критерии: есть >=1 таблица, не исключённая ни для detection, ни для structure; у документа все таблицы с
pdf_file_name одним; выбор детерминированный (seed 1)."""
import json, os, sys, random, glob
ann, N, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
files = sorted(glob.glob(os.path.join(ann, '*_tables.json')))
random.seed(int(sys.argv[4]) if len(sys.argv) > 4 else 1); random.shuffle(files)
sel = []
for f in files:
    try: d = json.load(open(f))
    except Exception: continue
    if not d: continue
    ok = [t for t in d if not t['exclude_for_detection'] and not t['exclude_for_structure']]
    if not ok: continue
    if len({t['pdf_file_name'] for t in d}) != 1: continue
    pmc = d[0]['pmc_id']
    pages = {t['pdf_page_index'] for t in d}
    sel.append((pmc, len(d), len(ok), len(pages), d[0]['split'], os.path.basename(f)))
    if len(sel) >= N: break
with open(out, 'w') as fo:
    for s in sel: fo.write('\t'.join(map(str, s)) + '\n')
print(len(sel), 'docs;', sum(s[1] for s in sel), 'tables;', sum(s[2] for s in sel), 'usable')
