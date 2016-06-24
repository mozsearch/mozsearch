#!/usr/bin/env python

import json
import sys

j = json.load(open(sys.argv[1]))
components = sys.argv[2].split('/')
for component in components:
    j = j[component]

if type(j) == str or type(j) == unicode:
    print j
elif type(j) == dict:
    print ' '.join(j.keys())
else:
    raise 'Unexpected type', j

