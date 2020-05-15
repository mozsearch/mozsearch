#!/usr/bin/env python2

from __future__ import absolute_import
from __future__ import print_function
import datetime
import os
import subprocess
import sys

subj_prefix = sys.argv[1]
dest_email = sys.argv[2]

dir_path = os.path.dirname(os.path.realpath(__file__))

delta = datetime.timedelta(hours=6)
when = datetime.datetime.now() + delta
s = when.strftime('%M %H %d %m *')

s += ' ' + os.path.join(dir_path, 'send-failure-email.py') + ' ' + subj_prefix + ' ' + dest_email + '\n'

print(s)

p = subprocess.Popen(['crontab', '-'], stdin=subprocess.PIPE)
p.communicate(s)
