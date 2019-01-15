#!/usr/bin/env python

import datetime
import os
import subprocess

subj_prefix = sys.argv[1]
dest_email = sys.argv[2]

dir_path = os.path.dirname(os.path.realpath(__file__))

delta = datetime.timedelta(hours=6)
when = datetime.datetime.now() + delta
s = when.strftime('%M %H %d %m *')

s += ' ' + os.path.join(dir_path, 'send-failure-email.py') + ' ' + subj_prefix + ' ' + dest_email + '\n'

print s

p = subprocess.Popen(['crontab', '-'], stdin=subprocess.PIPE)
p.communicate(s)
