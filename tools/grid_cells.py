# Builds the bundled grid data file (src/modules/Grid frequencies (50 or 60 Hz)/grid-frequencies.json):
# every country's mains frequency and the synchronous areas, as 1° cells.
#
# Borders: Natural Earth 1:10m admin-0 map units, v5.1.2 (public domain; the UK split into its
# countries, Belgium into its regions). Frequencies: Wikipedia, "Mains electricity by country"
# (read 2026-11-18): the 60 Hz list below; every other country 50 Hz.
#
# A cell counts for a country when any part of the country lies in it (its centre inside, or a
# border passing through it), so no coast or small island is lost: the map never wrongly excludes
# a cell, it only fails to exclude one.
#
# Run: python -I tools/grid_cells.py <folder for the download>
# It downloads the borders (13 MB) into that folder once, then writes the JSON.

import json
import math
import os
import sys
import urllib.request

NE_URL = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/ne_10m_admin_0_map_units.geojson"
OUT = os.path.join(os.path.dirname(__file__), "..", "src", "modules", "Grid frequencies (50 or 60 Hz)", "grid-frequencies.json")
ROWS, COLS = 180, 360

# Wikipedia's 60 Hz rows (ADM0_A3 codes). Japan is split by the line below; French Polynesia uses both.
HZ60 = set("ASM AIA ATG ABW BHS BMU BLZ BRA VGB CAN CYM COL CRI CUB DOM ECU SLV GUM GTM GUY HTI HND LBR MEX FSM MSR NIC PLW PAN PER PHL PRI KNA MAF SAU SXM KOR SUR TWN TTO USA VIR VEN".split())
BOTH = {"PYF"}
# Japan: 50 Hz east of the line from the Fuji River mouth to Itoigawa (extended), 60 Hz west.
JP_LINE = ((35.12, 138.63), (37.04, 137.86))

# Synchronous areas: map units (GU_A3, or ADM0_A3 when marked) with dates, and pieces cut out.
# Islands joined only by DC links (asynchronous) are cut out where they are big enough to matter.
CE_ALWAYS = "PRX ESP AND FXX MCO ITA SMR VAT CHE LIE AUT DEU NLX BCR BFR BWR LUX POL CZE SVK HUN SVN HRV BHB BHF BIS SRS SRV KOS MNE MKD ALB GRC BGR ROU MAR DZA TUN".split()
AREAS = [
    ("GB", "Great Britain", 50, [(["ENG", "SCT", "WLS", "IMN"], None, None)]),
    ("IE", "Ireland and Northern Ireland", 50, [(["IRL", "NIR"], None, None)]),
    ("Nordic", "Nordic (Norway, Sweden, Finland, East Denmark)", 50, [(["NOW", "SWE", "FIN", "ALD", "DNK-east"], None, None)]),
    ("CE", "Continental Europe", 50, [
        (CE_ALWAYS + ["DNK-west"], None, None),
        (["TUR"], "2010-09-18", None),
        (["UKR", "MDA"], "2022-03-16", None),
        (["EST", "LVA", "LTU"], "2025-02-09", None),
    ]),
    ("Baltic", "Baltic states (with Russia's grid until 2025-02-08)", 50, [(["EST", "LVA", "LTU"], None, "2025-02-09")]),
]
# Pieces of a unit left out of its synchronous area: (unit, test on a polygon's centre (lat, lon)).
CUT = {
    "ESP": [lambda la, lo: la < 30, lambda la, lo: 38.5 < la < 40.2 and 1.0 < lo < 4.5],  # Canaries; Balearics (HVDC)
    "FXX": [lambda la, lo: 41.2 < la < 43.1 and 8.4 < lo < 9.7],  # Corsica (with Sardinia, HVDC)
    "ITA": [lambda la, lo: 38.8 < la < 41.4 and 8.0 < lo < 10.0],  # Sardinia (HVDC)
    "SCT": [lambda la, lo: la > 59.7, lambda la, lo: lo < -10],  # Shetland (isolated until 2024); Rockall
    "GRC": [lambda la, lo: lo > 26.0 and la > 35.8],  # East Aegean islands (isolated)
}


def load(folder):
    path = os.path.join(folder, "ne_10m_admin_0_map_units.geojson")
    if not os.path.exists(path):
        urllib.request.urlretrieve(NE_URL, path)
    with open(path, encoding="utf-8") as f:
        return json.load(f)["features"]


def polygons(feature):
    g = feature["geometry"]
    return g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]


def centre(poly):
    ring = poly[0]
    return sum(p[1] for p in ring) / len(ring), sum(p[0] for p in ring) / len(ring)


def cell(lat, lon):
    r = min(ROWS - 1, max(0, int(math.floor(90.0 - lat))))
    c = min(COLS - 1, max(0, int(math.floor(lon + 180.0))))
    return r, c


