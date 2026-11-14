cd "$(dirname "$0")/kit_osf"
while read u; do
  free=$(df -k . | tail -1 | awk '{print int($4/1048576)}')
  [ "$free" -lt 15 ] && { echo "stopped: only ${free} GB free" > ../kit_stopped.txt; exit 1; }
  id=$(echo "$u" | cut -d/ -f4)
  ls "$id"* >/dev/null 2>&1 || (mkdir -p tmp_$id && cd tmp_$id && curl -s -L -OJ "$u" && mv * ../ 2>/dev/null; cd .. && rmdir tmp_$id 2>/dev/null)
done < urls.txt
echo done > ../kit_done.txt
