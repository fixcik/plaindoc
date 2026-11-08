"""Synthetic table pool: only own generated tables (source=generated, fixed seed); no third-party text.
Таблица: dict(id, source, attrib, title, head=[row], body=[row]); row = [cell]; cell = dict(lines=[str], cs, rs, th, inner=None|table)
Текст ячейки для эталона = ' '.join(lines) (нормализация пробелов как в tbl/metric.py, регистр сохраняется).
Usage: python3 -I pool.py OUT_POOL_JSONL"""
import json, re, sys, random

WS = re.compile(r'[\s  ]+')

def cell(text, cs=1, rs=1, th=False, lines=None):
    return dict(lines=lines if lines is not None else [text], cs=cs, rs=rs, th=th, inner=None)

def ctext(c):
    t = ' '.join(c['lines'])
    if c.get('inner'):
        t = (t + ' ' + ' '.join(ctext(x) for r in c['inner']['head'] + c['inner']['body'] for x in r))
    return WS.sub(' ', t).strip()

def grid_ok(rows):
    """Проверка прямоугольности сетки с учётом spans; возврат число колонок или None."""
    occ = set(); widths = []
    for ri, r in enumerate(rows):
        ci = 0
        for c in r:
            while (ri, ci) in occ: ci += 1
            for dr in range(c['rs']):
                for dc in range(c['cs']):
                    if (ri + dr, ci + dc) in occ: return None
                    occ.add((ri + dr, ci + dc))
            ci += c['cs']
    n = len(rows)
    for ri in range(n):
        w = max([k[1] for k in occ if k[0] == ri], default=-1) + 1
        cnt = sum(1 for k in occ if k[0] == ri)
        if cnt != w: return None
        widths.append(w)
    if any(k[0] >= n for k in occ): return None
    return widths[0] if widths and len(set(widths)) == 1 else None

def feats(t):
    rows = t['head'] + t['body']
    ncol = grid_ok(rows)
    merged = any(c['cs'] > 1 or c['rs'] > 1 for r in rows for c in r)
    body_rs = any(c['rs'] > 1 for r in t['body'] for c in r)
    txt = [ctext(c) for r in t['body'] for c in r if ctext(c)]
    num = sum(1 for x in txt if re.fullmatch(r'[-+−–]?[\d\s.,%]+', x)) / max(1, len(txt))
    t['f'] = dict(ncol=ncol, nrow=len(rows), merged=merged, body_rowspan=body_rs, multihead=len(t['head']) > 1,
                  numeric=num > 0.5, maxlen=max((len(ctext(c)) for r in rows for c in r), default=0),
                  ncells=sum(len(r) for r in rows))
    return t

# ---------------- generated ----------------
R = random.Random(20261107)
GOODS = ['Бумага офисная А4, 500 л.', 'Картридж лазерный HP 85A', 'Ручка шариковая синяя', 'Монитор 24" Full HD', 'Клавиатура USB',
         'Услуги по доставке груза', 'Стол письменный угловой', 'Кресло офисное', 'Тонер-картридж Kyocera TK-1170', 'Кабель UTP cat.5e, 305 м',
         'Ноутбук 15,6" Intel Core i5', 'Сервер стоечный 2U', 'Лицензия ПО (12 мес.)', 'Монтаж оборудования', 'Консультационные услуги',
         'Шкаф для документов', 'Скоросшиватель картонный', 'Папка-регистратор 75 мм', 'Вода питьевая 19 л', 'Огнетушитель ОП-4',
         'Цемент М500, мешок 50 кг', 'Кирпич облицовочный', 'Труба ПНД 32 мм', 'Электроды МР-3, 3 мм', 'Краска фасадная белая 10 л',
         'Аренда спецтехники (маш.-ч)', 'Техническое обслуживание котельной', 'Поверка счётчиков', 'Уборка помещений (м²)', 'Ремонт кровли']
