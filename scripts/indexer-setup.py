#!/usr/bin/env python

import sys
import os
import os.path

mozSearchRoot = os.environ['MOZSEARCH_PATH']
indexRoot = os.environ['INDEX_ROOT']
treeRoot = os.environ['FILES_ROOT']
objdir = os.environ['OBJDIR']

plugin_folder = os.path.join(mozSearchRoot, 'clang-plugin')

flags = [
    '-load', os.path.join(plugin_folder, 'libclang-index-plugin.so'),
    '-add-plugin', 'mozsearch-index',
    '-plugin-arg-mozsearch-index', treeRoot,
    '-plugin-arg-mozsearch-index', os.path.join(indexRoot, 'analysis'),
    '-plugin-arg-mozsearch-index', objdir,
    '-fparse-all-comments',
]
flags_str = " ".join([ '-Xclang {}'.format(flag) for flag in flags ])

env = {
    'CC': "clang %s" % flags_str,
    'CXX': "clang++ %s" % flags_str,
}

for (k, v) in env.items():
    print 'export {}="{}"'.format(k, v)
