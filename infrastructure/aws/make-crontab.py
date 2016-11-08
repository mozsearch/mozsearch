#!/usr/bin/env python

import datetime
import os
import subprocess

dir_path = os.path.dirname(os.path.realpath(__file__))

delta = datetime.timedelta(hours=4)
when = datetime.datetime.now() + delta
s = when.strftime('%M %H %d %m *')

s += ' ' + os.path.join(dir_path, 'send-email.py') + '\n'

print s

p = subprocess.Popen(['crontab', '-'], stdin=subprocess.PIPE)
p.communicate(s)