UNITS = ['шт.', 'шт.', 'упак.', 'компл.', 'усл.', 'м', 'кг', 'л', 'ч', 'м²']
FAM = ['Иванов', 'Петрова', 'Сидоренко', 'Кузнецов', 'Смирнова', 'Попов', 'Васильева', 'Соколов', 'Михайлова', 'Новиков', 'Фёдорова',
       'Морозов', 'Волкова', 'Алексеев', 'Лебедева', 'Семёнов', 'Егорова', 'Павлов', 'Козлова', 'Степанов', 'Николаева', 'Орлов']
IO = ['А. С.', 'И. П.', 'Е. В.', 'О. Н.', 'М. Ю.', 'Т. А.', 'Д. К.', 'Н. Г.', 'С. Л.', 'В. Р.']
POS = ['Инженер', 'Бухгалтер', 'Менеджер', 'Водитель', 'Кладовщик', 'Ведущий специалист', 'Юрисконсульт', 'Оператор ЭВМ', 'Мастер участка']
ORG = ['ООО «Ромашка»', 'АО «СеверСтройМонтаж»', 'ИП Сидоренко Е. В.', 'ООО «Техносфера»', 'ПАО «Уралэнерго»', 'ООО «Вектор-Плюс»',
       'МУП «Водоканал»', 'ООО «Альфа Логистик»', 'ГБУ «Центр занятости»', 'ЗАО «Байкал-Сервис»']
REG = ['Москва', 'Санкт-Петербург', 'Новосибирская обл.', 'Свердловская обл.', 'Республика Татарстан', 'Краснодарский край',
       'Нижегородская обл.', 'Челябинская обл.', 'Самарская обл.', 'Ростовская обл.', 'Республика Бурятия', 'Пермский край',
       'Воронежская обл.', 'Волгоградская обл.', 'Красноярский край', 'Саратовская обл.', 'Тюменская обл.', 'Омская обл.']

def money(v, sep=' '):
    s = '%.2f' % v; a, b = s.split('.')
    g = []
    while len(a) > 3: g.insert(0, a[-3:]); a = a[:-3]
    g.insert(0, a)
    return sep.join(g) + ',' + b
def inn(n=10): return ''.join(str(R.randint(0, 9)) for _ in range(n))
def date(): return '%02d.%02d.%d' % (R.randint(1, 28), R.randint(1, 12), R.choice([2023, 2024, 2025, 2026]))
def fio(): return '%s %s' % (R.choice(FAM), R.choice(IO))
def H(*xs): return [cell(x, th=True) for x in xs]

def gen_invoice(i, nrows=None, sep=' '):
    n = nrows or R.randint(3, 14); body = []; tot = 0
    for k in range(n):
        q = R.choice([1, 2, 3, 5, 10, 12, 20, 50, 100, 250]); pr = R.choice([45.5, 120, 990, 1250.75, 3400, 15800, 64990, 129000.4])
        s = q * pr; tot += s
        body.append([cell(str(k + 1)), cell(R.choice(GOODS)), cell(R.choice(UNITS)), cell(str(q)), cell(money(pr, sep)), cell(money(s, sep))])
    vat = tot * 20 / 120
    body.append([cell('Итого:', cs=5), cell(money(tot, sep))])
    body.append([cell('В том числе НДС (20%):', cs=5), cell(money(vat, sep))])
    return dict(id='gen_invoice_%d' % i, source='generated', attrib=dict(kind='счёт'), title='Счёт на оплату № %d от %s' % (R.randint(10, 999), date()),
                head=[H('№', 'Наименование товара (работ, услуг)', 'Ед. изм.', 'Кол-во', 'Цена, руб.', 'Сумма, руб.')], body=body)

def gen_act(i):
    n = R.randint(3, 9); body = []
    for k in range(n):
        body.append([cell(str(k + 1)), cell(R.choice(GOODS[5:15] + GOODS[25:])), cell(date()), cell(money(R.uniform(1000, 250000)))])
    return dict(id='gen_act_%d' % i, source='generated', attrib=dict(kind='акт'), title='Акт выполненных работ № %d' % R.randint(1, 300),
                head=[H('№ п/п', 'Наименование работ, услуг', 'Дата выполнения', 'Стоимость, руб.')], body=body)

