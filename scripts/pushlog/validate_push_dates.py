#!/usr/bin/env python3
"""Check a push dates file (derive_push_dates.py's) against a firefox git
repo: parents pushed after their children (there should be none, but for
clock differences), and push dates against commit dates and the computed
landing dates (the latest of a commit's and its ancestors' commit dates), by
year.

Usage: validate_push_dates.py PUSH_DATES GIT_REPO [REF]
"""

import collections
import datetime
import subprocess
import sys


def bucket(seconds):
    for limit, name in ((-3600, "< -1h"), (-60, "< -1m"), (0, "< 0"), (60, "< 1m"),
                        (3600, "< 1h"), (86400, "< 1d"), (14 * 86400, "< 14d")):
        if seconds < limit:
            return name
    return ">= 14d"


def main():
    push_path, repo = sys.argv[1], sys.argv[2]
    ref = sys.argv[3] if len(sys.argv) > 3 else "HEAD"
    pushed = {}
    for line in open(push_path):
        fields = line.rstrip("\n").split("\t")
        pushed[fields[0]] = int(datetime.datetime.fromisoformat(fields[2].replace("Z", "+00:00")).timestamp())
    proc = subprocess.Popen(["git", "-C", repo, "log", "--topo-order", "--reverse", "--format=%H %ct %P", ref],
                            stdout=subprocess.PIPE, text=True)
    landed = {}
    effective = {}
    inversions = []
    vs_commit = collections.defaultdict(collections.Counter)
    vs_landed = collections.defaultdict(collections.Counter)
    with_push = collections.Counter()
    commits = collections.Counter()
    for line in proc.stdout:
        rev, time, *parents = line.split()
        time = int(time)
        landed[rev] = max([time] + [landed.get(p, 0) for p in parents])
        own = pushed.get(rev, time)
        effective[rev] = max([own] + [effective.get(p, 0) for p in parents])
        year = datetime.datetime.fromtimestamp(effective[rev], datetime.UTC).year
        commits[year] += 1
        if rev not in pushed:
            continue
        with_push[year] += 1
        for p in parents:
            if p in pushed and pushed[p] > pushed[rev]:
                inversions.append((pushed[p] - pushed[rev], rev[:12], p[:12]))
        vs_commit[year][bucket(pushed[rev] - time)] += 1
        vs_landed[year][bucket(pushed[rev] - landed[rev])] += 1
    inversions.sort(reverse=True)
    print(f"commits with push dates: {sum(with_push.values())} of {sum(commits.values())}")
    print(f"parents pushed after their children: {len(inversions)}; largest: {inversions[:5]}")
    order = ["< -1h", "< -1m", "< 0", "< 1m", "< 1h", "< 1d", "< 14d", ">= 14d"]
    print("push - commit date, then push - computed landing date, by year:")
    for year in sorted(commits):
        if not with_push[year]:
            continue
        c = " ".join(f"{k}:{vs_commit[year][k]}" for k in order if vs_commit[year][k])
        l = " ".join(f"{k}:{vs_landed[year][k]}" for k in order if vs_landed[year][k])
        print(f"{year} {with_push[year]}/{commits[year]}\n    commit: {c}\n    landed: {l}")


if __name__ == "__main__":
    main()
