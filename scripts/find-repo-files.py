#!/usr/bin/env python3

import collections
import json
import runpy
import os
import sys

from lib import run

config_repo = sys.argv[1]
config = json.load(open(sys.argv[2]))
tree_name = sys.argv[3]

# Dynamically import the repo_files.py script from the tree's scripts in the
# config repo.
try:
    repo_files = runpy.run_path(os.path.join(config_repo, tree_name, 'repo_files.py'))
except FileNotFoundError:
    # For simplicity allow the tree config to not have the script, in which case
    # we fall back to some default behaviour.
    repo_files = {}

tree_config = config['trees'][tree_name]
tree_repo = tree_config['files_path']
lines = run(['git', 'ls-files', '--recurse-submodules'], cwd=tree_repo).splitlines()
if len(lines) == 0:
    # find . -type f -printf '%P\n'
    lines = run(['/usr/bin/find', '.', '-type', 'f', '-printf', '%P\n'], cwd=tree_repo).splitlines()

if 'modify_file_list' in repo_files:
    lines = repo_files['modify_file_list'](lines, config=tree_config)

files = []
js = []
html = []
css = []
idl = []
ipdl = []

dirs = collections.OrderedDict()
ipdl_dirs = collections.OrderedDict()

for line in lines:
    path = line.strip()
    if not path:
        continue
    path = path.decode()

    fullpath = os.path.join(tree_repo, path)

    elts = path.split('/')
    for i in range(len(elts)):
        sub = '/'.join(elts[:i])
        if sub and sub not in dirs:
            dirs[sub] = True

    files.append(path + '\n')

    (_, ext) = os.path.splitext(path)
    if ext == '.idl':
        if 'filter_idl' in repo_files:
            if not repo_files['filter_idl'](path):
                continue

        idl.append(path + '\n')

    if ext in ['.ipdl', '.ipdlh']:
        if 'filter_ipdl' in repo_files:
            if not repo_files['filter_ipdl'](path):
                continue

        ipdl.append(path + '\n')

        dir = '/'.join(elts[:-1])
        ipdl_dirs[dir] = True

    if ext in ['.js', '.jsm', '.mjs', '.xml', '.xul', '.inc']:
        if 'filter_js' in repo_files:
            if not repo_files['filter_js'](path):
                continue

        js.append(path + '\n')

    if ext in ['.html', '.xhtml']:
        if 'filter_html' in repo_files:
            if not repo_files['filter_html'](path):
                continue

        html.append(path + '\n')

    if ext in ['.css']:
        if 'filter_css' in repo_files:
            if not repo_files['filter_css'](path):
                continue

        css.append(path + '\n')

index_path = tree_config['index_path']
open(os.path.join(index_path, 'repo-files'), 'w').writelines(files)
open(os.path.join(index_path, 'repo-dirs'), 'w').writelines([d + '\n' for d in dirs])
open(os.path.join(index_path, 'js-files'), 'w').writelines(js)
open(os.path.join(index_path, 'html-files'), 'w').writelines(html)
open(os.path.join(index_path, 'css-files'), 'w').writelines(css)
open(os.path.join(index_path, 'idl-files'), 'w').writelines(idl)
open(os.path.join(index_path, 'ipdl-files'), 'w').writelines(ipdl)
open(os.path.join(index_path, 'ipdl-includes'), 'w').write(' '.join(['-I ' + d for d in ipdl_dirs]))
