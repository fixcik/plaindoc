#!/usr/bin/env python3
"""SP8 running header/footer gates.

python -I eval_running.py pt1m --ours JSON_DIR --odl ODL_HF_RAW_DIR [--sample 50]
    pages with an ODL header/footer node (`--include-header-footer`), minus pages where every ODL
    mark is a first occurrence in the document (kept as body text by design), that carry one of our
    Header/Footer/PageNumber notes; prints a random sample of our new marks for manual review.
python -I eval_running.py synth --ours JSON_DIR --base JSON_DIR
    synthetic multipage documents have no running heads: every new Header/Footer note (`parse -f json`)
    is false. Markdown no longer carries them by default (SP8 ruling В-2).
"""
import argparse, glob, json, os, random, re

mask = lambda t: re.sub(r'\d+', '#', re.sub(r'\s+', ' ', t).strip().lower())


def odl_marks(raw):
    m = {}
    def walk(k):
        for nd in k:
            if nd.get('type') in ('header', 'footer'):
                txt = ' '.join((x.get('content') or '') for x in nd['kids']) if nd.get('kids') else (nd.get('content') or '')
                m.setdefault(nd.get('page number', 1) - 1, []).append(txt)
            elif nd.get('kids'): walk(nd['kids'])
    walk(raw.get('kids', []))
    return m


def notes(f):
    d = json.load(open(f))
    return [(s['index'], b['kind'], b['text']) for s in d['sections'] for b in s['blocks']
            if b.get('type') == 'note' and b.get('kind') in ('header', 'footer', 'page_number')]


def pt1m(a):
    tot = hit = 0; marks = []
    for f in sorted(glob.glob(a.ours + '/**/*.pdf.json', recursive=True)):
        doc = os.path.basename(f)[:-len('.pdf.json')]; ours = notes(f); marks += [(doc,) + x for x in ours]
        o = f'{a.odl}/{doc}.json'
        if not os.path.exists(o): continue
        m = odl_marks(json.load(open(o))); seen = set(); pages = set()
        for p in sorted(m):
            keys = {mask(t) for t in m[p]}
            if keys <= seen: pages.add(p)
            seen |= keys
        tot += len(pages); hit += len(pages & {p for p, _, _ in ours})
    print(f'ODL pages (first occurrences excluded): {tot}, with our notes: {hit} = {hit / max(tot, 1):.4f}')
    random.seed(7)
    for x in random.sample(marks, min(a.sample, len(marks))): print(x)


def synth(a):
    def heads(d):
        return {os.path.basename(f)[:-len('.pdf.json')]: [t for _, k, t in notes(f) if k in ('header', 'footer')]
                for f in glob.glob(d + '/**/*.pdf.json', recursive=True)}
    o, b = heads(a.ours), heads(a.base)
    docs = sorted(d for d in o if 'multipage' in d or len(o[d]) > len(b.get(d, [])))
    false = [d for d in docs if len(o[d]) > len(b.get(d, []))]
    print(f'documents with new header/footer notes: {len(false)} {false}')


if __name__ == '__main__':
    ap = argparse.ArgumentParser(); ap.add_argument('corpus', choices=['pt1m', 'synth'])
    ap.add_argument('--ours', required=True); ap.add_argument('--odl'); ap.add_argument('--base'); ap.add_argument('--sample', type=int, default=50)
    a = ap.parse_args(); pt1m(a) if a.corpus == 'pt1m' else synth(a)
