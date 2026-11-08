# Synthetic resume set (SP8b S5)

22 PDFs in `crates/formats/tests/fixtures/pdf/resume/` with a ground-truth manifest `<name>.json` each.
All names, companies and texts are invented; real resumes are used only locally by hand and never copied here.

## Regenerating

    python3 -I scripts/resume/gen_resumes.py [OUT_DIR]

Needs macOS Chrome (`CHROME=...` to override) and Python 3 stdlib only. HTML is printed with headless Chrome
(font Helvetica, A4, `--generate-pdf-document-outline` for bookmark variants). The generator checks the page
count. Output differs between runs only in the timestamps of the PDF info dictionary. Each PDF is 55-110 KB.

## Manifest

`headings` (`text` canonical, `level` 1 name / 2 section / 3 numbered subsection, `printed` as drawn),
`list_items` (`text`, `ordered`, `depth`, `marker`), `links` (`text`, `url`, Link annotations https and mailto),
`tables` (always 0), `pages`, `description` (the variant options). Not headings by design: the job line
(bold company + role), dates, contact labels, chips, open-source entry names.

## Variants (r01..r14 one column, r20..r27 sidebar)

- Title style: `caps`, `spaced` (`К О Н Т А К Т Ы`, literal spaces), `tracked` (CSS letter-spacing), `bold` (larger bold), `rule` (bold caps with an underline rule), numbered (`1.`, `1.1`: r10, r11, r24).
- Markers: `•` `–` `—` `▪` `-`, ordered `1.` `1)` `а)`, nested bullets (r09, r25).
- Sidebar (r20..r27): page-wide coloured rectangle on the right (left in r26), contacts as small caps label over value, skills as rounded chips, open-source entries (r22, r23, r26); chips in one column in r02, r14.
- Timeline of jobs with dates, wrapped multi-line bullets, links inside bullets and contacts, 1-3 pages (r12, r25: 3; r13, r23: 2), bookmarks in r11, r12, r14, r24, r25.

## Test

`crates/formats/tests/pdf_resume_set.rs` (feature `pdf`). Green: link recall (all links found, text and URL), no tables on one-column variants and on sidebar variants (`resume_sidebar_has_no_false_tables`, SP8b S3), and all list items of the one-column variants (`resume_list_items_match_manifest`, SP8b S1).
Headings (SP8b S2): `resume_one_column_headings_match_manifest_with_levels` is green (every heading of the one-column variants, text and level).
Ignored with a reason: `resume_headings_match_manifest` (sidebar variants: a main-column title shares its line with a sidebar label, 6 of 129), `resume_sidebar_list_items_match_manifest`.
Scoreboard:

    cargo test -p plaindoc-formats --features pdf --test pdf_resume_set -- --ignored --nocapture resume_scoreboard

Matching is by text (lower case, whitespace removed; list items at the same depth). When a lane lands, drop the matching `#[ignore]`.
