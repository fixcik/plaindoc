#!/usr/bin/env python3
"""Synthetic questionnaire PDFs (bank / government forms) with field labels over their values,
for the heading rules (SP8b S2, review K2): the labels must not become headings. All names and
values are invented. HTML is printed to PDF by headless Chrome, like scripts/resume/gen_resumes.py;
every fixture gets <name>.json with `headings` (expected) and `labels` (never headings).

    python3 -I scripts/forms/gen_forms.py [OUT_DIR]     (default: crates/formats/tests/fixtures/pdf/forms)
"""
import html, json, os, shutil, subprocess, sys, tempfile

CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "crates/formats/tests/fixtures/pdf/forms")

FIELDS = [("ФИО", "Петров Пётр Петрович"), ("Дата рождения", "01.02.1980"), ("Паспорт", "4500 123456"),
          ("Адрес регистрации", "г. Примерск, ул. Тестовая, д. 1"), ("Телефон", "+7 000 000-00-00"),
          ("Электронная почта", "petrov@example.test")]
CSS = """body{font-family:Helvetica,Arial,sans-serif;margin:48pt;color:#222}
h1{font-size:16pt;margin:0 0 18pt}h2{font-size:12pt;margin:16pt 0 8pt}
.lab{margin:10pt 0 2pt}.val{font-size:11pt;margin:0}.line{border-bottom:0.8pt solid #444;height:14pt;width:300pt}
.caps{text-transform:uppercase;font-size:8.5pt;color:#1f56a8}.bold{font-weight:bold;font-size:10pt}
.bcaps{text-transform:uppercase;font-weight:bold;font-size:10pt}"""


def variant(label_cls, numbered=False, values=True, sections=False):
    out, labels = [], []
    heads = [{"text": "Анкета клиента — физического лица", "level": 1}]
    out.append("<h1>Анкета клиента — физического лица</h1>")
    groups = [("Раздел 1. Личные данные", FIELDS[:3]), ("Раздел 2. Контакты", FIELDS[3:])] if sections else [(None, FIELDS)]
    n = 0
    for sec, fields in groups:
        if sec:
            out.append(f"<h2>{html.escape(sec)}</h2>")
            heads.append({"text": sec, "level": 2})
        for lab, val in fields:
            n += 1
            shown = f"{n}. {lab}" if numbered else lab
            labels.append(shown.upper() if "caps" in label_cls else shown)
            out.append(f'<p class="lab {label_cls}">{html.escape(shown)}</p>')
            out.append(f'<p class="val">{html.escape(val)}</p>' if values else '<div class="line"></div>')
    return out, heads, labels


V = {
    "f01_caps_accent_values": dict(label_cls="caps"),
    "f02_numbered_bold_values": dict(label_cls="bold", numbered=True),
    "f03_bold_caps_blank_lines": dict(label_cls="bcaps", values=False),
    "f04_sections_caps_values": dict(label_cls="caps", sections=True),
    "f05_numbered_bold_blank_lines": dict(label_cls="bold", numbered=True, values=False),
}


def main():
    os.makedirs(OUT, exist_ok=True)
    work = tempfile.mkdtemp(prefix="forms-gen-")
    try:
        for name, o in V.items():
            body, heads, labels = variant(**o)
            src = os.path.join(work, name + ".html")
            open(src, "w", encoding="utf-8").write(
                f'<!doctype html><html lang="ru"><meta charset="utf-8"><style>{CSS}</style><body>{"".join(body)}</body></html>')
            pdf = os.path.join(OUT, name + ".pdf")
            cmd = [CHROME, "--headless=new", "--disable-gpu", "--no-pdf-header-footer", f"--print-to-pdf={pdf}"]
            subprocess.run(cmd + ["file://" + src], check=True, stderr=subprocess.DEVNULL)
            json.dump(dict(description=o, headings=heads, labels=labels),
                      open(os.path.join(OUT, name + ".json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
            print(f"{name}: {os.path.getsize(pdf)} B, {len(heads)} headings, {len(labels)} labels")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
