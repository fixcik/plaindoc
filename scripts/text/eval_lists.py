#!/usr/bin/env python3
"""List-item completeness of `plaindoc parse -o DIR` Markdown against GT Markdown.

python -I eval_lists.py MD_DIR GT_DIR [-v]
A list item is a Markdown line `- x`, `* x`, `+ x` or `N. x` / `N) x` (any indent; a page rule `---`
is not one). Item text = the line without its marker, as in eval_text.strip_md, lower case.
A leading `·`/`•` left in the item text (ODL GT writes `- · text`) is dropped.
Recall: GT items with an output item of the same document at fuzz.ratio >= 80, either whole or
cut to the GT item's length (ODL GT often keeps only an item's first line); each output item
matches once. Precision: matched output items / all output items (documents without GT items
included, so list items invented on prose count against it). Needs rapidfuzz.
"""
import glob, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rapidfuzz import fuzz  # noqa: E402
from eval_text import strip_md  # noqa: E402

ITEM = re.compile(r'^\s*(?:[-*+]|\d{1,9}[.)])\s+(\S.*)$')


def items(md):
    md = re.sub(r'<!--.*?-->', '\n', md, flags=re.S)
    out = []
    for line in md.splitlines():
        m = ITEM.match(line)
        if m:
            t = strip_md(m.group(1)).lower().lstrip('·• ')
            if t:
                out.append(t)
    return out


def sim(x, y):
    return max(fuzz.ratio(x, y), fuzz.ratio(x, y[:len(x)]) if len(x) >= 10 else 0)


def score(gt_md, out_md):
    g, o = items(gt_md), items(out_md)
    used = [False] * len(o)
    tp = 0
    for x in g:
        best = max(((sim(x, y), j) for j, y in enumerate(o) if not used[j]), default=(0, -1))
        if best[0] >= 80:
            used[best[1]] = True
            tp += 1
    return tp, len(g), len(o)


def main(md, gt, verbose=False):
    outs = {os.path.basename(f)[:-len('.pdf.md')]: f for f in glob.glob(md + '/**/*.pdf.md', recursive=True)}
    tp = ng = no = 0; docs = 0
    for g in sorted(glob.glob(gt + '/*.md')):
        d = os.path.basename(g)[:-3]
        if d not in outs:
            continue
        a, b, c = score(open(g, errors='replace').read(), open(outs[d], errors='replace').read())
        tp += a; ng += b; no += c; docs += 1
        if verbose and (b or c):
            print(f'  {d}: tp {a} gt {b} out {c}')
    r = tp / ng if ng else 0; p = tp / no if no else 0
    print(f'docs {docs} list items: R {r:.3f} ({tp}/{ng}) P {p:.3f} ({tp}/{no})')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], '-v' in sys.argv[3:])
