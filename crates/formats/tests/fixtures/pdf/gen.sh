#!/usr/bin/env bash
# Regenerates the committed PDF fixtures. Derived cases (truncated, mojibake, zero, garbage,
# encrypted) are built inside the tests from these files and need no regeneration.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
WORK=/tmp/plaindoc-fixtures   # neutral path: Chrome prints the URL into footers
rm -rf "$WORK" && mkdir -p "$WORK" && cp "$DIR"/src/*.html "$WORK"/
chrome() { "$CHROME" --headless=new --disable-gpu --allow-file-access-from-files "$@" 2>/dev/null; }
chrome --hide-scrollbars --window-size=800,260 --screenshot="$WORK/scan.png" "file://$WORK/scanpage.html"
for f in ru scanimg imgtext empty; do
  chrome --no-pdf-header-footer --print-to-pdf="$WORK/$f.pdf" "file://$WORK/$f.html"
done
chrome --print-to-pdf="$WORK/notes.pdf" "file://$WORK/notes.html"   # with header/footer on purpose
cp "$WORK/ru.pdf" "$WORK/imgtext.pdf" "$WORK/empty.pdf" "$WORK/notes.pdf" "$DIR"/
cp "$WORK/scanimg.pdf" "$DIR/scan.pdf"
ls -l "$DIR"/*.pdf
