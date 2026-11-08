#!/usr/bin/env python3
"""Synthetic resume-class PDFs with ground truth (no personal data: all names, companies and
texts are invented). HTML is printed to PDF by headless Chrome (like crates/formats/tests/
fixtures/pdf/gen.sh); every fixture gets <name>.json with the expected structure.

    python3 -I scripts/resume/gen_resumes.py [OUT_DIR]     (default: crates/formats/tests/fixtures/pdf/resume)

Output is deterministic up to the timestamps Chrome writes into the PDF info dictionary.
"""
import html, json, os, re, shutil, subprocess, sys, tempfile

CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "crates/formats/tests/fixtures/pdf/resume")

L = {
    "ru": dict(
        name="Алексей Примеров", role="Инженер-программист",
        sections=dict(about="О себе", exp="Опыт работы", edu="Образование", skills="Навыки",
                      contacts="Контакты", oss="Open Source", extra="Дополнительно"),
        jobs=[("ООО «Ромашка-Тест»", "Ведущий разработчик", "март 2021 — настоящее время"),
              ("АО «Северный контур»", "Разработчик", "июнь 2018 — февраль 2021"),
              ("ИП Образцов", "Стажёр", "сентябрь 2016 — май 2018"),
              ("ООО «Вымышленные системы»", "Тестировщик", "2014 — 2016"),
              ("АО «Пример-Софт»", "Аналитик", "2012 — 2014")],
        bullets=["Разработал модуль учёта заказов, сократил время обработки заявки на треть",
                 "Настроил сборку и выкладку, перевёл команду на еженедельные релизы",
                 "Провёл обучение для новых сотрудников и подготовил регламент проверки кода",
                 "Оптимизировал медленные запросы к базе данных, нагрузка снизилась вдвое",
                 "Написал автоматические тесты для основных сценариев и подключил их к конвейеру",
                 "Согласовывал требования с заказчиком и вёл планирование на квартал",
                 "Участвовал в разборе инцидентов и подготовил план улучшений по итогам"],
        about_text="Опытный разработчик, интересуюсь качеством кода и понятными процессами. "
                   "Люблю небольшие команды и ясные задачи. Все сведения в этом документе вымышлены.",
        edu=[("Тестовый государственный университет", "Прикладная информатика, 2012")],
        skills=["Rust", "Python", "SQL", "Linux", "Docker", "Git", "REST", "Kafka", "CI/CD"],
        contacts=[("Телефон", "+7 000 000-00-00", None), ("Почта", "{{alex@example.test|mailto:alex@example.test}}", None),
                  ("Сайт", "{{example.test/alex|https://example.test/alex}}", None), ("Город", "Вымышленск", None)],
        oss=[("tiny-parser", "Небольшая библиотека для разбора конфигурации"),
             ("sample-cli", "Утилита командной строки для проверки примеров")],
        link_bullet="Подробности проекта: {{страница проекта|https://example.test/project}}",
        extra=["Английский: чтение технической литературы", "Права категории B"],
    ),
    "en": dict(
        name="Jane Sample", role="Software Engineer",
        sections=dict(about="Summary", exp="Experience", edu="Education", skills="Skills",
                      contacts="Contacts", oss="Open Source", extra="Additional"),
        jobs=[("Acme Widgets Ltd", "Senior Developer", "Mar 2021 – Present"),
              ("Northwind Samples", "Developer", "Jun 2018 – Feb 2021"),
              ("Example Studio", "Intern", "Sep 2016 – May 2018"),
              ("Fictional Systems Inc", "QA Engineer", "2014 – 2016"),
              ("Lorem Software", "Analyst", "2012 – 2014")],
        bullets=["Built an order tracking module and cut request handling time by a third",
                 "Set up build and deployment pipelines and moved the team to weekly releases",
                 "Trained new colleagues and wrote the code review guidelines for the team",
                 "Optimised slow database queries so that the load dropped by half",
                 "Wrote automated tests for the main scenarios and plugged them into CI",
                 "Agreed requirements with the customer and planned the work per quarter",
                 "Took part in incident reviews and prepared the improvement plan afterwards"],
        about_text="Experienced developer who cares about code quality and clear processes. "
                   "Prefers small teams and well defined tasks. Everything in this document is made up.",
        edu=[("Test State University", "Applied Computer Science, 2012")],
        skills=["Rust", "Python", "SQL", "Linux", "Docker", "Git", "REST", "Kafka", "CI/CD"],
        contacts=[("Phone", "+0 000 000 0000", None), ("Email", "{{jane@example.test|mailto:jane@example.test}}", None),
                  ("Web", "{{example.test/jane|https://example.test/jane}}", None), ("City", "Nowhereville", None)],
        oss=[("tiny-parser", "A small library for configuration parsing"),
             ("sample-cli", "A command line tool to check examples")],
        link_bullet="Project details: {{project page|https://example.test/project}}",
        extra=["English: reading technical literature", "Driving licence B"],
    ),
}

