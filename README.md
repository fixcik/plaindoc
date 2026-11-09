# plaindoc

A fast document parser for LLM agents: turns office files, PDFs and scans into Markdown or JSON.

## What it does

- **Formats:** PDF with a text layer and scanned PDF (OCR), doc/docx, xls/xlsx, ppt/pptx, odt/ods, rtf, csv/tsv, txt, md, html/xml, images.
- **Output:** Markdown or JSON (plaindoc's own schema, not DoclingDocument).
- **Structure:** links, headings, lists, tables, pictures (files next to the output), running headers and footers, hidden-text removal.
- **OCR:** PP-OCRv5 models (about 13 MB) embedded in the binary. Images on PDF pages with a text layer are not recognised by default: they are written out as picture files and linked (`--ocr images` recognises them too).
- One Rust binary, no Python and no heavy machine-learning frameworks.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/fixcik/plaindoc/main/install.sh | bash
```

The script picks the binary for your system (macOS on Apple silicon, Linux x86_64 or aarch64),
checks its sha256 and puts `plaindoc` into `~/.local/bin`. Set `PLAINDOC_VERSION` (for example
`0.1.0`) to install a specific release and `PLAINDOC_INSTALL_DIR` to change the directory. To
remove it, delete the binary. On Windows download the `.zip` from
[GitHub Releases](https://github.com/fixcik/plaindoc/releases) and put `plaindoc.exe` on `PATH`.

## Build and run

```sh
cargo build --release -p plaindoc-cli        # target/release/plaindoc; models: docs/building.md
plaindoc parse report.pdf -f markdown        # Markdown to stdout
plaindoc parse scans/ -f json -o out/ -j 8   # a directory, 8 files in parallel
plaindoc parse --help                        # OCR mode, hidden text, page separators, stats
```

Prebuilt binaries for macOS (Apple silicon), Linux (x86_64, aarch64) and Windows (x86_64), with
the OCR models embedded, are on [GitHub Releases](https://github.com/fixcik/plaindoc/releases). The
Linux binaries need glibc 2.38+ and libstdc++ from GCC 13+ (Ubuntu 24.04, Debian 13, Fedora 39,
RHEL 10 or newer); for older systems see [docs/building.md](docs/building.md#linux-requirements).

## Compared with Docling

- On small text PDFs roughly 60–200× faster (informal spot check on 23 one- and two-page PDFs, Apple silicon; not a benchmark): 0.112 s for all 23 files on one thread, 0.036 s on all cores, against 7.2 s of conversion time for Docling 2.148 with default settings (plus about 5 s of start-up and model loading).
- On scans with OCR the gap is much smaller (about 3×) and depends on the OCR engine: in a similar informal check, a one-page scanned invoice took about 4 s with plaindoc and about 12.6 s with Docling (17 s with start-up). Neither recovered the table structure.
- Docling has model-based page layout and table structure (layout model, TableFormer). plaindoc does not: tables in text PDFs come from ruling lines and text alignment, and tables on scans are not built.
- Docling is a mature project with a large community. plaindoc is young; forms and scans with complex structure are its weak spots.

## Status

Early version. Known gaps: table structure on scans, underlined form fields, borderless tables in DOCX. Plans: [docs/ROADMAP.md](docs/ROADMAP.md).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option. Third-party models, data and fixtures: [NOTICE](NOTICE).

Repository: https://github.com/fixcik/plaindoc
