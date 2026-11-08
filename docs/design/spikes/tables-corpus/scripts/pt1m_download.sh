#!/bin/sh
# Скачивание PDF из бакета PMC Open Access (AWS Open Data, без логина) по списку pt1m_selected.tsv.
# usage: sh pt1m_download.sh LIST OUTDIR MAXBYTES
LIST=$1; OUT=$2; MAX=${3:-1500000000}
mkdir -p "$OUT"; total=0; n=0
while IFS="$(printf '\t')" read -r pmc rest; do
  f="$OUT/$pmc.pdf"
  [ -s "$f" ] && { total=$((total + $(stat -f %z "$f"))); continue; }
  curl -s -f --retry 3 --max-time 120 -o "$f" "https://pmc-oa-opendata.s3.amazonaws.com/$pmc.1/$pmc.1.pdf" || { rm -f "$f"; echo "FAIL $pmc"; continue; }
  sz=$(stat -f %z "$f"); total=$((total + sz)); n=$((n + 1))
  [ "$total" -gt "$MAX" ] && { echo "budget reached $total"; break; }
done < "$LIST"
echo "done n=$n total=$total"
