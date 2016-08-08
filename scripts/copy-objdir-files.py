#!/usr/bin/env python

# Usage: copy-objdir-files.py <dest-dir>

import os
import os.path
import sys
import subprocess

mozSearchPath = os.environ['MOZSEARCH_PATH']
indexRoot = os.environ['INDEX_ROOT']
objdir = os.environ['OBJDIR']

destDir = sys.argv[1]

for d in open(os.path.join(indexRoot, 'objdir-dirs')).readlines():
    d = d.strip()
    os.system('mkdir -p {}'.format(os.path.join(destDir, d)))

paths = open(os.path.join(indexRoot, 'objdir-files')).readlines()
for path in paths:
    path = path.strip()
    source = path.replace('__GENERATED__', objdir)
    try:
        data = open(source).read()
    except:
        continue

    dest = os.path.join(destDir, path)
    f = open(dest, 'w')
    f.write(data)
    f.close()
