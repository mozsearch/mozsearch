#!/usr/bin/env python3

from __future__ import absolute_import
import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
suppression_file = sys.argv[1]
subj_prefix = sys.argv[2]
dest_email = sys.argv[3]
log_location = sys.argv[4]
warning_limit = "50"

# We use the idiom described at
# https://docs.python.org/3/library/subprocess.html#replacing-shell-pipeline
# to run a grep that first excludes any warnings we don't care about and
# then pipe that to our grep that finds warnings and provides "before"
# context.
suppress_proc = subprocess.Popen(["grep", "--invert-match", "-f", suppression_file, log_location], stdout=subprocess.PIPE)

# The regex here intentionally matches any `warn!` logger output from rust code
matches_proc = subprocess.Popen(["grep", "-B16", "-i", "-m", warning_limit, "-P", "^([ ]*|\\[\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}Z )warn", '-'],
                                stdin=suppress_proc.stdout,
                                stdout=subprocess.PIPE)
suppress_proc.stdout.close() # allow SIGPIPE from matches_proc to suppress_proc
warnings, _ =  matches_proc.communicate()

if matches_proc.returncode:
    # grep found no matches, so no need to send this email
    sys.exit(0)

if dest_email == "test":
    print("warnings:\n", warnings.decode())
    sys.exit(0)

warnings = warnings.decode('utf-8', 'replace')

response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            dest_email,
        ]
    },
    Message={
        'Subject': {
            'Data': subj_prefix + ' Searchfox warnings',
        },
        'Body': {
            'Text': {
                'Data': 'Searchfox produced warnings during indexing! The first ' + warning_limit + ' warnings:\n\n' + warnings,
            },
        }
    }
)
