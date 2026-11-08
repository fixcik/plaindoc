"""Сборка HTML-документов (таблицы разных стилей + декои) из пула таблиц.
python3 -I compose.py POOL.jsonl OUT_HTML_DIR PLAN.jsonl --generated-only
План (PLAN.jsonl) содержит для каждого документа логический эталон таблиц и атрибуты срезов."""
import json, sys, os, random, html, re
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pool import ctext, WS

R = random.Random(7)
E = lambda s: html.escape(s, quote=True)
STYLES = ['grid', 'grid_fill', 'thin', 'borderless', 'zebra', 'hlines', 'hlines_all', 'double', 'rounded', 'rounded_cells']
BORDERLESS = {'borderless', 'zebra', 'hlines', 'hlines_all'}
CSS = """
@page { size: A4; margin: 18mm 16mm; }
@page land { size: A4 landscape; margin: 12mm 10mm; }
body { font-family: 'Times New Roman', 'PT Serif', serif; font-size: 11pt; color: #111; background: #fff; hyphens: manual; }
h1 { font-size: 15pt; } h2 { font-size: 12.5pt; margin: 14px 0 6px; }
p { text-align: justify; margin: 5px 0; line-height: 1.35; }
table { border-collapse: collapse; margin: 8px 0 12px; font-size: 10pt; }
td, th { padding: 3px 5px; vertical-align: top; text-align: left; }
th { font-weight: bold; }
tr { break-inside: avoid; }
.cap { font-weight: bold; margin-top: 12px; text-align: left; }
.note { font-size: 8.5pt; margin-top: -6px; }
.t-grid td, .t-grid th { border: 1px solid #000; }
.t-grid_fill td, .t-grid_fill th { border: 0.75px solid #333; } .t-grid_fill th { background: #d9e2f3; }
.t-thin td, .t-thin th { border: 0.25px solid #666; }
.t-borderless td, .t-borderless th { padding: 3px 12px 3px 0; }
.t-zebra th { background: #c9c9c9; } .t-zebra tbody tr:nth-child(even) td { background: #ececec; } .t-zebra td, .t-zebra th { padding: 3px 8px; }
.t-hlines { border-top: 2px solid #000; border-bottom: 2px solid #000; } .t-hlines thead tr:last-child th { border-bottom: 1px solid #000; }
.t-hlines td, .t-hlines th { padding: 3px 10px; }
.t-hlines_all td, .t-hlines_all th { border-bottom: 0.5px solid #777; padding: 3px 9px; } .t-hlines_all thead th { border-bottom: 1.5px solid #000; }
.t-double { border: 3px double #000; } .t-double td, .t-double th { border: 3px double #000; }
.t-rounded { border-collapse: separate; border-spacing: 0; border: 1px solid #444; border-radius: 10px; }
.t-rounded td, .t-rounded th { border-right: 1px solid #444; border-bottom: 1px solid #444; }
.t-rounded_cells { border-collapse: separate; border-spacing: 3px; }
.t-rounded_cells td, .t-rounded_cells th { border: 1px solid #555; border-radius: 6px; }
.small table { font-size: 7pt; } .small td, .small th { padding: 1px 3px; }
.wide table { font-size: 6.5pt; } .wide td, .wide th { padding: 1px 2px; }
.narrow td, .narrow th { max-width: 34mm; }
.twocol { column-count: 2; column-gap: 8mm; } .twocol table { break-inside: avoid; font-size: 8.5pt; width: 100%; }
th.rot { writing-mode: vertical-rl; transform: rotate(180deg); white-space: nowrap; vertical-align: bottom; text-align: left; }
.land { page: land; }
"""
WORDS = ('в рамках реализации программы показатели отчётного периода существенно изменились относительно прошлого года '
         'данные указаны на последний день квартала с учётом уточнений органов статистики расчёт выполнен '
         'на основании первичных документов бухгалтерского учёта и сведений представленных подразделениями '
         'наблюдается устойчивая тенденция к росту объёмов при сохранении структуры затрат значения округлены '
         'методика сопоставима с применявшейся ранее отдельные позиции требуют дополнительной проверки').split()
