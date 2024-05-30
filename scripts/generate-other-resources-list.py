#!/usr/bin/env python3

import json
import sys

analysis_files_path = sys.argv[1]
url_map_path = sys.argv[2]
other_resources_path = sys.argv[3]

with open(analysis_files_path, "r") as f:
    analysis_files = set(f.read().split("\n"))

with open(url_map_path, "r") as f:
    aliases_map = json.load(f)

other_resources = set()

for (sym, aliases) in aliases_map.items():
    for item in aliases:
        path = item["pretty"].replace("file ", "")

        if path in analysis_files:
            continue

        other_resources.add(path)

with open(other_resources_path, "w") as f:
    for path in other_resources:
        print(path, file=f)
