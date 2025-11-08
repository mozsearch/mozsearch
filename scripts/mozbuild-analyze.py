#!/usr/bin/env python3

import sys
import os
import re
import json
import ast


def at_escape(text):
  return re.sub("[^A-Za-z0-9_/]", lambda m: "@" + "{:02X}".format(ord(m.group(0))), text)


def to_sym(prefix, name):
    return prefix + "_" + at_escape(name)


def to_loc(line, c1, c2):
    return f"{line:05d}:{c1}-{c2}"


def get_platforms(obj_root):
    platforms = [obj_root]
    for name in os.listdir(obj_root):
        objdir = os.path.join(obj_root, name)
        if os.path.isdir(objdir) and name.startswith("__") and name.endswith("__"):
            platforms.append(name)
    return platforms


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
            "sym": to_sym("FILE", local_path),
        })

    def add_use(self, prefix, path, line, c1, c2):
        self.items.append({
            "loc": to_loc(line, c1, c2),
            "target": 1,
            "kind": "use",
            "pretty": path,
            "sym": to_sym(prefix, path),
        })
        self.items.append({
            "loc": to_loc(line, c1, c2),
            "source": 1,
            "syntax": "use,file",
            "pretty": path,
            "sym": to_sym(prefix, path),
        })

    def write(self):
        with open(self.analysis_path, "w") as f:
            for item in sorted(self.items, key=lambda x: x["loc"]):
                print(json.dumps(item), file=f)


def analyze(local_path, files_root, analysis_root, obj_root, platforms):
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
            # Generated files in EXPORTS has "!" prefix,
            # But generated files in GeneratedFile arguments doesn't.
            if n.value.startswith("!"):
                target = n.value[1:]
            else:
                target = n.value

            if target.startswith("/"):
                local_target = target[1:]
            else:
                local_target = os.path.join(local_dir, target)

            local_target = os.path.normpath(local_target)
            if os.path.isfile(os.path.join(files_root, local_target)):
                w.add_use("FILE", local_target, n.lineno, n.col_offset, n.end_col_offset)
                continue
            elif os.path.isdir(os.path.join(files_root, local_target)):
                w.add_use("DIR", local_target, n.lineno, n.col_offset, n.end_col_offset)
                continue

            for platform in platforms:
                if target.startswith("/"):
                    local_target = target[1:]
                else:
                    local_target = os.path.join(local_dir, target)

                local_target = os.path.normpath(local_target)
                if os.path.isfile(os.path.join(obj_root, platform, local_target)):
                    webpath = os.path.join("__GENERATED__", platform, local_target)
                    w.add_use("FILE", webpath, n.lineno, n.col_offset, n.end_col_offset)

    w.write()


index_root = sys.argv[1]
files_root = sys.argv[2]
obj_root = sys.argv[3]
analysis_root = sys.argv[4]

platforms = get_platforms(obj_root)

for local_path in sys.stdin:
    local_path = local_path.strip()
    analyze(local_path, files_root, analysis_root, obj_root, platforms)