SENT = ['Сведения сгруппированы по основным направлениям деятельности.', 'Подробная расшифровка приведена в приложении к настоящему отчёту.',
        'Значения показателей указаны без учёта налога на добавленную стоимость, если не оговорено иное.',
        'Расхождения между итоговыми суммами объясняются округлением.', 'Ответственный исполнитель обеспечивает контроль за сроками.',
        'Согласно пункту 4.2 договора, оплата производится в течение десяти рабочих дней.']
def prose(n=3):
    out = []
    for _ in range(n):
        k = R.randint(12, 30); w = [R.choice(WORDS) for _ in range(k)]
        s = ' '.join(w); s = s[0].upper() + s[1:] + '. ' + R.choice(SENT)
        out.append('<p>%s</p>' % E(s))
    return '\n'.join(out)

def gt_rows(t):
    return [[dict(t=ctext(c), cs=c['cs'], rs=c['rs']) for c in r] for r in t['head'] + t['body']]

def cell_html(c, rot=False):
    tag = 'th' if c['th'] else 'td'
    a = ''
    if c['cs'] > 1: a += ' colspan="%d"' % c['cs']
    if c['rs'] > 1: a += ' rowspan="%d"' % c['rs']
    if rot: a += ' class="rot"'
    inner = '<br>'.join(E(x) for x in c['lines'])
    if c.get('inner'):
        inner += table_html(c['inner'], 'grid', inner=True)
    return '<%s%s>%s</%s>' % (tag, a, inner, tag)

def table_html(t, style, rotate=False, caption=None, inner=False):
    h = ['<table class="t-%s"%s>' % (style, ' style="margin:2px 0;font-size:8pt"' if inner else '')]
    if caption: h.append('<caption class="cap">%s</caption>' % E(caption))
    h.append('<thead>')
    for ri, r in enumerate(t['head']):
        h.append('<tr>' + ''.join(cell_html(c, rotate and ri == len(t['head']) - 1 and len(ctext(c)) <= 28 and c is not r[0]) for c in r) + '</tr>')
    h.append('</thead><tbody>')
    for r in t['body']: h.append('<tr>' + ''.join(cell_html(c) for c in r) + '</tr>')
    h.append('</tbody></table>')
    return ''.join(h)

def page(body, cls='', title='Документ'):
    return '<!doctype html><html lang="ru"><head><meta charset="utf-8"><title>%s</title><style>%s</style></head><body class="%s">%s</body></html>' % (E(title), CSS, cls, body)

def bucket_cols(n): return '2-4' if n <= 4 else '5-8' if n <= 8 else '9-14' if n <= 14 else '15-30'
def bucket_rows(n): return '<=10' if n <= 10 else '11-30' if n <= 30 else '31-60' if n <= 60 else '>60'

def tab_entry(t, style, **kw):
    f = t['f']
    a = dict(table_id=t['id'], source=t['source'], lang='ru', style=style, borderless=style in BORDERLESS,
             merged=f['merged'], multihead=f['multihead'], columns=f['ncol'], rows=f['nrow'], numeric=f['numeric'],
             col_bucket=bucket_cols(f['ncol']), row_bucket=bucket_rows(f['nrow']), rotated_header=False, nested=False,
             nested_inner=False, parent=None, caption=False, layout='plain', head_rows=len(t['head']), attrib=t['attrib'],
             multi_page_intended=False, nbsp_thin=any(' ' in x for r in t['body'] for c in r for x in c['lines']))
    a.update(kw); a['gt_rows'] = gt_rows(t)
    return a

