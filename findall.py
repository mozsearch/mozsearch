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

        if ext not in ['.sqlite', '.webm', '.opus', '.ico', '.mp4', '.wav', '.bmp', '.reg', '.icns', '.ttx', '.jar', '.ttf', '.vcproj', '.ogv', '.png', '.pyc', '.jpg', '.woff']:
            print join(root, f)
            ext_sizes[ext] = ext_sizes.get(ext, 0) + getsize(join(root, f))

    if 'objdir-ff-opt' in dirs:
        dirs.remove('objdir-ff-opt')
    if 'objdir-ff-dbg' in dirs:
        dirs.remove('objdir-ff-dbg')
    if '.git' in dirs:
        dirs.remove('.git')

other = 0
for ext in ext_sizes:
    sz = ext_sizes[ext]/1024/1024
    #if sz:
    #    print sz, ext
    #else:
    #    other += ext_sizes[ext]
    #print ext, sz
#print 'OTHER', other/1024/1024
#print 'TOTAL', total_size/1024/1024
