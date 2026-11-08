#!/usr/bin/env python3
"""SP8 text gates over `plaindoc parse -o DIR` Markdown outputs (corpora stay outside the repo).

python -I eval_text.py --synth-md DIR --synth-gt DIR --odl-md DIR --odl-gt DIR [--json OUT] [--base BASE.json]
Metrics as in the ODL comparison (odl-bench eval_text.py): cer = 1 - normalised char edit distance,
order = word LCS, headings = '#' lines matched fuzzy >= 0.8, recall/precision averaged per document
(Pm = micro precision: matched '#' lines / all '#' lines of the slice; the per-document mean only
counts documents with at least one '#').
Slices: ODL all / ODL two-column (lists/odl_twocol.json) / synth two-column / synth one-column.
Decoys: false headings per synthetic decoy document. Needs rapidfuzz.
"""
import argparse, collections, glob, json, os, re, statistics
from rapidfuzz import fuzz
from rapidfuzz.distance import LCSseq, Levenshtein

HERE = os.path.dirname(os.path.abspath(__file__))
IMG = re.compile(r'!\[[^\]]*\]\([^)]*\)'); TAG = re.compile(r'<[^>\n]+>'); MD = re.compile(r'[#*_`>|~\\]+'); WS = re.compile(r'\s+')
DECOYS = ('toc', 'toc_dots', 'form', 'aligned_list', 'headers_footers', 'svg_chart', 'formulas', 'code', 'boxes', 'twocol_prose')


# `[text](url)` links (not images) reduced to their text: the renderer percent-encodes `()` and
# whitespace in the destination and escapes backslash, brackets, backtick and `<` in the label
LINK = re.compile(r'(?<!!)\[((?:\\.|[^\[\]\\])*)\]\((?:[^()\s\\]|%[0-9A-Fa-f]{2})*\)')


def unlink(t):
    return LINK.sub(lambda m: re.sub(r'\\([\\\[\]`<])', r'\1', m.group(1)), t)


def strip_md(t):
    t = unlink(t)
    t = re.sub(r'^(\s*\d{1,9})\\([.)])', r'\1\2', t, flags=re.M)  # paragraph start escaped by the renderer
    t = IMG.sub(' ', t); t = TAG.sub(' ', t)
    t = re.sub(r'^\s*[-:| ]+\s*$', ' ', t, flags=re.M); t = re.sub(r'^\s*[-*+]\s+', ' ', t, flags=re.M)
    # a bullet glyph opening a line is a list marker too (ODL GT writes `- · item` / `· item`; a PDF
    # list item drops its glyph): SP8b S1. Check boxes (`■□○`) are text: not here, so a lost one shows
    t = re.sub(r'^\s*[•·●▪◦]\s*', ' ', t, flags=re.M)
    return WS.sub(' ', MD.sub(' ', t).replace('\xa0', ' ')).strip()


def headings(t):
    return [WS.sub(' ', re.sub(r'[#*_`]+', ' ', unlink(l))).strip().lower() for l in t.splitlines() if re.match(r'^\s*#{1,6}\s', l)]


def score(gt, out):
    a, b = strip_md(gt), strip_md(out); wa, wb = a.lower().split(), b.lower().split()
    gh, oh = headings(gt), headings(out)
    tp_g = sum(1 for h in gh if any(fuzz.ratio(h, x) >= 80 for x in oh)); tp_o = sum(1 for x in oh if any(fuzz.ratio(h, x) >= 80 for h in gh))
    return dict(cer=Levenshtein.normalized_similarity(a, b), order=LCSseq.normalized_similarity(wa, wb),
                hr=tp_g / len(gh) if gh else None, hp=tp_o / len(oh) if oh else None, nh=len(oh), fh=len(oh) - tp_o)


def outputs(d):
    return {os.path.basename(f)[:-len('.pdf.md')]: f for f in glob.glob(d + '/**/*.pdf.md', recursive=True)}


def corpus(md, gt, docs=None):
    o = outputs(md); rows = {}
    for g in sorted(glob.glob(gt + '/*.md')):
        doc = os.path.basename(g)[:-3]
        if docs is None or doc in docs:
            rows[doc] = score(open(g, errors='replace').read(), open(o[doc], errors='replace').read() if doc in o else '')
    return rows


def summary(name, rows):
    c = [r['cer'] for r in rows.values()]; o = [r['order'] for r in rows.values()]
    hr = [r['hr'] for r in rows.values() if r['hr'] is not None]; hp = [r['hp'] for r in rows.values() if r['hp'] is not None]
    nh = sum(r['nh'] for r in rows.values()); tp = nh - sum(r['fh'] for r in rows.values())
    print(f"{name:12s} N={len(rows):3d} cer {statistics.median(c):.3f}/{statistics.mean(c):.3f} order {statistics.median(o):.3f}/{statistics.mean(o):.3f} "
          f"cer<0.5: {sum(x < 0.5 for x in c):3d} headings R {statistics.mean(hr) if hr else 0:.3f} P {statistics.mean(hp) if hp else 0:.3f} "
          f"Pm {tp / nh if nh else 0:.3f} ({tp}/{nh})")


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    for k in ('synth-md', 'synth-gt', 'odl-md', 'odl-gt'): ap.add_argument('--' + k, required=True)
    ap.add_argument('--json'); ap.add_argument('--base')
    a = ap.parse_args(); L = lambda n: set(json.load(open(f'{HERE}/lists/{n}.json')))
    R = {'odl_all': corpus(a.odl_md, a.odl_gt), 'odl_twocol': corpus(a.odl_md, a.odl_gt, L('odl_twocol')),
         'syn_twocol': corpus(a.synth_md, a.synth_gt, L('synth_twocol')), 'syn_onecol': corpus(a.synth_md, a.synth_gt, L('synth_onecol'))}
    for k, v in R.items(): summary(k, v)
    dec = collections.defaultdict(list)
    for d, r in R['syn_onecol'].items():
        kind = d.split('_', 2)[2]
        if kind in DECOYS: dec[kind].append(r['fh'])
    print('decoys: false headings per document (max):', {k: max(v) for k, v in sorted(dec.items())})
    if a.base:
        B = json.load(open(a.base))
        for sl in ('odl_all', 'syn_onecol'):
            worse = [d for d in R[sl] if d in B[sl] and R[sl][d]['cer'] < B[sl][d]['cer'] - 0.05]
            print(f'{sl}: cer worse by > 0.05 than base: {len(worse)} {worse[:10]}')
    if a.json: json.dump(R, open(a.json, 'w'))
