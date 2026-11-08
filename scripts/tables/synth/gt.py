"""Постраничный эталон по реальной раскладке PDF + проверка текстового слоя.
python3 -I gt.py PLAN.jsonl KIT_DIR OXIDE_DUMP_DIR OUT_DIR
KIT_DIR: дамп PDFKit (pdftext), независимый от pdf_oxide; OXIDE_DUMP_DIR: дамп pdf-spike-segs (words).
Пишет OUT_DIR/gt/<doc>.json (фрагменты по страницам), OUT_DIR/gt_logical/<doc>.json, OUT_DIR/manifest.jsonl, OUT_DIR/verify.json"""
import json, sys, os, re
from collections import defaultdict, Counter
WS = re.compile(r'[\s  ]+')
norm = lambda s: WS.sub(' ', s).strip().lower()
def toks(s): return [x for x in norm(s).split(' ') if x]

class Page:
    def __init__(s, tk, w=1e9, h=1e9):
        s.inst = defaultdict(list)  # token -> [[y, used]]
        tk = [(t, x, y) for t, x, y in tk if -1 <= x <= w - 3 and -1 <= y <= h]  # текст за краем страницы (обрезан) не считается
        for t, x, y in tk:
            for z in toks(t): s.inst[z].append([y, False])
        for v in s.inst.values(): v.sort()
        s.text = ''.join(norm(t).replace(' ', '') for t, x, y in tk)
    def match(s, tokens, ymin):
        """Все токены найдены среди неиспользованных с y >= ymin? Возврат (ok, picked, missing)."""
        picked = []; missing = []; ids = set()
        for z in tokens:
            c = next((it for it in s.inst.get(z, []) if not it[1] and it[0] >= ymin and id(it) not in ids), None)
            if c is None: missing.append(z)
            else: picked.append(c); ids.add(id(c))
        return not missing, picked, missing
    def commit(s, picked):
        for it in picked: it[1] = True

def row_tokens(r): return [z for c in r for z in toks(c['t'])]

def build(doc, kit_pages, ox_pages):
    pages = [Page(p['tok'], p['w'], p['h']) for p in kit_pages]
    ox = [Counter(z for w in p['words'] for z in toks(w[4])) for p in ox_pages] if ox_pages else []
    cur = 0; frags_all = []; logical = []; recs = []
    for e in doc['tables']:
        if e['nested_inner']: continue
        rows = e['gt_rows']; nh = e['head_rows']
        assign = []; miss_rows = 0; soft_rows = 0; soft_ex = []; missing_tok = []; ymin = -1e9; rep = {}
        for ri, r in enumerate(rows):
            tk = row_tokens(r)
            placed = False
            for p in range(cur, min(cur + 2, len(pages))):
                if p != cur:
                    ymin = -1e9
                    if ri >= nh and nh and p not in rep:  # повтор шапки на новой странице
                        allh = [z for hr in rows[:nh] for z in row_tokens(hr)]
                        ok, pk, _ = pages[p].match(allh, -1e9)
                        if ok:
                            pages[p].commit(pk); rep[p] = True; ymin = min(it[0] for it in pk) if pk else ymin
                ok, pk, ms = pages[p].match(tk, ymin + 0.5 if ymin > -1e9 else ymin)
                if ok:
                    pages[p].commit(pk); cur = p
                    if pk: ymin = min(it[0] for it in pk)
                    assign.append(p); placed = True; break
            if not placed:
                # токены разрезаны/склеены переносом (например '(маш.' + '-ч)'): проверка по подстроке текста без пробелов
                p2 = next((p for p in range(cur, min(cur + 2, len(pages))) if all(z in pages[p].text for z in tk)), None)
                if p2 is None: miss_rows += 1; missing_tok += [z for z in tk if z not in pages[cur].text][:5]; p2 = cur
                else: soft_rows += 1; soft_ex.append([z for z in ms][:3])
                cur = p2; assign.append(p2)
        # rowspan через разрыв страницы
        broken = any(c['rs'] > 1 and len(set(assign[ri:ri + c['rs']])) > 1 for ri, r in enumerate(rows) for c in r)
        pg = sorted(set(assign[nh:] if len(rows) > nh else assign))
        hp = assign[0] if assign else 0
        frs = []
        for k, p in enumerate(sorted(set(assign))):
            fr = [r for r, a in zip(rows, assign) if a == p]
            hr = 0
            if p in rep: fr = rows[:nh] + fr; hr = nh
            frs.append(dict(page=p, rows=fr, table_id=e['table_id'], frag=k, repeated_header_rows=hr))
        for f in frs: f['nfrag'] = len(frs)
        # подстраховка по pdf_oxide (находка, а не брак)
        ox_miss = 0; ox_tot = 0
        for r, a in zip(rows, assign):
            for c in r:
                tz = toks(c['t'])
                if not tz: continue
                ox_tot += 1
                if ox and a < len(ox) and not all(ox[a][z] > 0 for z in tz): ox_miss += 1
        rec = {k: v for k, v in e.items() if k != 'gt_rows'}
        rec.update(doc=doc['doc'], kind=doc['kind'], pages=sorted(set(assign)), multi_page=len(set(assign)) > 1, nfrag=len(frs),
                   repeated_header=bool(rep), bad_rows=miss_rows, soft_rows=soft_rows, soft_examples=soft_ex[:3], missing_examples=missing_tok[:5], rowspan_broken=broken,
                   oxide_cells_missing=ox_miss, cells_nonempty=ox_tot, landscape=e.get('landscape', False))
        rec['ok'] = miss_rows == 0 and not broken
        recs.append(rec)
        frags_all += frs
        logical.append(dict(page=hp, pages=sorted(set(assign)), rows=rows, table_id=e['table_id']))
        # вложенные: страница родителя
        for e2 in doc['tables']:
            if e2['nested_inner'] and e2['parent'] == e['table_id']:
                r2 = {k: v for k, v in e2.items() if k != 'gt_rows'}
                r2.update(doc=doc['doc'], kind=doc['kind'], pages=[hp], multi_page=False, nfrag=1, repeated_header=False, bad_rows=0,
                          rowspan_broken=False, ok=rec['ok'], oxide_cells_missing=None, cells_nonempty=None, landscape=False)
                recs.append(r2)
                frags_all.append(dict(page=hp, rows=e2['gt_rows'], table_id=e2['table_id'], frag=0, nfrag=1, repeated_header_rows=0, nested_inner=True))
    return frags_all, logical, recs

