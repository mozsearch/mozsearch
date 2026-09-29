#!/usr/bin/env python3

import datetime
import os
import subprocess
import sys

# Usage: make-crontab.py SUBJECT_PREFIX DEST_EMAIL HOURS [LOG_NAME]
#
# After HOURS, send a failure email (which shuts the instance down), first
# uploading the log (see upload-log.sh) as a "failed" log named LOG_NAME if
# given.

subj_prefix = sys.argv[1]
dest_email = sys.argv[2]
allowed_runtime_hours = int(sys.argv[3])
log_name = sys.argv[4] if len(sys.argv) > 4 else None

dir_path = os.path.dirname(os.path.realpath(__file__))

delta = datetime.timedelta(hours=allowed_runtime_hours)
when = datetime.datetime.now() + delta
s = when.strftime('%M %H %d %m *')

s += ' '
if log_name:
    s += os.path.join(dir_path, 'upload-log.sh') + ' failed ' + log_name + '; '
s += os.path.join(dir_path, 'send-failure-email.py') + ' ' + subj_prefix + ' ' + dest_email + '\n'

print(s)

p = subprocess.Popen(['crontab', '-'], stdin=subprocess.PIPE)
p.communicate(s.encode())
