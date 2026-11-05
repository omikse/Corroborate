"""Log the aircraft around a point from public ADS-B (Automatic Dependent Surveillance–Broadcast)
data, as ground truth for audio recorded there.

Polls the adsb.lol API (open data, ODbL licence) every 10 s and appends one JSON line per poll:
    {"pc_utc": ..., "data_utc": ..., "api": ..., "ac": [{hex, flight, r, t, category, type,
     lat, lon, alt_baro, alt_geom, gs, track, baro_rate, seen_pos}, ...]}
Times are Unix seconds. `data_utc` is the API's own clock; positions are `seen_pos` seconds older.
When the API asks us to slow down (HTTP 429) or fails, the wait doubles, up to a minute, and goes
back to normal after the next good poll; the failure is logged as {"pc_utc", "error", "next_poll_s"}.
Only aircraft that transmit ADS-B (or are seen by MLAT, multilateration) and are in range of a
volunteer receiver appear: small planes without ADS-B Out, or flying low, can be missing.

Usage:
    python tools/adsb-log.py --lat 42.4797 --lon -76.4514 --radius-nm 25 --minutes 80 --out log.jsonl
Stop early with Ctrl+C; every poll is written as it arrives.
"""

import argparse
import json
import time
import urllib.request

URL = "https://api.adsb.lol/v2/point/{lat}/{lon}/{r}"
MAX_WAIT = 60.0
KEEP = ["hex", "flight", "r", "t", "category", "type", "lat", "lon", "alt_baro", "alt_geom",
        "gs", "track", "baro_rate", "seen_pos"]


def poll(lat, lon, radius_nm):
    req = urllib.request.Request(URL.format(lat=lat, lon=lon, r=radius_nm),
                                 headers={"User-Agent": "doppler-live research logger"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.load(resp)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--lat", type=float, required=True)
    ap.add_argument("--lon", type=float, required=True)
    ap.add_argument("--radius-nm", type=float, default=25)
    ap.add_argument("--every", type=float, default=10, help="seconds between polls")
    ap.add_argument("--minutes", type=float, default=60)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    end = time.time() + a.minutes * 60
    n = 0
    wait = a.every
    with open(a.out, "a", encoding="utf-8") as f:
        while time.time() < end:
            t0 = time.time()
            try:
                d = poll(a.lat, a.lon, a.radius_nm)
                ac = [{k: x[k] for k in KEEP if k in x} for x in d.get("ac", [])]
                data_utc = d.get("now", t0 * 1000) / 1000
                f.write(json.dumps({"pc_utc": round(t0, 3), "data_utc": round(data_utc, 3), "api": "adsb.lol", "ac": ac}) + "\n")
                n += 1
                wait = a.every
                if n % 30 == 1:
                    print(f"{time.strftime('%H:%M:%S', time.gmtime(t0))} UTC  poll {n}  {len(ac)} aircraft", flush=True)
            except Exception as e:  # rate limit or network trouble: back off
                wait = min(MAX_WAIT, wait * 2)
                f.write(json.dumps({"pc_utc": round(t0, 3), "error": str(e), "next_poll_s": wait}) + "\n")
                print(f"poll failed: {e}; next in {wait:.0f} s", flush=True)
            f.flush()
            time.sleep(max(0.0, wait - (time.time() - t0)))
    print(f"done: {n} polls", flush=True)


if __name__ == "__main__":
    main()
