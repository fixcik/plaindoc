"""Эталон текста/заголовков для синтетики из исходного HTML (corpus-synth/work/html): порядок HTML = логический порядок чтения.
python -I synth_gt.py HTML_DIR OUT_DIR  -> OUT_DIR/<doc>.md  (h1/h2 -> '# ', '## '; таблицы -> строки через ' | ' (как GFM); остальное — абзацы)"""
import sys, os, re, glob, html
H, O = sys.argv[1], sys.argv[2]; os.makedirs(O, exist_ok=True)
for f in sorted(glob.glob(H + '/*.html')):
    s = open(f, errors='replace').read()
    body = s.split('<body', 1)[1].split('>', 1)[1] if '<body' in s else s
    body = re.sub(r'<style.*?</style>', ' ', body, flags=re.S); body = re.sub(r'<svg.*?</svg>', ' ', body, flags=re.S)
    body = re.sub(r'<h1[^>]*>(.*?)</h1>', lambda m: '\n\n# ' + re.sub(r'<[^>]+>', ' ', m.group(1)) + '\n\n', body, flags=re.S)
    body = re.sub(r'<h2[^>]*>(.*?)</h2>', lambda m: '\n\n## ' + re.sub(r'<[^>]+>', ' ', m.group(1)) + '\n\n', body, flags=re.S)
    body = re.sub(r'</(tr|li|div|p|caption)>', '\n', body); body = re.sub(r'</t[dh]>', ' | ', body)
    body = re.sub(r'<br\s*/?>', '\n', body)
    body = re.sub(r'<[^>]+>', ' ', body); body = html.unescape(body)
    lines = [re.sub(r'[ \t\xa0]+', ' ', l).strip() for l in body.splitlines()]
    out = '\n'.join(l for l in lines if l)
    open(f'{O}/{os.path.basename(f)[:-5]}.md', 'w').write(out + '\n')
print(len(glob.glob(O + '/*.md')))
