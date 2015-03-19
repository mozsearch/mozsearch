import os
import os.path

import os
from os.path import join, getsize

ext_sizes = {}

total_size = 0
for root, dirs, files in os.walk('.'):
    total_size += sum(getsize(join(root, name)) for name in files)

    for f in files:
        (_, ext) = os.path.splitext(f)

        if os.access(join(root, f), os.X_OK):
            continue

        if ext in ['.js', '.jsm']:
            print join(root, f)

    if 'objdir-ff-opt' in dirs:
        dirs.remove('objdir-ff-opt')
    if 'objdir-ff-dbg' in dirs:
        dirs.remove('objdir-ff-dbg')
    if 'test' in dirs:
        dirs.remove('test')
    if 'tests' in dirs:
        dirs.remove('tests')
    if 'mochitest' in dirs:
        dirs.remove('mochitest')
    if 'unit' in dirs:
        dirs.remove('unit')
    if 'testing' in dirs:
        dirs.remove('testing')
    if '.git' in dirs:
        dirs.remove('.git')
