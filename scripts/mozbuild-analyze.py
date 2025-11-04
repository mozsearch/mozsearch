#!/usr/bin/env python3

import sys
import os
import re
import json
import ast


def at_escape(text):
  return re.sub("[^A-Za-z0-9_/]", lambda m: "@" + "{:02X}".format(ord(m.group(0))), text)


def to_file_sym(filename):
    return "FILE_" + at_escape(filename)


def to_loc(line, c1, c2):
    return f"{line:05d}:{c1}-{c2}"


class AnalysisWriter:
    def __init__(self, local_path, analysis_path):
        self.test_dir = os.path.dirname(local_path)
        self.analysis_path = analysis_path
        self.items = []

        self.items.append({
            "loc": "00001:0",
            "target": 1,
            "kind": "def",
            "pretty": local_path,
            "sym": to_file_sym(local_path),
        })

    def add_use(self, path, line, c1, c2):
        self.items.append({
            "loc": to_loc(line, c1, c2),
            "target": 1,
            "kind": "use",
            "pretty": path,
            "sym": to_file_sym(path),
        })
        self.items.append({
            "loc": to_loc(line, c1, c2),
            "source": 1,
            "syntax": "use,file",
            "pretty": path,
            "sym": to_file_sym(path),
        })

    def write(self):
        with open(self.analysis_path, "w") as f:
            for item in sorted(self.items, key=lambda x: x["loc"]):
                print(json.dumps(item), file=f)


def analyze(local_path, files_root, analysis_root):
    mozbuild_path = os.path.join(files_root, local_path)
    analysis_path = os.path.join(analysis_root, local_path)

    local_dir = os.path.dirname(local_path)

    w = AnalysisWriter(local_path, analysis_path)

    with open(mozbuild_path, "r") as f:
        text = f.read()
    try:
        root = ast.parse(text)
    except:
        return

    for n in ast.walk(root):
        if isinstance(n, ast.Constant) and isinstance(n.value, str):
            target = n.value
            if target.startswith("!"):
                # Generated files.
                continue

            if target.startswith("/"):
                target = target[1:]
            else:
                target = os.path.join(local_dir, target)

            target = os.path.normpath(target)
            if os.path.isfile(os.path.join(files_root, target)):
                # NOTE: Currently there's no simple way to have a
                #       reference to directory.
                w.add_use(target, n.lineno, n.col_offset, n.end_col_offset)

    w.write()

index_root = sys.argv[1]
files_root = sys.argv[2]
analysis_root = sys.argv[3]

for local_path in sys.stdin:
    local_path = local_path.strip()
    analyze(local_path, files_root, analysis_root)
