#!/usr/bin/env python

import os
import os.path
import sys
import subprocess
import json

from lib import run

config = json.load(open(sys.argv[1]))
tree_name = sys.argv[2]

repo_path = config['repos'][tree_name]['repo_path']

stdout = run('git ls-files', shell=True, cwd=repo_path)

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

    fullpath = os.path.join(repo_path, path)

    elts = path.split('/')
    for i in range(len(elts)):
        sub = '/'.join(elts[:i])
        if sub and sub not in dirDict:
            dirDict[sub] = True
            dirs.append(sub + '\n')

    files.append(path + '\n')

    (_, ext) = os.path.splitext(path)
    if ext == '.idl':
        # This file causes problems because an IDL file of the same name
        # exists in browser/ and they both end up in dist/include.
        if path != 'mobile/android/components/build/nsIShellService.idl':
            idl.append(path + '\n')

    if 'js/src/tests' in path or 'jit-test' in path:
        continue

    if ext in ['.js', '.jsm', '.xml']:
        js.append(path + '\n')

index_path = config['repos'][tree_name]["index_path"]
open(os.path.join(index_path, 'repo-files'), 'w').writelines(files)
open(os.path.join(index_path, 'repo-dirs'), 'w').writelines(dirs)
open(os.path.join(index_path, 'js-files'), 'w').writelines(js)
open(os.path.join(index_path, 'idl-files'), 'w').writelines(idl)

