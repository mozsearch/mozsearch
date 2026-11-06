#!/usr/bin/env python3

"""Split a pyright project into chunks for scip-python-index.sh.

  scip-python-chunks.py ROOT CONFIG N PREFIX

CONFIG is a pyright configuration file whose include, exclude and extraPaths
are relative to ROOT.  The project's Python files (.py and .pyi, under its
include entries but not its exclude entries, as pyright finds them) are split
by directory, deeper where a directory has more than a chunk's share of their
bytes, and the directories (and files) go, the biggest first, to the chunk
with the fewest bytes so far, of N.  Each chunk's configuration, CONFIG with
the chunk's directories and files as its include, is written to
ROOT/PREFIXI.json (for I from 0; fewer than N if there are fewer entries).
Entries with wildcards aren't split: the project is one chunk then.
"""

import json
import os
import sys

PYTHON_EXTENSIONS = (".py", ".pyi")


def main():
    root, config_path, chunks, prefix = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
    with open(config_path) as f:
        config = json.load(f)
    include = config.get("include", ["."])
    exclude = config.get("exclude", [])

    def write(index, entries):
        with open(os.path.join(root, f"{prefix}{index}.json"), "w") as f:
            json.dump(dict(config, include=entries), f)

    if any(any(c in entry for c in "*?[") for entry in include + exclude):
        write(0, include)
        return

    def normal(path):
        path = os.path.normpath(path)
        return "" if path == "." else path

    excluded = {normal(entry) for entry in exclude}

    # The bytes of each file, by path relative to ROOT.
    sizes = {}
    for entry in include:
        entry = normal(entry)
        start = os.path.join(root, entry)
        if os.path.isfile(start):
            if entry.endswith(PYTHON_EXTENSIONS) and entry not in excluded:
                sizes[entry] = os.path.getsize(start)
            continue
        for dirpath, dirnames, filenames in os.walk(start):
            rel = os.path.relpath(dirpath, root)
            rel = "" if rel == "." else rel
            dirnames[:] = sorted(
                d for d in dirnames if os.path.join(rel, d) not in excluded
            )
            for name in filenames:
                path = os.path.join(rel, name)
                if name.endswith(PYTHON_EXTENSIONS) and path not in excluded:
                    try:
                        sizes[path] = os.path.getsize(os.path.join(root, path))
                    except OSError:
                        pass
    share = sum(sizes.values()) / chunks

    # The entries, (path, bytes): the top-level ones of each include entry,
    # those with more than a share split into theirs.
    def split(paths, depth):
        groups = {}
        for path in paths:
            parts = path.split("/")
            key = "/".join(parts[: depth + 1])
            groups.setdefault(key, []).append(path)
        out = []
        for key, group in sorted(groups.items()):
            total = sum(sizes[p] for p in group)
            if total > share and key not in sizes:
                out.extend(split(group, depth + 1))
            else:
                out.append((key, total))
        return out

    entries = []
    for entry in include:
        entry = normal(entry)
        under = [p for p in sizes if entry == "" or p == entry or p.startswith(entry + "/")]
        depth = 0 if entry == "" else entry.count("/") + 1
        if entry in sizes:
            entries.append((entry, sizes[entry]))
        else:
            entries.extend(split(under, depth))

    bins = [[0, []] for _ in range(chunks)]
    for key, total in sorted(entries, key=lambda e: (-e[1], e[0])):
        lightest = min(bins, key=lambda b: b[0])
        lightest[0] += total
        lightest[1].append(key)
    index = 0
    for total, keys in bins:
        if keys:
            write(index, sorted(keys))
            print(f"chunk {index}: {len(keys)} entries, {total} bytes")
            index += 1


if __name__ == "__main__":
    main()
