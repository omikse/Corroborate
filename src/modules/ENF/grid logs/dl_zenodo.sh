cd "$(dirname "$0")/zenodo"
for f in Scripts.zip DESCRIPTION.md Data_cleansed.zip Data_converted.zip; do curl -s -L -C - -o "$f" "https://zenodo.org/records/3744121/files/$f?download=1"; done
echo done > ../zenodo_done.txt
