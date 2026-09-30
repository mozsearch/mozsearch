#!/usr/bin/env python3

import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
subj_prefix = sys.argv[1]
dest_email = sys.argv[2]

log_tail = subprocess.check_output(["tail", "-n", "120", "/home/ubuntu/index-log"])
log_tail = log_tail.decode('utf-8', 'replace')

# With KEEP_ON_FAILURE set in the environment (ex: `--setenv KEEP_ON_FAILURE=1`
# for a reblame we're watching), leave the instance running, with its local
# storage, rather than shutting it down, so that we can salvage what it did.
# (The crontab's timeout, from make-crontab.py, runs this without it.)
keep = bool(os.environ.get('KEEP_ON_FAILURE'))
kept = ('The instance was left running (KEEP_ON_FAILURE); terminate it when you are done with it.\n\n'
        if keep else '')

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

if not keep:
    os.system("sudo /sbin/shutdown -h +5")
