#!/usr/bin/env python

# Usage: copy-objdir-files.py <dest-dir>

import os
import os.path
import sys
import subprocess

mozSearchRoot = os.environ['MOZSEARCH_ROOT']
indexRoot = os.environ['INDEX_ROOT']
treeRoot = os.environ['TREE_ROOT']
objdir = os.environ['OBJDIR']

destDir = sys.argv[1]

for d in open(os.path.join(indexRoot, 'objdir-dirs')).readlines():
    d = d.strip()
    os.system('mkdir -p {}'.format(os.path.join(destDir, d)))

paths = open(os.path.join(indexRoot, 'objdir-files')).readlines()
for path in paths:
    path = path.strip()
    source = path.replace('__GENERATED__', objdir)
    data = open(source).read()

    dest = os.path.join(destDir, path)
    f = open(dest, 'w')
    f.write(data)
    f.close()
