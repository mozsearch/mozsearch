#!/usr/bin/env python3

import sys
import boto3
import aws_credentials  # (Before using boto3; see there.)
import datetime
import os
import subprocess

client = boto3.client('ses')
subj_prefix = sys.argv[1]
dest_email = sys.argv[2]

# (Even without the log, the email is sent, and the instance shut down.)
try:
    log_tail = subprocess.check_output(["tail", "-n", "120", "/home/ubuntu/index-log"])
    log_tail = log_tail.decode('utf-8', 'replace')
except (OSError, subprocess.CalledProcessError) as e:
    log_tail = f'(The log could not be read: {e})'

# With KEEP_ON_FAILURE set in the environment (ex: `--setenv KEEP_ON_FAILURE=1`
# for a reblame we're watching), leave the instance running, with its local
# storage, rather than shutting it down, so that we can salvage what it did.
# (The crontab's timeout, from make-crontab.py, runs this without it.)  With
# FAILURE_SHUTDOWN_HOURS (ex: a reblame's; see main.sh), it's shut down after
# that many hours, rather than 5 minutes, which the timeout's has too.
keep = bool(os.environ.get('KEEP_ON_FAILURE'))
shutdown_hours = float(os.environ.get('FAILURE_SHUTDOWN_HOURS') or 0)
shutdown_minutes = max(5, round(shutdown_hours * 60))
if keep:
    kept = 'The instance was left running (KEEP_ON_FAILURE); terminate it when you are done with it.\n\n'
elif shutdown_hours:
    stops = datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=shutdown_minutes)
    kept = (f'The instance stays up, with its local storage, for {shutdown_hours:g} hours, until '
            f'{stops:%Y-%m-%d %H:%M} UTC, so that its work can be salvaged: ssh in (ssh.py), '
            'and `sudo systemctl stop` the poweroff timer (see `systemctl list-timers`) to keep it '
            'longer.  Then it stops, and its local storage is lost.\n\n')
else:
    kept = ''

response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            dest_email,
        ]
    },
    Message={
        'Subject': {
            'Data': subj_prefix + ' Searchfox indexing error',
        },
        'Body': {
            'Text': {
                'Data': 'Searchfox failed to index successfully! ' + kept + 'Last 120 lines of log:\n\n' + log_tail,
            },
        }
    }
)

# Stop the instance after 5 minutes (or FAILURE_SHUTDOWN_HOURS), during which
# we can still ssh in: `shutdown -h +5` would create /run/nologin right away,
# refusing logins (bug 1767477), so this is a timer instead (which `systemctl
# stop` can cancel; see `systemctl list-timers`).
if not keep:
    os.system(f"sudo systemd-run --on-active={shutdown_minutes}min --timer-property=AccuracySec=1s /bin/systemctl poweroff")
