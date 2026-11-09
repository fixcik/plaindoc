# Output format

`Document::to_json()` (crate `plaindoc-core`, file `crates/core/src/model.rs`) gives pretty-printed JSON
without a trailing newline. The CLI prints the same JSON with `--format json`. Field names are
`snake_case`; the structs are `#[non_exhaustive]`, so new fields may appear.

## Document

| field | type | meaning |
|---|---|---|
| `metadata` | object | `format` (lower-case name: `pdf`, `docx`, ...), `title`, `author`, `page_count`; each may be `null` |
| `sections` | array | pages of a PDF, sheets, slides, or one `body` |
| `warnings` | array | document-level warnings |
| `quality` | number | 0.0 to 1.0, 1.0 when nothing is suspicious |

Pictures are not part of the JSON; their bytes live in `Document::images` and are written next to the
Markdown output (see `ImageOptions`).

## Section

`kind` (`page`, `sheet`, `slide`, `body`), `index` (counted from 0; Markdown shows `index + 1`), `name`
(sheet or slide title, may be `null`), `blocks`, `warnings`, `quality`, `source` (`text_layer`, `ocr`,
`mixed`).

## Block

A block is an object with a `type` tag; every variant has `bbox` (`{x, y, w, h}` in points, or `null`).

| `type` | other fields |
|---|---|
| `heading` | `level`, `text`, `links` |
| `paragraph` | `text`, `links` |
| `list` | `ordered`, `items` (each `{blocks}`; the first paragraph is the item text, a nested list is a `list` block) |
| `table` | `rows` (array of rows of cells), `header_rows` |
| `image` | `alt`, `ocr_text`, `image` (`file`, `mime`, `size`, `sha256`, `source`, `url`, `skipped`; absent fields are omitted) |
| `code` | `lang`, `text` |
| `note` | `kind` (`header`, `footer`, `footnote`, `endnote`, `speaker_notes`, `comment`, `page_number`, `marginal`), `text` |

`links` is omitted when empty. A link is `{start, end, url}`: char offsets (Unicode scalar values, not
bytes) into `text`, sorted and non-overlapping; `url` is absolute `http`, `https` or `mailto`.

## Pictures of PDF pages

A PDF page that keeps its text layer gives its raster images as `image` blocks (no `alt`, no
`ocr_text`; `bbox` is the displayed box clipped to the page), placed among the page's blocks by
their top edge. They are stored like office pictures: one `imageN.png` per distinct picture
(a plain JPEG goes out as stored, `imageN.jpg`; an image with a soft mask is a PNG with alpha), in
`img-<id>/` next to the output with `-o`, linked as `![изображение N](img-<id>/imageN.png)`.
`--no-images` leaves bare `<!-- image -->` marks. Without `-o` (output on stdout) pictures of any
format are only named, and stderr gets one line per document with pictures: `note: FILE: N
picture(s) not saved: pass -o DIR to write them as files` (not with `--no-warnings`).

Left out as page furniture: the same raster in the top or bottom 15 % of the page on at least 3
pages (a running header or footer logo; documents of fewer pages keep it), images under 32 pt or
16 px on a side (rules, bullets, icons) and plates (every channel within 32 levels of 255: a fill
or a faint gradient; not checked on JPEGs, which are not decoded). Scans give no pictures: pages
that go to full-page OCR, pages whose images cover 80 % or more, and "scans with service text"
(images over 60 % of the page with a layer under 200 non-space chars), which keep their OCR;
so do pages the engine classifies as scanned. Boxes are clipped to the visible page (CropBox).

OCR of images on such pages (`--ocr`, `ParseOptions::ocr`):

- `auto` (default): images on a page with a text layer are not recognised. Where the need-OCR
  classifier would have OCRed them (text-like ink outside the layer), the page gets an
  `ImageTextPage`/`info` warning `N image(s) not recognised: OCR is skipped on a page with a text
  layer (K kept as pictures)` and the classifier's own `ImageTextPage` hint is `info`, not `warn`.
  A page whose images all turn out to be furniture (running repeats, small, plates, undecodable)
  keeps its OCR. A text page whose images cover at most 1 % of it is not rendered for the
  classifier (`NeedOcrConfig::render_min_image_cover`), so a small logo never triggers OCR.
- `images`: as `auto`, plus the OCR of those images merged into the page (the `auto` of earlier
  versions), with the pictures linked too.
- `always`: OCR of every page, merged with the layer; pictures linked. `never`: no OCR.

`--stats` counts them in `images`: `linked` (picture blocks with a stored file, every occurrence;
all formats), `ocr_skipped` (PDF images not sent to OCR under `auto`) and `dropped` (PDF page
furniture left out); decoding time is the `images` stage.

## Cell

`blocks` (content in order: text, nested table, text), `colspan`, `rowspan`, `value`. Rows hold only
source cells: positions covered by a span are not repeated (HTML model). `value` is `null` for text
extracted from PDF and legacy `.xls`; for spreadsheets it is `{value, formula, number_format}`, where
`value` is `{type, value}` with `type` one of `number`, `text`, `bool`, `date` (serial day number),
`error`, `formula` has no leading `=`, and `number_format` is e.g. `0.00%`.

## Warnings

`{kind, level, message, code}`; `code` (rule name within `kind`, e.g. `tiny` for `HiddenContent`) is
omitted when absent. `level` is `info`, `warn` or `loss`. `kind` is serialized as is: `NoTextLayer`,
`ImageTextPage`, `SuspectedGarbage`, `PartialContent`, `Truncated`, `Encrypted`, `OcrUsed`,
`LowOcrConfidence`, `OcrEmpty`, `OcrUnavailable`, `TableUncertain`, `FormulaSkipped`, `EmptyOutput`,
`HiddenContent`, `ContentDropped`.

## Markdown defaults

- Notes of kinds `header`, `footer`, `page_number` are not printed; `MarkdownOptions::headers_footers`
  (CLI `--headers-footers`) brings them back. The JSON is unchanged.
- PDF pages are separated by a `---` line (not before the first or after the last page);
  `MarkdownOptions::page_separator` (CLI `--page-separator rule|comment|none`) changes it. Sheets and
  slides keep the comments `<!-- sheet N: name -->` and `<!-- slide N -->`.

## CLI exit codes

`0` success; `1` a file failed; `2` only with `--strict --min-quality X` when a document is below the
threshold and nothing failed; `64` usage error. `1` wins over `2`.
