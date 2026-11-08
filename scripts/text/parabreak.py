#!/usr/bin/env python3
"""Paragraph-break P/R/F1 of `plaindoc parse -o DIR` Markdown against GT Markdown.

python -I parabreak.py MD_DIR GT_DIR   (MINW=n env: count only breaks between GT blocks of >= n words)
GT and output are cut into blocks: blank lines, plus list items, headings and table rows (one block per
line). Words are aligned with difflib (lower case, text as in eval_text.strip_md); an output break maps
to the GT position through the nearest matched word on its left; a break matches within TOL words.
"""
import difflib, glob, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eval_text import strip_md  # noqa: E402

TOL = 1
LIST_LINE = re.compile(r'^\s*(?:[-*+•●▪◦■–]\s|\d{1,3}[.)]\s|#{1,6}\s|\|)')


def blocks(md):
    md = re.sub(r'<!--.*?-->', '\n\n', md, flags=re.S)
    md = re.sub(r'^---[ \t]*$', '', md, flags=re.M)  # page rule between pages (replaced the page comment)
    out = []
    for chunk in re.split(r'\n\s*\n', md):
        cur = []
        for line in chunk.splitlines():
            if LIST_LINE.match(line) and cur:
                out.append(' '.join(cur)); cur = []
            cur.append(line)
        if cur:
            out.append(' '.join(cur))
    return out


MINW = 0  # > 0: "prose" only, a break counts when both neighbouring GT blocks have >= MINW words


def breaks(md, prose=False):
    """prose=True (GT only): breaks between blocks of >= MINW words and the set of "prose" words."""
    words, br, sizes = [], [], []
    for b in blocks(md):
        w = strip_md(b).lower().split()
        if not w:
            continue
        words += w
        br.append(len(words) - 1); sizes.append(len(w))
    keep, zone = set(), set()
    start = 0
    for i, e in enumerate(br):
        if sizes[i] >= MINW:
            zone.update(range(start, e + 1))
        start = e + 1
    for i in range(len(br) - 1):
        if MINW and (sizes[i] < MINW or sizes[i + 1] < MINW):
            continue
        keep.add(br[i])
    return (words, keep, zone) if prose else (words, keep)


def score(gt_md, out_md):
    """-> (tp_p, n_p, tp_r, n_r)"""
    gw, gb, zone = breaks(gt_md, prose=True)
    m0 = globals()['MINW']; globals()['MINW'] = 0
    ow, ob = breaks(out_md)
    globals()['MINW'] = m0
    sm = difflib.SequenceMatcher(None, gw, ow, autojunk=False)
    o2g, g2o = {}, {}
    for a, b, n in sm.get_matching_blocks():
        for k in range(n):
            o2g[b + k] = a + k; g2o[a + k] = b + k

    def near(m, i):
        for d in range(0, 4):
            if i - d in m:
                return m[i - d] + d
        return None
    tp_p = n_p = 0
    for j in ob:
        g = near(o2g, j)
        if g is None:
            continue
        if m0 and not (g in zone and g + 1 in zone):
            continue  # outside GT prose (chart labels, tables, short lines)
        n_p += 1
        if any(g + d in gb for d in range(-TOL, TOL + 1)):
            tp_p += 1
    tp_r = n_r = 0
    for g in gb:
        o = near(g2o, g)
        if o is None:
            continue
        n_r += 1
        if any(o + d in ob for d in range(-TOL, TOL + 1)):
            tp_r += 1
    return tp_p, n_p, tp_r, n_r


def main(md, gt):
    outs = {os.path.basename(f)[:-len('.pdf.md')]: f for f in glob.glob(md + '/**/*.pdf.md', recursive=True)}
    a = [0, 0, 0, 0]; n = 0
    for g in sorted(glob.glob(gt + '/*.md')):
        d = os.path.basename(g)[:-3]
        if d not in outs:
            continue
        s = score(open(g, errors='replace').read(), open(outs[d], errors='replace').read())
        a = [x + y for x, y in zip(a, s)]; n += 1
    tp, np_, tr, nr = a
    p = tp / np_ if np_ else 0; r = tr / nr if nr else 0; f = 2 * p * r / (p + r) if p + r else 0
    print(f'docs {n} P {p:.3f} R {r:.3f} F1 {f:.3f} (tp_p {tp}/{np_}, tp_r {tr}/{nr})')


if __name__ == '__main__':
    MINW = int(os.environ.get('MINW', '0'))
    main(sys.argv[1], sys.argv[2])