def gen_payroll(i, n=None):
    """Расчётная ведомость с двухуровневой шапкой (colspan/rowspan)."""
    n = n or R.randint(4, 16)
    head = [[cell('№', rs=2, th=True), cell('Ф. И. О.', rs=2, th=True), cell('Должность', rs=2, th=True), cell('Начислено, руб.', cs=3, th=True),
             cell('Удержано, руб.', cs=2, th=True), cell('К выплате, руб.', rs=2, th=True)],
            H('Оклад', 'Премия', 'Всего', 'НДФЛ', 'Прочие')]
    body = []
    for k in range(n):
        o = R.choice([32000, 45500, 58000, 74300, 91000]); p = round(o * R.choice([0, .1, .15, .25]), 2); v = o + p; nd = round(v * .13, 2); pr = R.choice([0, 0, 500, 1200.5])
        body.append([cell(str(k + 1)), cell(fio()), cell(R.choice(POS)), cell(money(o)), cell(money(p)), cell(money(v)), cell(money(nd)), cell(money(pr)), cell(money(v - nd - pr))])
    return dict(id='gen_payroll_%d' % i, source='generated', attrib=dict(kind='ведомость'), title='Расчётная ведомость за %s' % R.choice(['январь', 'март', 'июнь', 'октябрь']) + ' 2026 г.',
                head=head, body=body)

def gen_counterparties(i, n=None):
    n = n or R.randint(5, 15)
    body = [[cell(str(k + 1)), cell(R.choice(ORG)), cell(inn(R.choice([10, 12]))), cell(inn(9)), cell(date()), cell(money(R.uniform(5000, 3e6)))] for k in range(n)]
    return dict(id='gen_cp_%d' % i, source='generated', attrib=dict(kind='реестр'), title='Реестр договоров с контрагентами',
                head=[H('№', 'Контрагент', 'ИНН', 'КПП', 'Дата договора', 'Сумма договора, руб.')], body=body)

def gen_regional(i, years=None, multihead=True, n=None):
    """Статистика по регионам: много колонок-годов, шапка с colspan, группы регионов rowspan."""
    yrs = years or list(range(2026 - R.randint(4, 8), 2026))
    n = n or R.randint(6, 14)
    if multihead:
        head = [[cell('Субъект РФ', rs=2, th=True), cell('Показатель по годам', cs=len(yrs), th=True)], H(*[str(y) for y in yrs])]
    else:
        head = [H('Субъект РФ', *[str(y) for y in yrs])]
    regs = R.sample(REG, min(n, len(REG)))
    body = [[cell(r)] + [cell(('%.1f' % R.uniform(0.5, 120)).replace('.', ',')) for _ in yrs] for r in regs]
    return dict(id='gen_reg_%d' % i, source='generated', attrib=dict(kind='статистика'), title='Показатели социально-экономического развития', head=head, body=body)

def gen_grouped(i):
    """Таблица с rowspan в теле (группы)."""
    body = []; k = 0
    for g in R.sample(['Федеральный бюджет', 'Региональный бюджет', 'Внебюджетные средства', 'Собственные средства'], R.randint(2, 4)):
        m = R.randint(2, 4)
        for j in range(m):
            row = [cell(g, rs=m)] if j == 0 else []
            row += [cell(R.choice(GOODS)), cell(money(R.uniform(1e4, 9e6))), cell(('%d' % R.randint(1, 100)) + ' %')]
            body.append(row)
    head = [H('Источник финансирования', 'Направление расходов', 'Сумма, руб.', 'Доля')]
    return dict(id='gen_grp_%d' % i, source='generated', attrib=dict(kind='смета'), title='Распределение средств по источникам', head=head, body=body)

