#!/usr/bin/env python

import urllib
import re

req = urllib.urlopen('http://hg.mozilla.org/mozilla-central/rev/tip')
data = req.read()
req.close()

m = re.search('changeset [0-9]+:([0-9a-z]+)', data)
print m.group(1)


