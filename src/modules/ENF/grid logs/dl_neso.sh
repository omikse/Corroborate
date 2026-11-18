# NESO (Great Britain) system frequency, 1 s, monthly CSV; the list of files is neso-urls.txt.
mkdir -p "$(dirname "$0")/neso" && cd "$(dirname "$0")/neso"
while read u; do f=$(basename "$u"); [ -s "$f" ] || curl -s -L -o "$f" "$u"; done < ../neso-urls.txt
echo done > ../neso_done.txt
