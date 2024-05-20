#!/usr/bin/env python3

import json
import glob
import os
import sys

analysis_dir = sys.argv[1]
alias_map_path = sys.argv[2]


def replace_aliases(path, alias_map):
    """Replace URL records with FILE records, based on the mapping extracted
    from *.chrome-map.json files.

    Note that the analysis record transformations done below are only safe for
    URL records which are currently never referenced for contextsym purposes
    or by structured records. Any more involved transformations should likely
    happen in rust code where we have the analysis.rs types available and can
    easily add helper transforms."""

    has_alias = False
    lines = []

    with open(path, "r") as f:
        for line in f:
            line = line.rstrip()
            if "URL_" not in line:
                lines.append(line)
                continue

            datum = json.loads(line)
            sym = datum["sym"]

            if sym not in alias_map:
                lines.append(line)
                continue

            has_alias = True
            
            for alias in alias_map[sym]:
                datum["sym"] = alias["sym"]
                datum["pretty"] = alias["pretty"]

                lines.append(json.dumps(datum))

    if not has_alias:
        return

    with open(path, "w") as f:
        for line in lines:
            print(line, file=f)


with open(alias_map_path, "r") as f:
    alias_map = json.load(f)

for path in sys.stdin:
    path = path.rstrip()
    if not path:
        continue

    replace_aliases(os.path.join(analysis_dir, path), alias_map)
