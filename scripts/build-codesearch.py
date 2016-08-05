#!/usr/bin/env python

import os
import sys
import json

from lib import run

def copy_objdir_files(dest_dir, config):
    for d in open(os.path.join(config['index_path'], 'objdir-dirs')).readlines():
        d = d.strip()
        run(['mkdir', '-p', os.path.join(dest_dir, d)])

    paths = open(os.path.join(config['index_path'], 'objdir-files')).readlines()
    for path in paths:
        path = path.strip()
        source = path.replace('__GENERATED__', config['objdir_path'])
        try:
            data = open(source).read()
        except:
            continue

        dest = os.path.join(dest_dir, path)
        f = open(dest, 'w')
        f.write(data)
        f.close()

os.mkdir('/tmp/dummy')

config_fname = sys.argv[1]

livegrep_config = {
    'name': 'Searchfox',
    'repositories': [],
    'fs_paths': [],
}

config = json.load(open(config_fname))
repos = config['trees']
for key in repos:
    repo_name = key

    if 'git_path' in repos[key]:
        run(['ln', '-s', repos[key]['git_path'], '/tmp/dummy/%s' % key])

        livegrep_config['repositories'].append({
            'name': key,
            'path': repos[key]['git_path'],
            'revisions': ['HEAD']
        })
    else:
        run(['ln', '-s', repos[key]['files_path'], '/tmp/dummy/%s' % key])

        # If we don't include the trailing '/', then all search
        # results will include an initial slash in their paths.
        livegrep_config['fs_paths'].append({
            'name': key,
            'path': repos[key]['files_path'] + '/'
        })

    tmp_objdir = '/tmp/dummy/objdir-%s' % key
    os.mkdir(tmp_objdir)
    copy_objdir_files(tmp_objdir, repos[key])

    livegrep_config['fs_paths'].append({
        'name': key + '-__GENERATED__',
        'path': 'objdir-%s/' % key,
    })

json.dump(livegrep_config, open('/tmp/livegrep.json', 'w'))

run(['codesearch', '/tmp/livegrep.json', '-dump_index', '%s/livegrep.idx' % config['livegrep_path'],
     '-max_matches', '1000'], stdin=open('/dev/null'), cwd='/tmp/dummy')

run(['rm', '-rf', '/tmp/dummy'])
run(['rm', '-rf', '/tmp/livegrep.json'])