def main(pool_path, outdir, plan_path, generated_only=False):
    os.makedirs(outdir, exist_ok=True)
    pool = [json.loads(l) for l in open(pool_path)]
    if not generated_only:
        raise SystemExit('refusing to compose without --generated-only (only source=generated tables are allowed)')
    gen = [t for t in pool if t['source'] == 'generated']
    G = lambda k: [t for t in gen if t['id'].startswith('gen_' + k + '_')]
    docs = []; si = [0]
    def nst():
        s = STYLES[si[0] % len(STYLES)]; si[0] += 1; return s
    def take(lst):
        return lst.pop() if lst else None
    def add(kind, tables, cls='', land=False, pre='', decoy=None, extra_html=None):
        docs.append(dict(kind=kind, tables=tables, cls=cls, land=land, pre=pre, decoy=decoy, extra_html=extra_html))
    narrow = lambda: [t for t in G('reg') if t['f']['ncol'] <= 10]
    # ---- ordinary: 1-2 tables per document ----
    simple_srcs = [G('invoice'), G('act'), G('cp'), G('payroll'), G('grp'), narrow()]
    for i in range(130):
        src = simple_srcs[i % len(simple_srcs)]
        k = 1 if i % 3 else 2
        ts = [x for x in (take(src) for _ in range(k)) if x]
        if ts: add('plain', [(t, nst(), {}) for t in ts])
    # ---- caption + footnote ----
    for i in range(40):
        t = take(G('invoice') or G('reg') or G('cp'))
        if t: add('caption', [(t, nst(), dict(caption=True))])
    # ---- small font ----
    for i in range(20):
        t = take([t for t in G('reg') if t['f']['ncol'] <= 12])
        if t: add('small', [(t, nst(), dict(layout='small_font'))], cls='small')
    # ---- multiline ----
    for i in range(30):
        t = take(G('ml'))
        if t: add('multiline', [(t, nst(), dict(layout='multiline'))], cls='narrow')
    # ---- rotated header ----
    rot_src = G('reg') + G('ts')[:6]
    for i in range(26):
        t = take(rot_src)
        if t: add('rotated', [(t, nst(), dict(rotated_header=True))], cls='small' if t['f']['ncol'] > 12 else '', land=t['f']['ncol'] > 12)
    # ---- nested ----
    for i in range(24):
        t = take(G('nested'))
        if t: add('nested', [(t, ['grid', 'grid_fill', 'thin', 'double', 'rounded', 'hlines_all'][i % 6], dict(nested=True))])
    # ---- wide (15-30 columns), landscape ----
    wide_src = G('reg') + G('ts')
    for i in range(36):
        t = take(wide_src)
        if t: add('wide', [(t, nst(), dict(layout='wide'))], cls='wide', land=True)
    # ---- multipage with repeated header ----
    long_src = [t for t in G('long') + G('payroll') if t['f']['nrow'] >= 55]
    for i in range(40):
        t = take(long_src)
        if not t: continue
        land = i % 5 == 0
        add('multipage', [(t, nst(), dict(multi_page_intended=True, layout='multipage_land' if land else 'multipage'))], land=land,
            cls='small' if i % 7 == 3 else '')
    # ---- decoys ----
    for d in ['toc', 'toc_dots', 'aligned_list', 'twocol_prose', 'code', 'formulas', 'boxes', 'svg_chart', 'headers_footers', 'form']:
        for i in range(8): add('decoy', [], decoy=d)
    # ---- запись ----
    plan = open(plan_path, 'w'); n_tab = 0
    for di, d in enumerate(docs):
        name = 'syn_%03d_%s' % (di, d['decoy'] or d['kind'])
        body = ['<h1>%s</h1>' % E(['Отчёт о деятельности', 'Пояснительная записка', 'Справка', 'Приложение № %d' % (di % 9 + 1), 'Аналитический обзор'][di % 5])]
        entries = []
        if d['decoy']:
            body.append(decoy_html(d['decoy'], di))
        else:
            if d['kind'] == 'twocol': body.append(prose(1) + '<div class="twocol">' + prose(2))
            else: body.append(prose(2))
            for ti, (t, style, kw) in enumerate(d['tables']):
                kw = dict(kw)
                if d['kind'] == 'mixed_orient' and ti == 1:
                    body.append('<div class="land small">')
                if t['title']: body.append('<h2>%s</h2>' % E(t['title'][:120]))
                cap = None
                if kw.get('caption'):
                    capt = 'Таблица %d. %s' % (ti + 1, t['title'][:90] or 'Сводные данные')
                    if di % 2: body.append('<p class="cap">%s</p>' % E(capt))
                    else: cap = capt
                body.append(table_html(t, style, rotate=kw.get('rotated_header'), caption=cap))
                if kw.get('caption'):
                    body.append('<p class="note">%s</p>' % E(R.choice(['Примечание. Данные предварительные; по ряду позиций возможны уточнения.',
                                                                     '* Источник: расчёты автора по данным отчётности.', '¹ Без учёта НДС. ² По состоянию на 01.01.2026.'])))
                if d['kind'] == 'mixed_orient' and ti == 1: body.append('</div>')
                if d['kind'] == 'wide': kw.setdefault('layout', 'wide')
                e = tab_entry(t, style, **kw)
                if d['land'] or (d['kind'] == 'mixed_orient' and ti == 1): e['landscape'] = True
                entries.append(e)
                for r in t['head'] + t['body']:
                    for c in r:
                        if c.get('inner'):
                            entries.append(tab_entry(c['inner'], 'grid', nested_inner=True, parent=t['id'], nested=True))
                body.append(prose(1 if d['kind'] != 'plain' else R.randint(1, 2)))
            if d['kind'] == 'twocol': body.append(prose(2) + '</div>')
        cls = d['cls'] + (' landdoc' if d['land'] else '')
        htmls = page('\n'.join(body), cls)
        if d['land']: htmls = htmls.replace('@page { size: A4; margin: 18mm 16mm; }', '@page { size: A4 landscape; margin: 12mm 10mm; }')
        open('%s/%s.html' % (outdir, name), 'w').write(htmls)
        n_tab += sum(1 for e in entries if not e['nested_inner'])
        plan.write(json.dumps(dict(doc=name, kind=d['kind'], decoy=d['decoy'], landscape=d['land'], tables=entries), ensure_ascii=False) + '\n')
    print('docs', len(docs), 'tables', n_tab)

