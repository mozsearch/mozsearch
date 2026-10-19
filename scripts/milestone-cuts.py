#!/usr/bin/env python3

# Firefox's version cuts, from the history of the tree's config/milestone.txt
# (for the file timeline's releases; see static/js/file-timeline.js): the
# commit that bumps mozilla-central's version to `N.0a1` (on merge day, when
# N-1 goes to beta) is N-1's cut, and its parent is the last of N-1's
# mozilla-central commits.  (Since Firefox 5: before the rapid releases, the
# version wasn't Firefox's.)  Writes, as JSON, `[version, rev, parent, time]`
# for each cut, oldest first.
#
# Usage: milestone-cuts.py GIT_ROOT

import json
import re
import subprocess
import sys

git_root = sys.argv[1]

log = subprocess.run(
    ['git', '-C', git_root, 'log', '--first-parent', '--no-renames',
     '--format=commit\t%H\t%P\t%ct', '-p', 'HEAD', '--', 'config/milestone.txt'],
    check=True, capture_output=True, text=True).stdout

# The commits that changed the version, newest first: [rev, parent, time,
# version].
commits = []
for line in log.split('\n'):
    if line.startswith('commit\t'):
        _, rev, parents, time = line.split('\t')
        commits.append([rev, (parents.split() or [None])[0], int(time), None])
        continue
    match = re.match(r'^\+(\d+)\.0a1$', line)
    if match and commits:
        commits[-1][3] = int(match.group(1))

cuts = {}
for rev, parent, time, version in reversed(commits):
    # (The first bump to a version, if it was backed out and relanded.)
    if version is not None and version >= 6 and parent and version - 1 not in cuts:
        cuts[version - 1] = [version - 1, rev, parent, time]

json.dump(sorted(cuts.values()), sys.stdout)
