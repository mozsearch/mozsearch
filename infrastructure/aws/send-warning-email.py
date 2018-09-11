#!/usr/bin/env python

import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
subj_prefix = sys.argv[1]
dest_email = sys.argv[2]
warning_limit = "50"

try:
    # The regex here intentionally matches any `warn!` logger output from rust code
    warnings = subprocess.check_output(["grep", "-i", "-m", warning_limit, "^[[:space:]]*warn", "/home/ubuntu/index-log"])
except subprocess.CalledProcessError:
    # grep found no matches, so no need to send this email
    sys.exit(0)

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