def main(plan, kit, oxd, out):
    for sub in ('gt', 'gt_logical'): os.makedirs('%s/%s' % (out, sub), exist_ok=True)
    man = open('%s/manifest.jsonl' % out, 'w'); stat = Counter(); bad_docs = []
    for l in open(plan):
        d = json.loads(l)
        try: kp = json.load(open('%s/%s.json' % (kit, d['doc'])))['pages']
        except Exception: kp = []
        try: op = json.load(open('%s/%s.json' % (oxd, d['doc'])))['pages']
        except Exception: op = []
        stat['docs'] += 1
        if not kp: stat['no_pdf'] += 1; bad_docs.append(d['doc']); continue
        if d['decoy']:
            json.dump([], open('%s/gt/%s.json' % (out, d['doc']), 'w')); json.dump([], open('%s/gt_logical/%s.json' % (out, d['doc']), 'w'))
            man.write(json.dumps(dict(doc=d['doc'], kind='decoy', decoy=d['decoy'], n_pages=len(kp), tables=0, ok=len(kp) == 1), ensure_ascii=False) + '\n')
            stat['decoy'] += 1; stat['decoy_pages'] += len(kp); continue
        fr, lg, recs = build(d, kp, op)
        for r in recs:
            r['n_pages_doc'] = len(kp)
            man.write(json.dumps(r, ensure_ascii=False) + '\n')
            stat['tables'] += 1; stat['tables_ok'] += r['ok']
            if not r['ok']: bad_docs.append(d['doc'])
        json.dump(fr, open('%s/gt/%s.json' % (out, d['doc']), 'w'), ensure_ascii=False)
        json.dump(lg, open('%s/gt_logical/%s.json' % (out, d['doc']), 'w'), ensure_ascii=False)
        stat['pages'] += len(kp)
    stat['bad_docs'] = len(set(bad_docs))
    json.dump(dict(stat=stat, bad_docs=sorted(set(bad_docs))), open('%s/verify.json' % out, 'w'), ensure_ascii=False, indent=1)
    print(dict(stat))

if __name__ == '__main__':
    main(*sys.argv[1:5])
