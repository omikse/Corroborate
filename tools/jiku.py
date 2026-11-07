"""Fetch one event of the Jiku Mobile Video Dataset for the sync tests: its synchronization ground
truth and the audio of every recording in it.

    python tools/jiku.py GT_090912

Writes tests/data/jiku/<event>/: <event>.xml (the ground truth), every <recording>.mp4 (kept: the
videos are the dataset) and <recording>.m4a (its audio track, copied as it is: no re-encoding,
for the tests). Files already there are skipped, so a rerun picks up where it stopped. Nothing
here is committed (tests/data is ignored). GT_090912 is 12.8 GB.

Dataset: M. Saini, S. P. Venkatagiri, W. T. Ooi, M. C. Chan, "The Jiku Mobile Video Dataset",
MMSys 2013 (mirror at UMass). Ground truth: M. Guggenberger, M. Lux, L. Böszörmenyi,
"A Synchronization Ground Truth for the Jiku Mobile Video Dataset", MMM 2015
(github.com/protyposis/JikuMVD-SynchronizationGroundTruth).
"""

import os
import re
import shutil
import subprocess
import sys
import urllib.request

GT = "https://raw.githubusercontent.com/protyposis/JikuMVD-SynchronizationGroundTruth/master/groundtruth/{}.xml"
VIDEO = "http://skuld.cs.umass.edu/traces/mmsys/2013/jiku/dataset/{}.mp4"


def fetch(url, path):
    tmp = path + ".part"
    with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f, 1 << 20)
    os.replace(tmp, path)


def main():
    event = sys.argv[1] if len(sys.argv) > 1 else "GT_090912"
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tests", "data", "jiku", event)
    os.makedirs(root, exist_ok=True)
    xml = os.path.join(root, event + ".xml")
    if not os.path.exists(xml):
        fetch(GT.format(event), xml)
    names = sorted(set(re.findall(r'<recording name="([^"]+)"', open(xml, encoding="utf-8").read())))
    for i, name in enumerate(names, 1):
        audio = os.path.join(root, name + ".m4a")
        video = os.path.join(root, name + ".mp4")
        if os.path.exists(video) and os.path.exists(audio):
            continue
        print(f"{i}/{len(names)} {name}", flush=True)
        if not os.path.exists(video):
            fetch(VIDEO.format(name), video)
        if not os.path.exists(audio):
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", video, "-vn", "-c:a", "copy", audio + ".part.m4a"], check=True)
            os.replace(audio + ".part.m4a", audio)
    print(f"done: {len(names)} recordings in {os.path.normpath(root)}")


if __name__ == "__main__":
    main()
