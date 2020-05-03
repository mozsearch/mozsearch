#!/usr/bin/env python2

import json
import sys
from collections import OrderedDict

j = json.load(open(sys.argv[1]), object_pairs_hook=OrderedDict)
components = sys.argv[2].split('/')
for component in components:
    if component not in j:
        print ''
        sys.exit(0)
    j = j[component]

if type(j) == str or type(j) == unicode:
    print j
elif type(j) == dict or type(j) == OrderedDict:
    print ' '.join(j.keys())
else:
    raise Exception('Unexpected type', j)