def rasterize(polys, keep=lambda r, c: True):
    """Cells any part of the polygons lies in: centres inside (even-odd over all rings), and every
    cell a border passes through (each edge sampled every 0.02°)."""
    out = set()
    for poly in polys:
        for ring in poly:
            for (x1, y1), (x2, y2) in zip(ring, ring[1:]):
                n = max(1, int(max(abs(x2 - x1), abs(y2 - y1)) / 0.02))
                for k in range(n + 1):
                    out.add(cell(y1 + (y2 - y1) * k / n, x1 + (x2 - x1) * k / n))
        lats = [p[1] for p in poly[0]]
        for r in range(max(0, int(90 - max(lats)) - 1), min(ROWS, int(90 - min(lats)) + 2)):
            y = 89.5 - r
            xs = []
            for ring in poly:
                for (x1, y1), (x2, y2) in zip(ring, ring[1:]):
                    if (y1 > y) != (y2 > y):
                        xs.append(x1 + (y - y1) * (x2 - x1) / (y2 - y1))
            xs.sort()
            for a, b in zip(xs[0::2], xs[1::2]):
                for c in range(max(0, math.ceil(a + 179.5)), min(COLS - 1, math.floor(b + 179.5)) + 1):
                    out.add((r, c))
    return {rc for rc in out if keep(*rc)}


def runs(cells):
    v = []
    for r in range(ROWS):
        cs = sorted(c for (rr, c) in cells if rr == r)
        i = 0
        while i < len(cs):
            a = cs[i]
            while i + 1 < len(cs) and cs[i + 1] == cs[i] + 1:
                i += 1
            v.append([r, a, cs[i]])
            i += 1
    return v


def side(lat, lon):
    (a1, b1), (a2, b2) = JP_LINE
    return (b2 - b1) * (lat - a1) - (a2 - a1) * (lon - b1)


def main():
    feats = load(sys.argv[1] if len(sys.argv) > 1 else ".")
    units = {}
    for f in feats:
        p = f["properties"]
        units[p["GU_A3"]] = (p, polygons(f))

    # Frequencies, grouped by continent.
    groups = {}
    for gu, (p, polys) in units.items():
        a3, cont = p["ADM0_A3"], p["CONTINENT"]
        if cont == "Antarctica":
            continue  # no grid: stations run their own generators, at 50 or 60 Hz
        cont = {"Seven seas (open ocean)": "Ocean islands"}.get(cont, cont)
        name = p["NAME"]
        if a3 == "JPN":
            cells = rasterize(polys)
            # A cell is east (50 Hz) or west (60 Hz) of the line by its corners; across it, both.
            for hz, test in ((50, lambda r, c: max(side(90 - r - dy, c - 180 + dx) for dy in (0, 1) for dx in (0, 1)) < 0),
                             (60, lambda r, c: min(side(90 - r - dy, c - 180 + dx) for dy in (0, 1) for dx in (0, 1)) > 0)):
                part = {rc for rc in cells if test(*rc)}
                groups.setdefault((cont, hz), (set(), []))[0].update(part)
                groups[(cont, hz)][1].append(f"{name} ({'east' if hz == 50 else 'west'})")
            continue
        hzs = [50, 60] if a3 in BOTH else [60] if a3 in HZ60 else [50]
        cells = rasterize(polys)
        for hz in hzs:
            g = groups.setdefault((cont, hz), (set(), []))
            g[0].update(cells)
            g[1].append(name)
    regions = [
        {"id": f"{cont} {hz} Hz", "frequency_hz": hz, "countries": sorted(names), "cells": runs(cells)}
        for (cont, hz), (cells, names) in sorted(groups.items())
    ]

    # Synchronous areas.
    def unit_cells(u):
        if u.startswith("DNK-"):
            # Denmark: Jutland and Funen in CE, Zealand and Bornholm in Nordic; the Great Belt
            # (about 11° E) runs through the cells from 10° to 11° E, which count for both.
            west = u == "DNK-west"
            return rasterize(units["DNK"][1], lambda r, c: (c - 180 < 11) if west else (c - 180 >= 10))
        p, polys = units[u]
        cuts = CUT.get(u, [])
        polys = [q for q in polys if not any(t(*centre(q)) for t in cuts)]
        return rasterize(polys)

    areas = []
    for aid, name, hz, parts in AREAS:
        out = []
        for us, frm, to in parts:
            cells = set()
            for u in us:
                cells |= unit_cells(u)
            part = {"units": us, "cells": runs(cells)}
            if frm:
                part["from"] = frm
            if to:
                part["to"] = to
            out.append(part)
        areas.append({"id": aid, "name": name, "frequency_hz": hz, "parts": out})

    doc = {
        "source": "Natural Earth 1:10m admin-0 map units v5.1.2 (public domain) with each country's frequency from Wikipedia, Mains electricity by country (2026-11-18); a 1° cell counts when any part of the country lies in it. Made by tools/grid_cells.py",
        "regions": regions,
        "synchronous_areas": areas,
    }
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, ensure_ascii=False, separators=(",", ":"))
        f.write("\n")
    print(f"{len(regions)} regions, {len(areas)} synchronous areas -> {os.path.normpath(OUT)} ({os.path.getsize(OUT) // 1024} KB)")


if __name__ == "__main__":
    main()