def gen_timesheet(i, days=None):
    """Табель: широкая таблица (ФИО + дни + итого)."""
    days = days or R.choice([12, 15, 16, 20, 24])
    n = R.randint(4, 10)
    head = [[cell('Ф. И. О.', rs=2, th=True), cell('Числа месяца', cs=days, th=True), cell('Итого, ч', rs=2, th=True)], H(*[str(d + 1) for d in range(days)])]
    body = []
    for k in range(n):
        hs = [R.choice(['8', '8', '8', 'В', '4', 'Б', 'О']) for _ in range(days)]
        body.append([cell(fio())] + [cell(h) for h in hs] + [cell(str(sum(int(h) for h in hs if h.isdigit())))])
    return dict(id='gen_ts_%d' % i, source='generated', attrib=dict(kind='табель'), title='Табель учёта рабочего времени', head=head, body=body)

def gen_long(i, n):
    """Длинная таблица для многостраничности (без rowspan в теле)."""
    body = [[cell(str(k + 1)), cell(date()), cell(R.choice(ORG)), cell(R.choice(GOODS)), cell(money(R.uniform(100, 1e6)))] for k in range(n)]
    return dict(id='gen_long_%d' % i, source='generated', attrib=dict(kind='выписка'), title='Выписка операций по счёту',
                head=[H('№', 'Дата', 'Контрагент', 'Назначение платежа', 'Сумма, руб.')], body=body)

def gen_nested(i):
    """Внешняя таблица, у части ячеек вложенная таблица."""
    body = []
    for k in range(R.randint(2, 4)):
        inner = dict(id='gen_nested_%d_in%d' % (i, k), source='generated', attrib=dict(kind='вложенная'), title='',
                     head=[H('Позиция', 'Кол-во')], body=[[cell(R.choice(GOODS)), cell(str(R.randint(1, 40)))] for _ in range(R.randint(2, 3))])
        feats(inner)
        c = cell(''); c['lines'] = ['Комплект %d' % (k + 1)]; c['inner'] = inner
        body.append([cell(str(k + 1)), cell(R.choice(ORG)), c, cell(money(R.uniform(1e4, 5e5)))])
    return dict(id='gen_nested_%d' % i, source='generated', attrib=dict(kind='спецификация'), title='Спецификация поставки',
                head=[H('№', 'Поставщик', 'Состав комплекта', 'Сумма, руб.')], body=body)

def gen_multiline(i):
    """Ячейки с <br> и длинным текстом (перенос)."""
    body = []
    for k in range(R.randint(3, 7)):
        body.append([cell(str(k + 1)), cell('', lines=[R.choice(ORG), 'ИНН ' + inn(10), R.choice(REG)]),
                     cell('Поставка товара «%s» в соответствии с условиями договора и спецификацией к нему, включая погрузочно-разгрузочные работы' % R.choice(GOODS)),
                     cell('', lines=[date(), 'до ' + date()])])
    return dict(id='gen_ml_%d' % i, source='generated', attrib=dict(kind='договоры'), title='Перечень обязательств',
                head=[H('№', 'Исполнитель', 'Предмет договора', 'Срок')], body=body)

def generated(n_each=12):
    out = []
    for i in range(n_each * 2):
        out.append(gen_invoice(i, sep=' ' if i % 5 == 0 else ' ')); out.append(gen_payroll(i)); out.append(gen_regional(i, multihead=i % 3 != 0))
        out.append(gen_counterparties(i)); out.append(gen_grouped(i)); out.append(gen_act(i)); out.append(gen_multiline(i))
        out.append(gen_nested(i)); out.append(gen_timesheet(i))
        out.append(gen_regional(1000 + i, years=list(range(2026 - R.randint(15, 26), 2026)), multihead=True))  # широкие
    for i in range(40): out.append(gen_long(i, R.randint(55, 140)))
    for i in range(10): out.append(gen_payroll(500 + i, n=R.randint(60, 90)))
    for t in out: feats(t)
    return out

if __name__ == '__main__':
    pools = generated()
    with open(sys.argv[1], 'w') as o:
        for t in pools: o.write(json.dumps(t, ensure_ascii=False) + '\n')
    from collections import Counter
    print(Counter(t['source'] for t in pools))
    print('merged', sum(1 for t in pools if t['f']['merged']), 'multihead', sum(1 for t in pools if t['f']['multihead']))
