#!/usr/bin/env python3
"""SP8 hidden-text gate: share of characters removed or flagged, by rule, over `parse -f json -o DIR`.

python -I eval_corpus.py JSON_DIR...   (one line per corpus; gate: removed <= 0.06 % besides offpage)
"""
import collections, glob, json, re, sys

for d in sys.argv[1:]:
    rem, flag, files = collections.Counter(), collections.Counter(), collections.defaultdict(set)
    chars = 0
    for f in glob.glob(d + '/**/*.pdf.json', recursive=True):
        doc = json.load(open(f))
        for s in doc['sections']:
            chars += sum(len(re.sub(r'\s', '', b['text'])) for b in s['blocks'] if 'text' in b)
            for w in s['warnings']:
                if w['kind'] == 'HiddenContent':
                    tgt = rem if w['message'].startswith('hidden text removed') else flag
                    for r, n in re.findall(r'(\w+)=(\d+) chars', w['message']):
                        tgt[r] += int(n); files[r].add(f)
    pct = lambda c: 100 * sum(c.values()) / max(chars, 1)
    print(f'{d}: chars {chars}; removed {dict(rem)} = {pct(rem):.4f} %; flagged {dict(flag)} = {pct(flag):.4f} %; '
          f'files {dict((k, len(v)) for k, v in files.items())}')
