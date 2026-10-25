#!/usr/bin/env python3
"""Derive a push dates file (see the mozsearch tools' `hyperblame::landing::
PushDates`) from an archive of hg.mozilla.org's pushlogs (scrape_pushlog.py's)
and a git-hg revision mapping.

Usage: derive_push_dates.py ARCHIVE GIT_HG_TSV[.gz] OUT --until YYYY-MM-DD
                             --graph GIT_REPO [--ref REF]

Each hg changeset's push date is its first push to any of the archive's repos
(mozilla-central, the integration repos, the release repos): when it landed.
That can't be before its parents' first pushes, as a repo has a changeset's
parents before it.  Left out:
- pushes from UNTIL on, when hg was a mirror of the canonical git repo (its
  pushes replicate git's landings, whose commit dates are when they landed).
  2025-06-01 for firefox: from then on, main's git commit dates are within a
  minute or two of their pushes (medians 24-115 s by month), and none is a
  day or more before it, while before then some are;
- pushes of more than 1,500 changesets, which brought history from
  elsewhere, whose date isn't when its changesets landed: mozilla-central's
  first (13,383 changesets from before the pushlog extension, 2008-03-20),
  the imports of Firefox Sync's (2010-06-23), the mobile repo's (2011-04-06),
  Servo's (2017-02-04) and the Android repos' histories (2024-03-18), and
  project branches merged whole (ex: IonMonkey's, 2012-09-11).  (Regular
  merges' changesets were pushed before, in smaller pushes to the
  integration repos, which those changesets' dates are from.  The vendoring
  of libwebrtc's 5,106 commits on 2022-08-30 is left out too, but their commit
  dates are that day.)  Their changesets' dates are then their commit dates
  (as landing dates, the latest of their ancestors').

And a revision landed before its children were pushed, so its date is at
most theirs (and their descendants'), from GIT_REPO's commit graph: the
pushlogs miss a few pushes (some chunks of 1,000 push IDs have 993-999), and
a changeset in a missing push has a later push's date otherwise (ex: its
merge into mozilla-central, 42 hours after its child was pushed to inbound),
which would carry forward to everything after it as a landing date.  Its row
then has its child's push.

OUT has `GIT_REV<TAB>HG_REV<TAB>PUSHED<TAB>REPO<TAB>PUSH_ID` lines, sorted by
git revision, PUSHED being ISO 8601 in UTC, for the changesets the mapping
has.
"""

import argparse
import collections
import datetime
import glob
import gzip
import json
import os
import subprocess

MAX_CHANGESETS = 1500

# For pushes at the same time, the repo to name: repos created as clones kept
# their origin's pushlog (ex: autoland's first 30,000 or so pushes are
# mozilla-central's), so the origin first.
REPO_ORDER = ["mozilla-central", "integration/", "services/", "releases/"]


def repo_rank(repo):
    return next(i for i, prefix in enumerate(REPO_ORDER + [""]) if repo.startswith(prefix))


def open_text(path):
    return gzip.open(path, "rt") if path.endswith(".gz") else open(path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("archive")
    parser.add_argument("git_hg")
    parser.add_argument("out")
    parser.add_argument("--until", required=True)
    parser.add_argument("--graph", required=True)
    parser.add_argument("--ref", default="HEAD")
    args = parser.parse_args()
    until = datetime.datetime.fromisoformat(args.until).replace(tzinfo=datetime.UTC).timestamp()

    # hg changeset -> (date, repo rank, repo, push id) of its first push.
    first = {}
    # (hg changeset -> the date of its first push of more than MAX_CHANGESETS.)
    imported = {}
    stats = collections.Counter()
    for repo_dir in sorted(glob.glob(os.path.join(args.archive, "*"))):
        if not os.path.isdir(repo_dir):
            continue
        repo = os.path.basename(repo_dir).replace("_", "/", 1)
        for chunk in sorted(glob.glob(os.path.join(repo_dir, "*.json.gz"))):
            pushes = json.load(gzip.open(chunk))["pushes"]
            for push_id, push in pushes.items():
                push_id = int(push_id)
                stats[f"{repo} pushes"] += 1
                if push["date"] >= until:
                    stats[f"{repo} pushes from --until"] += 1
                    continue
                if len(push["changesets"]) > MAX_CHANGESETS:
                    stats[f"{repo} pushes of imported history"] += 1
                    for changeset in push["changesets"]:
                        imported[changeset] = min(push["date"], imported.get(changeset, push["date"]))
                    continue
                key = (push["date"], repo_rank(repo), repo, push_id)
                for changeset in push["changesets"]:
                    if changeset not in first or key < first[changeset]:
                        first[changeset] = key
    # (An imported changeset's first push otherwise is a later push which had
    # it again, if any, ex: a merge into another repo, which isn't when it
    # landed either; but one pushed before, ex: to an integration repo before
    # a big merge, keeps that push.)
    for changeset, date in imported.items():
        if changeset in first and first[changeset][0] >= date:
            del first[changeset]
            stats["imported changesets left out"] += 1
    stats["changesets with push dates"] = len(first)
    stats["imported changesets"] = len(imported)

    # git revision -> (hg revision, (date, repo rank, repo, push id)).
    pushed = {}
    for line in open_text(args.git_hg):
        git_rev, hg_rev = line.rstrip("\n").split("\t")[:2]
        found = first.get(hg_rev)
        if found is None:
            stats["mapped commits without a push date"] += 1
            continue
        pushed[git_rev] = (hg_rev, found)
    stats["changesets the mapping lacks"] = len(first) - len({hg for hg, _ in pushed.values()})

    # (Children first: each commit's bound is the earliest push of it and its
    # descendants, passed through commits without push dates.)
    bound = {}
    graph = subprocess.Popen(["git", "-C", args.graph, "rev-list", "--topo-order", "--parents", args.ref],
                             stdout=subprocess.PIPE, text=True)
    for line in graph.stdout:
        rev, *parents = line.split()
        own = pushed.get(rev)
        mine = bound.pop(rev, None)
        if own is not None:
            if mine is not None and mine < own[1]:
                pushed[rev] = (own[0], mine)
                stats["rows dated by a child's push"] += 1
            else:
                mine = own[1]
        if mine is not None:
            for parent in parents:
                if parent not in bound or mine < bound[parent]:
                    bound[parent] = mine
    if graph.wait() != 0:
        raise Exception("git rev-list failed")

    rows = []
    for git_rev, (hg_rev, (date, _, repo, push_id)) in pushed.items():
        when = datetime.datetime.fromtimestamp(date, datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
        rows.append(f"{git_rev}\t{hg_rev}\t{when}\t{repo}\t{push_id}\n")
        stats[f"rows first pushed to {repo}"] += 1
    rows.sort()
    tmp = args.out + ".tmp"
    with open(tmp, "w") as f:
        f.writelines(rows)
    os.rename(tmp, args.out)
    stats["rows"] = len(rows)
    for key in sorted(stats):
        print(f"{key}: {stats[key]}")


if __name__ == "__main__":
    main()
