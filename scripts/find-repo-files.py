#!/usr/bin/env python

import os
import os.path
import sys
import subprocess

mozSearchRoot = os.environ['MOZSEARCH_ROOT']
indexRoot = os.environ['INDEX_ROOT']
treeRoot = os.environ['TREE_ROOT']

p = subprocess.Popen('git ls-files', shell=True, stdout=subprocess.PIPE, cwd=treeRoot)
(stdout, stderr) = p.communicate()

files = []
js = []
dirs = ['/\n']
dirDict = {'/': True}

lines = stdout.split('\n')
for line in lines:
    path = line.strip()
    if not path:
        continue

    fullpath = os.path.join(treeRoot, path)

    elts = path.split('/')
    for i in range(len(elts)):
        sub = '/'.join(elts[:i])
        if sub and sub not in dirDict:
            dirDict[sub] = True
            dirs.append('/' + sub + '\n')

    files.append('/' + path + '\n')

    if os.access(fullpath, os.X_OK):
        continue

    pathElts = path.split(os.sep)
    if 'tps.jsm' in pathElts:
        continue
    if 'shell' in pathElts:
        continue
    if 'jit-test' in pathElts:
        continue
    if 'test' in pathElts:
        continue
    if 'tests' in pathElts:
        continue
    if 'mochitest' in pathElts:
        continue
    if 'unit' in pathElts:
        continue
    if 'testing' in pathElts:
        continue

    (_, ext) = os.path.splitext(path)
    if ext in ['.js', '.jsm']:
        js.append('/' + path + '\n')

open(os.path.join(indexRoot, 'all-files'), 'w').writelines(files)
open(os.path.join(indexRoot, 'all-dirs'), 'w').writelines(dirs)
open(os.path.join(indexRoot, 'js-files'), 'w').writelines(js)

