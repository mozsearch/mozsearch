#!/usr/bin/env python3
"""Turn commits into the lines they changed, in the coordinates of an
index's (possibly later) versions of their files, for `searchfox-tool
dataflow --slice`.

Usage: dataflow-patch-lines.py <git checkout> <index files root> <commit>...

Outputs JSON: {"commits": [...], "files": {path: {"added": [lines],
"removed": [anchor lines]}}}, where added lines are the commits' new-side
lines that survive (unchanged) into the index's version, and removed lines
are the index lines where removed code was (the line after it).
"""

import difflib
import json
import os
import re
import subprocess
import sys


def git(repo, *args):
    return subprocess.run(
        ["git", "-C", repo, *args], check=True, capture_output=True, text=True
    ).stdout


def main():
    repo, root, *commits = sys.argv[1:]
    files = {}
    for commit in commits:
        diff = git(repo, "show", "--format=", "--unified=0", "--no-renames", commit)
        path = None
        hunks = {}
        for line in diff.splitlines():
            if line.startswith("+++ "):
                path = line[6:] if line.startswith("+++ b/") else None
                continue
            m = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", line)
            if m and path:
                old_count = int(m.group(2) or "1")
                new_start = int(m.group(3))
                new_count = int(m.group(4) or "1")
                hunks.setdefault(path, []).append((new_start, new_count, old_count))
        for path, path_hunks in hunks.items():
            index_path = os.path.join(root, path)
            if not os.path.exists(index_path):
                continue
            post = git(repo, "show", f"{commit}:{path}").splitlines()
            with open(index_path, encoding="utf-8", errors="replace") as f:
                current = f.read().splitlines()
            # Map the commit's lines to the index's, through unchanged runs.
            mapping = {}
            matcher = difflib.SequenceMatcher(None, post, current, autojunk=False)
            for tag, i1, i2, j1, j2 in matcher.get_opcodes():
                if tag == "equal":
                    for k in range(i2 - i1):
                        mapping[i1 + k + 1] = j1 + k + 1
            entry = files.setdefault(path, {"added": [], "removed": []})
            for new_start, new_count, old_count in path_hunks:
                for line in range(new_start, new_start + new_count):
                    if new_count and line in mapping:
                        entry["added"].append(mapping[line])
                if old_count and not new_count:
                    # (Removed lines: where they were.)
                    anchor = mapping.get(new_start + 1) or mapping.get(new_start)
                    if anchor:
                        entry["removed"].append(anchor)
    for entry in files.values():
        entry["added"] = sorted(set(entry["added"]))
        entry["removed"] = sorted(set(entry["removed"]))
    json.dump({"commits": commits, "files": files}, sys.stdout, indent=1)


if __name__ == "__main__":
    main()
