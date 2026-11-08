#!/bin/bash
# render.sh HTML_DIR PDF_DIR [JOBS] — печать всех HTML в PDF через Chrome headless (thead повторяется на страницах)
H="$1"; P="$2"; J="${3:-6}"; mkdir -p "$P"
ls "$H"/*.html | xargs -P "$J" -n 1 "$(dirname "$0")/render1.sh" "$P"
ls "$P"/*.pdf | wc -l
