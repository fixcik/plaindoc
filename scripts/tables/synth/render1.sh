#!/bin/bash
# render1.sh PDF_DIR file.html — один HTML в PDF
P="$1"; f="$2"; n=$(basename "$f" .html)
[ -s "$P/$n.pdf" ] && exit 0
ud=$(mktemp -d "$P/.ud.XXXX")
"${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}" --headless=new --disable-gpu --no-pdf-header-footer --no-first-run --user-data-dir="$ud" --print-to-pdf="$P/$n.pdf" "file://$f" >/dev/null 2>&1
rm -rf "$ud"
