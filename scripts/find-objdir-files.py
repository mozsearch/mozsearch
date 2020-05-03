#!/usr/bin/env python2

import os
import os.path
import sys
import subprocess

mozSearchRoot = os.environ['MOZSEARCH_PATH']
indexRoot = os.environ['INDEX_ROOT']

p = subprocess.Popen('find . -type f', shell=True, stdout=subprocess.PIPE,
                     cwd=os.path.join(indexRoot, 'analysis', '__GENERATED__'))
(stdout, stderr) = p.communicate()

files = []
dirs = []
dirDict = {}

lines = stdout.split('\n')
for line in lines:
    path = line[2:].strip()
    if not path:
        continue

    if 'conftest' in path:
        continue

    path = '__GENERATED__/' + path

    elts = path.split('/')
    for i in range(len(elts)):
        sub = '/'.join(elts[:i])
        if sub and sub not in dirDict:
            dirDict[sub] = True
            dirs.append(sub + '\n')

    files.append(path + '\n')

open(os.path.join(indexRoot, 'objdir-files'), 'w').writelines(files)
open(os.path.join(indexRoot, 'objdir-dirs'), 'w').writelines(dirs)

