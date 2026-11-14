cd "$(dirname "$0")/neso"
while read u; do f=$(basename "$u"); [ -s "$f" ] || curl -s -L -o "$f" "$u"; done < urls.txt
echo done > ../neso_done.txt
