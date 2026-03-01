#!/usr/bin/env python3

import os
import sys

files_path = sys.argv[1]
dirs_path = sys.argv[2]

dirs = set()


def push_dirs(dirs, path):
    while True:
        path = os.path.dirname(path)
        if path == '':
            break
        if path == '__GENERATED__':
            break
        dirs.add(path)


with open(files_path, "r") as f:
    for line in f:
        push_dirs(dirs, line.strip())


with open(dirs_path, "w") as f:
    for d in sorted(dirs):
        print(d, file=f)
