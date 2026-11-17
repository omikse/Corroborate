# Energy-Charts (Fraunhofer ISE, Freiburg): Continental Europe grid frequency, 1 s, UTC (unix seconds), one file per UTC day. Resumable.
# The API rate-limits (HTTP 429): wait and retry, backing off up to 5 min.
cd "$(dirname "$0")/energy_charts"
rm -f failed.txt
d=2022-05-01
end=$(date -u +%Y-%m-%d)
while [ "$d" \< "$end" ]; do
  n=$(date -u -d "$d + 1 day" +%Y-%m-%d)
  wait=20
  while [ ! -s "$d.json" ]; do
    code=$(curl -s -o "$d.tmp" -w "%{http_code}" "https://api.energy-charts.info/frequency?start=${d}T00:00Z&end=${n}T00:00Z")
    if [ "$code" = 200 ]; then mv "$d.tmp" "$d.json"; sleep 2; break; fi
    rm -f "$d.tmp"
    if [ "$code" != 429 ]; then echo "failed $d ($code)" >> failed.txt; break; fi
    sleep $wait; [ $wait -lt 300 ] && wait=$((wait * 2))
  done
  d=$n
done
echo done > ../energy_charts_done.txt
