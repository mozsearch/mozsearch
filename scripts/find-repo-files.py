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
idl = []
dirs = []
dirDict = {}

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
            dirs.append(sub + '\n')

    files.append(path + '\n')

    if os.access(fullpath, os.X_OK):
        continue

    (_, ext) = os.path.splitext(path)
    if ext == '.idl':
        idl.append(path + '\n')

    if 'js/src/tests' in path or 'jit-test' in path:
        continue

    if ext in ['.js', '.jsm', '.xml']:
        js.append(path + '\n')

open(os.path.join(indexRoot, 'repo-files'), 'w').writelines(files)
open(os.path.join(indexRoot, 'repo-dirs'), 'w').writelines(dirs)
open(os.path.join(indexRoot, 'js-files'), 'w').writelines(js)
open(os.path.join(indexRoot, 'idl-files'), 'w').writelines(idl)

