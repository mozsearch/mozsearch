#!/usr/bin/env python

import os
import os.path
import sys
import subprocess
import json
import collections

from lib import run

config = json.load(open(sys.argv[1]))
tree_name = sys.argv[2]

repo_path = config['trees'][tree_name]['files_path']

files = run("find . -not -name '*~' -and -not -name '.'", shell=True, cwd=repo_path)
lines = files.split('\n')

files = []
js = []
idl = []
ipdl = []
dirs = []
dirDict = {}
ipdl_dirs = collections.OrderedDict()

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
        # This file causes problems because an IDL file of the same
        # name exists in browser/, android/, and other places, and
        # they all end up in dist/include.
        if not path.endswith('nsIShellService.idl'):
            idl.append(path + '\n')

    if ext == '.ipdl' or ext == '.ipdlh':
        if 'ipc/ipdl/test' in path or 'accessible/ipc/win' in path or 'widget/win' in path:
            continue

        ipdl.append(path + '\n')

        dir = '/'.join(elts[:-1])
        ipdl_dirs[dir] = True

    if 'js/src/tests' in path or 'jit-test' in path:
        continue

    if ext in ['.js', '.jsm', '.xml', '.xul', '.inc']:
        js.append(path + '\n')

index_path = config['trees'][tree_name]['index_path']
open(os.path.join(index_path, 'repo-files'), 'w').writelines(files)
open(os.path.join(index_path, 'repo-dirs'), 'w').writelines(dirs)
open(os.path.join(index_path, 'js-files'), 'w').writelines(js)
open(os.path.join(index_path, 'idl-files'), 'w').writelines(idl)
open(os.path.join(index_path, 'ipdl-files'), 'w').writelines(ipdl)
open(os.path.join(index_path, 'ipdl-includes'), 'w').write(' '.join([ '-I ' + d for d in ipdl_dirs ]))
