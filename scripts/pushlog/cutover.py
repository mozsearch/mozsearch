#!/usr/bin/env python3
"""When hg became git's mirror: for each month, main's commits' first pushes
(from the archive), by whether Lando's sync pushed them, and the median of
push - git commit date.  Usage: cutover.py ARCHIVE GIT_HG_TSV.gz GIT_REPO"""

import collections
import datetime
import glob
import gzip
import json
import os
import statistics
import subprocess
import sys

archive, git_hg, repo = sys.argv[1:4]
first = {}
for repo_dir in sorted(glob.glob(os.path.join(archive, "*"))):
    if not os.path.isdir(repo_dir):
        continue
    name = os.path.basename(repo_dir)
    for chunk in sorted(glob.glob(os.path.join(repo_dir, "*.json.gz"))):
        for push_id, push in json.load(gzip.open(chunk))["pushes"].items():
            if push["date"] < 1704067200:  # 2024
                continue
            key = (push["date"], name, push["user"])
            for cs in push["changesets"]:
                if cs not in first or key < first[cs]:
                    first[cs] = key
hg_of = {}
for line in gzip.open(git_hg, "rt"):
    g, h = line.split()
    if h in first:
        hg_of[g] = h
out = subprocess.run(["git", "-C", repo, "log", "--since=2024-06-01", "--format=%H %ct", "HEAD"],
                     capture_output=True, text=True).stdout
months = collections.defaultdict(lambda: collections.defaultdict(list))
for line in out.splitlines():
    g, t = line.split()
    h = hg_of.get(g)
    if not h:
        continue
    date, name, user = first[h]
    month = datetime.datetime.fromtimestamp(date, datetime.UTC).strftime("%Y-%m")
    who = "lando" if "lando.moz.tools" in user else "people"
    months[month][who].append(date - int(t))
for month in sorted(months):
    parts = []
    for who in ("people", "lando"):
        gaps = months[month][who]
        if gaps:
            parts.append(f"{who} {len(gaps)} median {statistics.median(gaps):.0f}s "
                         f"(<0: {sum(1 for g in gaps if g < 0)}, >1d: {sum(1 for g in gaps if g > 86400)})")
    print(month, "; ".join(parts))
