# Zenodo record 5105820: Continental Europe (TransnetBW) and Nordic (Fingrid), 2012-2021, yearly zips.
mkdir -p "$(dirname "$0")/zenodo_2012_2021" && cd "$(dirname "$0")/zenodo_2012_2021"
curl -s "https://zenodo.org/api/records/5105820" | python -c "
import sys,json
for f in json.load(sys.stdin)['files']: print(f['key'])" | tr -d '\r' > files.txt
while read f; do curl -s -L -C - -o "$f" "https://zenodo.org/records/5105820/files/$f?download=1"; done < files.txt
echo done > ../zenodo2_done.txt