MARKERS = {"dot": "•", "en": "–", "em": "—", "sq": "▪", "hy": "-"}
ORD = {"n.": lambda i: f"{i}.", "n)": lambda i: f"{i})", "a)": lambda i: "абвгде"[i - 1] + ")"}


def spaced(s):
    return "   ".join(" ".join(w) for w in s.split(" "))


class Doc:
    def __init__(self, o):
        self.o = o
        self.t = L[o["lang"]]
        self.h, self.heads, self.items, self.links = [], [], [], []
        self.sec_no = 0
        self.sub_no = 0

    # ---- text with links: {{text|url}} ----
    def rich(self, s):
        def sub(m):
            self.links.append({"text": m.group(1), "url": m.group(2)})
            return f'<a href="{html.escape(m.group(2))}">{html.escape(m.group(1))}</a>'
        parts, pos, out = list(re.finditer(r"\{\{(.+?)\|(.+?)\}\}", s)), 0, ""
        for m in parts:
            out += html.escape(s[pos:m.start()]) + sub(m)
            pos = m.end()
        return out + html.escape(s[pos:])

    def plain(self, s):
        return re.sub(r"\{\{(.+?)\|.+?\}\}", r"\1", s)

    def heading(self, level, text, out):
        """A section title (level 2) or sub-title (3) in the variant's style."""
        st = self.o["title"]
        printed = text
        if level == 2:
            if self.o.get("numbered"):
                self.sec_no += 1
                self.sub_no = 0
                printed = f"{self.sec_no}. {text}"
            cls = {"caps": "t-caps", "spaced": "t-caps", "tracked": "t-track", "bold": "t-bold", "rule": "t-rule"}[st]
            if st in ("caps", "tracked", "rule"):
                shown = printed.upper()
            elif st == "spaced":
                shown = spaced(printed.upper())
            else:
                shown = printed
            out.append(f'<h2 class="{cls}">{html.escape(shown)}</h2>')
        else:
            self.sub_no += 1
            printed = f"{self.sec_no}.{self.sub_no} {text}"
            out.append(f'<h3 class="t-sub">{html.escape(printed)}</h3>')
        self.heads.append({"text": printed.upper() if st in ("caps", "spaced", "tracked", "rule") and level == 2 else printed,
                           "level": level, "printed": shown if level == 2 else printed})

    def li(self, out, text, depth=0, idx=None):
        o = self.o
        if o["marker"] in ORD:
            m, ordered = ORD[o["marker"]](idx), True
        else:
            m, ordered = MARKERS[o["marker"]], False
            if depth:
                m = "–"
        self.items.append({"text": self.plain(text), "ordered": ordered, "depth": depth, "marker": m})
        out.append(f'<div class="li d{depth}"><span class="m">{m}</span><span class="b">{self.rich(text)}</span></div>')

    def bullets(self, out, texts, start=0):
        n = 0
        for i, b in enumerate(texts):
            n += 1
            self.li(out, b, 0, n)
            if self.o.get("nested") and i % 2 == 0:
                self.li(out, b.split(",")[0].split(" и ")[0] + " (детали)" if self.o["lang"] == "ru"
                        else b.split(" and ")[0] + " (details)", 1, 1)

    # ---- blocks ----
    def jobs(self, out, k0, k1, with_link=False):
        t = self.t
        for k in range(k0, k1):
            c, r, d = t["jobs"][k % len(t["jobs"])]
            if self.o.get("numbered"):
                self.heading(3, c, out)
                out.append(f'<div class="role">{html.escape(r)}</div>')
            else:
                out.append(f'<div class="job"><b>{html.escape(c)}</b> — {html.escape(r)}</div>')
            out.append(f'<div class="dates">{html.escape(d)}</div>')
            nb = 3
            texts = [t["bullets"][(k * 2 + j) % len(t["bullets"])] for j in range(nb)]
            if with_link and k == k0:
                texts.append(t["link_bullet"])
            self.bullets(out, texts)

    def chips(self, out):
        out.append('<div class="chips">' + "".join(f'<span class="chip">{html.escape(s)}</span>' for s in self.t["skills"]) + "</div>")

    def contacts(self, out):
        for lab, val, _ in self.t["contacts"]:
            out.append(f'<div class="ct"><div class="cl">{html.escape(lab)}</div><div class="cv">{self.rich(val)}</div></div>')

    def oss(self, out):
        for n, d in self.t["oss"]:
            out.append(f'<div class="os"><b>{html.escape(n)}</b><div>{html.escape(d)}</div></div>')

    def build(self):
        o, t, S = self.o, self.t, self.t["sections"]
        pages = o["pages"]
        side = o["cols"] == 2
        main = [[] for _ in range(pages)]
        sidebar = []
        # header on page 1
        main[0].append(f'<h1>{html.escape(t["name"])}</h1><div class="sub">{html.escape(t["role"])}</div>')
        self.heads.append({"text": t["name"], "level": 1, "printed": t["name"]})
        if not side:
            main[0].append(f'<div class="ct1">{" · ".join(self.rich(v) for _, v, _ in t["contacts"][1:3])}</div>')
        self.heading(2, S["about"], main[0])
        main[0].append(f'<p>{html.escape(t["about_text"])}</p>')
        # experience spread over the pages
        njobs = 2 * pages + 1 if pages > 1 else 2
        per = [njobs // pages + (1 if i < njobs % pages else 0) for i in range(pages)]
        k = 0
        for p in range(pages):
            if p == 0:
                self.heading(2, S["exp"], main[p])
            self.jobs(main[p], k, k + per[p], with_link=(p == 0))
            k += per[p]
        last = main[pages - 1]
        self.heading(2, S["edu"], last)
        for u, d in t["edu"]:
            last.append(f'<div class="job"><b>{html.escape(u)}</b></div><div class="dates">{html.escape(d)}</div>')
        if side:
            self.heading(2, S["contacts"], sidebar)
            self.contacts(sidebar)
            self.heading(2, S["skills"], sidebar)
            self.chips(sidebar)
            if o.get("oss"):
                self.heading(2, S["oss"], sidebar)
                self.oss(sidebar)
        else:
            if o.get("chips"):
                self.heading(2, S["skills"], last)
                self.chips(last)
            self.heading(2, S["extra"], last)
            self.bullets(last, t["extra"])
        return main, sidebar

    def render(self):
        main, sidebar = self.build()
        o = self.o
        parts = []
        for p, blocks in enumerate(main):
            side = ""
            if o["cols"] == 2:
                side = f'<div class="side">{"".join(sidebar) if p == 0 else ""}</div>'
            parts.append(f'<div class="page {"left" if o.get("left") else ""}"><div class="main">{"".join(blocks)}</div>{side}</div>')
        css = CSS_BASE + (CSS_2COL if o["cols"] == 2 else "")
        return f'<!doctype html><meta charset=utf-8><title>{html.escape(self.t["name"])}</title><style>{css}</style><body>{"".join(parts)}'


CSS_BASE = """
@page{size:A4;margin:0}
body{margin:0;font-family:Helvetica;font-size:10.5pt;color:#222}
.page{position:relative;width:210mm;height:296mm;overflow:hidden;break-after:page;box-sizing:border-box;padding:16mm 14mm}
.page:last-child{break-after:auto}
h1{font-size:22pt;margin:0 0 2pt}.sub{font-size:12pt;color:#555;margin-bottom:10pt}
h2{margin:14pt 0 6pt}.t-caps{font-size:11pt;letter-spacing:0;font-weight:bold}
.t-track{font-size:11pt;letter-spacing:.3em;font-weight:bold}
.t-bold{font-size:15pt;font-weight:bold}
.t-rule{font-size:11pt;font-weight:bold;border-bottom:1px solid #444;padding-bottom:3pt}
h3.t-sub{font-size:11.5pt;margin:8pt 0 1pt}.role{color:#555;margin-bottom:1pt}
.job{margin-top:7pt}.dates{color:#666;font-size:9.5pt;margin:1pt 0 3pt}
p{margin:0 0 4pt;line-height:1.35}
.li{display:flex;margin:2pt 0;line-height:1.3}.li .m{flex:0 0 14pt}.li.d1{margin-left:16pt}.li .b{flex:1}
.chips{display:flex;flex-wrap:wrap;gap:4pt}
.chip{border:1px solid #6a8fb8;border-radius:9pt;padding:2pt 7pt;font-size:9pt;background:#fff}
.ct{margin:4pt 0}.cl{font-size:7.5pt;text-transform:uppercase;color:#667;letter-spacing:.05em}.cv{font-size:10pt}
.os{margin:5pt 0}.os div{font-size:9.5pt}.ct1{color:#444;margin-bottom:4pt}
a{color:#1a56b0}
"""
CSS_2COL = """
.page::before{content:"";position:absolute;top:0;bottom:0;right:0;width:68mm;background:#e3eefb}
.page.left::before{right:auto;left:0}
.main{width:118mm;position:relative}.page.left .main{margin-left:60mm;width:118mm}
.side{position:absolute;top:16mm;right:6mm;width:56mm}.page.left .side{right:auto;left:6mm}
"""

# name -> options
V = {}
def v(name, lang, title, marker, cols=1, pages=1, **kw):
    V[name] = dict(lang=lang, title=title, marker=marker, cols=cols, pages=pages, **kw)

v("r01_ru_caps_dot", "ru", "caps", "dot")
v("r02_en_caps_endash", "en", "caps", "en", chips=True)
v("r03_ru_spaced_emdash", "ru", "spaced", "em")
v("r04_en_tracked_square", "en", "tracked", "sq")
v("r05_ru_rule_hyphen", "ru", "rule", "hy")
v("r06_ru_bold_num_dot", "ru", "bold", "n.")
v("r07_en_bold_num_paren", "en", "bold", "n)")
v("r08_ru_caps_alpha", "ru", "caps", "a)")
v("r09_en_rule_nested", "en", "rule", "dot", nested=True)
v("r10_ru_numbered_headings", "ru", "bold", "dot", numbered=True)
v("r11_en_numbered_bookmarks", "en", "bold", "en", numbered=True, bookmarks=True)
v("r12_ru_3pages_bookmarks", "ru", "caps", "dot", pages=3, bookmarks=True)
v("r13_en_2pages_spaced", "en", "spaced", "em", pages=2)
v("r14_ru_rule_chips", "ru", "rule", "dot", chips=True, bookmarks=True)
v("r20_ru_sidebar_caps_endash", "ru", "caps", "en", cols=2)
v("r21_en_sidebar_spaced_dot", "en", "spaced", "dot", cols=2)
v("r22_ru_sidebar_tracked_oss", "ru", "tracked", "dot", cols=2, oss=True)
v("r23_en_sidebar_rule_2pages", "en", "rule", "hy", cols=2, pages=2, oss=True)
v("r24_ru_sidebar_numbered", "ru", "bold", "dot", cols=2, numbered=True, bookmarks=True)
v("r25_ru_sidebar_3pages_nested", "ru", "caps", "dot", cols=2, pages=3, nested=True, bookmarks=True)
v("r26_en_sidebar_square_left", "en", "caps", "sq", cols=2, left=True, oss=True)
v("r27_ru_sidebar_alpha", "ru", "tracked", "a)", cols=2)


def count_pages(path):
    data = open(path, "rb").read()
    m = re.findall(rb"/Type\s*/Pages[^>]*?/Count\s+(\d+)", data) or re.findall(rb"/Count\s+(\d+)[^>]*?/Type\s*/Pages", data)
    return int(m[-1]) if m else None


def main():
    os.makedirs(OUT, exist_ok=True)
    work = tempfile.mkdtemp(prefix="resume-gen-")
    try:
        for name, o in V.items():
            d = Doc(o)
            h = d.render()
            src = os.path.join(work, name + ".html")
            open(src, "w", encoding="utf-8").write(h)
            pdf = os.path.join(OUT, name + ".pdf")
            cmd = [CHROME, "--headless=new", "--disable-gpu", "--no-pdf-header-footer", f"--print-to-pdf={pdf}"]
            if o.get("bookmarks"):
                cmd.append("--generate-pdf-document-outline")
            subprocess.run(cmd + ["file://" + src], check=True, stderr=subprocess.DEVNULL)
            pages = count_pages(pdf)
            if pages != o["pages"]:
                sys.exit(f"{name}: expected {o['pages']} pages, got {pages}")
            gt = dict(
                description=o,
                pages=o["pages"], tables=0,
                headings=[{"text": x["text"], "level": x["level"], "printed": x["printed"]} for x in d.heads],
                list_items=[{"text": x["text"], "ordered": x["ordered"], "depth": x["depth"], "marker": x["marker"]} for x in d.items],
                links=d.links,
            )
            json.dump(gt, open(os.path.join(OUT, name + ".json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
            print(f"{name}: {os.path.getsize(pdf)} B, {len(d.heads)} headings, {len(d.items)} items, {len(d.links)} links")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
