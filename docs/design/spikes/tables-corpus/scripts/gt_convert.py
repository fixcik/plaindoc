"""Приведение эталонов к единому формату:
  {doc: {"pages": N, "tables": [{"page": i (0-based), "bbox": [x0,top,x1,bot] | null, "rows": [[[cs,rs,text],...],...]}]}}
Координаты top-origin (как в proto.py). Использование:
  python3 -I gt_convert.py odl   ODL_GT_DIR  GEOM_DIR OUT.json
  python3 -I gt_convert.py icdar ICDAR_X_DIR GEOM_DIR OUT.json
  python3 -I gt_convert.py pt1m  ANN_DIR     GEOM_DIR OUT.json   (только документы, для которых есть GEOM_DIR/<pmc>.json)
"""
import json, glob, os, sys, re
import xml.etree.ElementTree as ET
from html.parser import HTMLParser

class P(HTMLParser):
    def __init__(s):
        super().__init__(); s.tables = []; s.t = None; s.r = None; s.c = None
    def handle_starttag(s, tag, a):
        a = dict(a)
        if tag == 'table': s.t = []
        elif tag == 'tr' and s.t is not None: s.r = []
        elif tag in ('td', 'th') and s.r is not None:
            s.c = [int(a.get('colspan', 1) or 1), int(a.get('rowspan', 1) or 1), '']
    def handle_data(s, d):
        if s.c is not None: s.c[2] += d
    def handle_endtag(s, tag):
        if tag in ('td', 'th') and s.c is not None: s.r.append(s.c); s.c = None
        elif tag == 'tr' and s.r is not None: s.t.append(s.r); s.r = None
        elif tag == 'table' and s.t is not None: s.tables.append(s.t); s.t = None

def npages(geom_dir, doc):
    try: return len(json.load(open(os.path.join(geom_dir, doc + '.json')))['pages'])
    except Exception: return None
def page_h(geom_dir, doc, pi):
    try: return json.load(open(os.path.join(geom_dir, doc + '.json')))['pages'][pi]['h']
    except Exception: return None

def odl(gt_dir, geom_dir):
    out = {}
    for g in sorted(glob.glob(os.path.join(gt_dir, '*.md'))):
        n = os.path.basename(g)[:-3]
        p = P(); p.feed(open(g).read())
        out[n] = dict(pages=npages(geom_dir, n) or 1, tables=[dict(page=0, bbox=None, rows=t) for t in p.tables])
    return out

def icdar(xdir, geom_dir):
    out = {}
    for pdf in sorted(glob.glob(os.path.join(xdir, '*', '*.pdf'))):
        n = os.path.basename(pdf)[:-4]
        xml = pdf[:-4] + '-str.xml'
        tables = []
        if os.path.exists(xml):
            root = ET.parse(xml).getroot()
            for tb in root.iter('table'):
                for reg in tb.iter('region'):
                    pi = int(reg.get('page', '1')) - 1
                    cells = []
                    for c in reg.iter('cell'):
                        sr = int(c.get('start-row')); sc = int(c.get('start-col'))
                        er = int(c.get('end-row', sr)); ec = int(c.get('end-col', sc))
                        bb = c.find('bounding-box'); cont = c.find('content')
                        txt = (cont.text or '') if cont is not None else ''
                        def fl(v):
                            m = re.match(r'-?\d+(\.\d+)?', v or ''); return float(m.group()) if m else None
                        bbv = [fl(bb.get(k)) for k in ('x1', 'y1', 'x2', 'y2')] if bb is not None else None
                        cells.append((sr, sc, er, ec, txt, bbv if bbv and all(v is not None for v in bbv) else None))
                    if not cells: continue
                    nr = max(c[2] for c in cells) + 1
                    rows = [[] for _ in range(nr)]
                    for sr, sc, er, ec, txt, bb in sorted(cells, key=lambda c: (c[0], c[1])):
                        rows[sr].append([ec - sc + 1, er - sr + 1, txt])
                    bbs = [c[5] for c in cells if c[5]]
                    H = page_h(geom_dir, n, pi)
                    bbox = None
                    if bbs and H:
                        x0 = min(b[0] for b in bbs); x1 = max(b[2] for b in bbs)
                        ylo = min(b[1] for b in bbs); yhi = max(b[3] for b in bbs)
                        bbox = [x0, H - yhi, x1, H - ylo]
                    tables.append(dict(page=pi, bbox=bbox, rows=rows))
        out[n] = dict(pages=npages(geom_dir, n), tables=tables)
    return out

def pt1m(ann_dir, geom_dir):
    out = {}
    for g in sorted(glob.glob(os.path.join(geom_dir, '*.json'))):
        n = os.path.basename(g)[:-5]
        f = os.path.join(ann_dir, n + '_tables.json')
        if not os.path.exists(f): continue
        d = json.load(open(f)); tables = []
        for t in d:
            cells = t['cells']
            nr = max(max(c['row_nums']) for c in cells) + 1 if cells else 0
            rows = [[] for _ in range(nr)]
            for c in sorted(cells, key=lambda c: (min(c['row_nums']), min(c['column_nums']))):
                rows[min(c['row_nums'])].append([len(c['column_nums']), len(c['row_nums']), c.get('pdf_text_content') or c.get('xml_text_content') or ''])
            tables.append(dict(page=t['pdf_page_index'], bbox=t['pdf_table_bbox'], rows=rows,
                               excl_det=t['exclude_for_detection'], excl_str=t['exclude_for_structure'], pdf_file_name=t['pdf_file_name']))
        out[n] = dict(pages=npages(geom_dir, n), tables=tables)
    return out

if __name__ == '__main__':
    kind, src, geom, dst = sys.argv[1:5]
    r = {'odl': odl, 'icdar': icdar, 'pt1m': pt1m}[kind](src, geom)
    json.dump(r, open(dst, 'w'), ensure_ascii=False)
    print(len(r), 'docs', sum(len(v['tables']) for v in r.values()), 'tables', sum(v['pages'] or 0 for v in r.values()), 'pages')