# ---------------- декои ----------------
def decoy_html(kind, i):
    R2 = random.Random(1000 + i * 31 + sum(map(ord, kind)))
    secs = ['Введение', 'Общие положения', 'Термины и определения', 'Порядок расчёта', 'Анализ результатов', 'Экономические итоги',
            'Кадровое обеспечение', 'Риски и меры по их снижению', 'Заключение', 'Список литературы', 'Приложение А', 'Приложение Б']
    if kind in ('toc', 'toc_dots'):
        rows = []; pg = 3
        for k, s in enumerate(secs):
            pg += R2.randint(1, 6)
            for sub in ([''] + ['.%d' % j for j in range(1, R2.randint(1, 4))]):
                lab = '%d%s %s' % (k + 1, sub, s if not sub else R2.choice(['Цели и задачи', 'Исходные данные', 'Методика', 'Результаты', 'Выводы']))
                if kind == 'toc_dots':
                    rows.append('<div style="display:flex"><span>%s</span><span style="flex:1;overflow:hidden;white-space:nowrap">%s</span><span>%d</span></div>' % (E(lab), ' .' * 120, pg))
                else:
                    rows.append('<div style="display:flex;margin-left:%dmm"><span style="flex:1;border-bottom:1px dotted #000">%s</span><span style="width:12mm;text-align:right">%d</span></div>' % (6 if sub else 0, E(lab), pg))
        return '<h2>Содержание</h2>' + ''.join(rows)
    if kind == 'aligned_list':
        keys = ['Полное наименование', 'Сокращённое наименование', 'ИНН', 'КПП', 'ОГРН', 'Юридический адрес', 'Расчётный счёт', 'Банк', 'БИК', 'Корр. счёт', 'Генеральный директор', 'Телефон']
        vals = ['Общество с ограниченной ответственностью «Ромашка»', 'ООО «Ромашка»', '7701234567', '770101001', '1027700123456', '125009, г. Москва, ул. Тверская, д. 7',
                '40702810900000012345', 'ПАО Сбербанк', '044525225', '30101810400000000225', 'Иванов Андрей Сергеевич', '+7 (495) 123-45-67']
        h = '<h2>Реквизиты организации</h2>' + ''.join('<div><span style="display:inline-block;width:62mm">%s:</span><span>%s</span></div>' % (E(k), E(v)) for k, v in zip(keys, vals))
        h += '<h2>Перечень документов</h2><ol>' + ''.join('<li><span style="display:inline-block;width:110mm">%s</span>%s</li>' % (E(R2.choice(['Устав', 'Свидетельство о регистрации', 'Выписка из ЕГРЮЛ', 'Доверенность', 'Приказ о назначении'])), E('%d л.' % R2.randint(1, 30))) for _ in range(8)) + '</ol>'
        return h + prose(2)
    if kind == 'twocol_prose':
        return '<div class="twocol">' + prose(12) + '</div>'
    if kind == 'code':
        code = '''fn parse_row(line: &str) -> Option<Row> {
    let parts: Vec<&str> = line.split('|').collect();   // разбиение строки
    if parts.len() < 3 { return None; }                 // слишком мало полей
    let id    = parts[0].trim().parse::<u32>().ok()?;   // идентификатор
    let name  = parts[1].trim().to_string();            // наименование
    let sum   = parts[2].trim().replace(',', ".");      // сумма
    Some(Row { id, name, sum: sum.parse().ok()? })
}
SELECT region,  year,  SUM(amount) AS total   -- итог по региону
FROM   payments
WHERE  year BETWEEN 2020 AND 2025
GROUP  BY region, year
ORDER  BY total DESC;'''
        return prose(1) + '<pre style="font-family:Menlo,monospace;font-size:9pt;background:#f4f4f4;padding:6px;border:1px solid #ccc">%s</pre>' % E(code) + prose(2)
    if kind == 'formulas':
        f = ['<i>S</i> = <i>P</i> · (1 + <i>r</i>/<i>n</i>)<sup><i>n</i>·<i>t</i></sup>', '<i>НДС</i> = <i>Σ</i> · 20 / 120',
             '<i>x</i><sub>1,2</sub> = (−<i>b</i> ± √(<i>b</i><sup>2</sup> − 4<i>ac</i>)) / 2<i>a</i>', '<i>σ</i><sup>2</sup> = (1/<i>N</i>) Σ<sub><i>i</i>=1</sub><sup><i>N</i></sup> (<i>x</i><sub><i>i</i></sub> − <i>μ</i>)<sup>2</sup>']
        h = prose(1)
        for k, x in enumerate(f):
            h += '<div style="display:flex;margin:8px 0"><span style="flex:1;text-align:center">%s</span><span>(%d)</span></div>' % (x, k + 1)
            h += '<p>где %s.</p>' % E(R2.choice(['S — итоговая сумма, P — начальный капитал, r — ставка', 'Σ — сумма с налогом', 'a, b, c — коэффициенты уравнения', 'N — объём выборки, μ — среднее']))
        return h + prose(1)
    if kind == 'boxes':
        h = prose(1)
        h += '<div style="border:1.5px solid #333;border-radius:8px;padding:8px;margin:10px 0"><b>Важно!</b> %s</div>' % E(R2.choice(SENT) + ' ' + R2.choice(SENT))
        h += '<div style="display:flex;gap:6mm;margin:10px 0">' + ''.join('<div style="flex:1;border:1px solid #000;padding:6px"><b>%s</b><br>%s</div>' % (E(t), E(R2.choice(SENT))) for t in ['Цель', 'Задачи', 'Результат']) + '</div>'
        h += '<div style="border-left:4px solid #888;background:#f0f0f0;padding:6px 10px">%s</div>' % E(' '.join(R2.choice(WORDS) for _ in range(40)))
        return h + prose(2)
    if kind == 'svg_chart':
        vals = [R2.randint(10, 95) for _ in range(7)]; labs = ['2019', '2020', '2021', '2022', '2023', '2024', '2025']
        s = ['<svg width="520" height="300" xmlns="http://www.w3.org/2000/svg" font-family="Arial" font-size="11">']
        for k in range(0, 101, 20):
            y = 260 - k * 2.2
            s.append('<line x1="50" y1="%.1f" x2="500" y2="%.1f" stroke="#bbb" stroke-width="0.6"/><text x="44" y="%.1f" text-anchor="end">%d</text>' % (y, y, y + 4, k))
        for k, (v, l) in enumerate(zip(vals, labs)):
            x = 70 + k * 62
            s.append('<rect x="%d" y="%.1f" width="34" height="%.1f" fill="#4a7ab5"/><text x="%d" y="278" text-anchor="middle">%s</text><text x="%d" y="%.1f" text-anchor="middle">%d</text>' % (x, 260 - v * 2.2, v * 2.2, x + 17, l, x + 17, 255 - v * 2.2, v))
        s.append('<line x1="50" y1="260" x2="500" y2="260" stroke="#000"/><line x1="50" y1="20" x2="50" y2="260" stroke="#000"/>')
        s.append('<text x="275" y="296" text-anchor="middle">Год</text><text x="12" y="140" transform="rotate(-90 12 140)" text-anchor="middle">Объём, млн руб.</text></svg>')
        return prose(1) + '<p class="cap">Рисунок 1. Динамика объёма закупок</p>' + ''.join(s) + prose(2)
    if kind == 'headers_footers':
        hf = ('<div style="position:fixed;top:0;left:0;right:0;border-bottom:0.75px solid #000;font-size:8.5pt;display:flex;justify-content:space-between">'
              '<span>ООО «Ромашка»</span><span>Годовой отчёт за 2025 год</span><span>Редакция 2</span></div>'
              '<div style="position:fixed;bottom:0;left:0;right:0;border-top:0.75px solid #000;font-size:8.5pt;display:flex;justify-content:space-between">'
              '<span>Конфиденциально</span><span>Утверждено приказом № 12 от 15.01.2026</span><span>стр. 1</span></div>')
        return hf + '<div style="padding-top:8mm">' + prose(8) + '</div>'
    if kind == 'form':
        fld = ['Ф. И. О. заявителя', 'Дата рождения', 'Паспорт: серия, номер', 'Кем выдан', 'Адрес регистрации', 'Телефон', 'Подпись заявителя']
        h = '<h2>Заявление</h2>' + prose(1)
        h += ''.join('<div style="display:flex;margin:9px 0"><span style="width:58mm">%s</span><span style="flex:1;border-bottom:0.75px solid #000">&nbsp;</span></div>' % E(f) for f in fld)
        h += '<div style="display:flex;gap:10mm;margin-top:14px"><span style="flex:1;border-top:0.75px solid #000;font-size:8pt">(подпись)</span><span style="flex:1;border-top:0.75px solid #000;font-size:8pt">(расшифровка)</span><span style="flex:1;border-top:0.75px solid #000;font-size:8pt">(дата)</span></div>'
        return h
    raise ValueError(kind)

if __name__ == '__main__':
    main(*sys.argv[1:4], generated_only='--generated-only' in sys.argv)
