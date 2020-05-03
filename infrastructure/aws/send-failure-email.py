#!/usr/bin/env python2

import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
subj_prefix = sys.argv[1]
dest_email = sys.argv[2]

log_tail = subprocess.check_output(["tail", "-n", "30", "/home/ubuntu/index-log"])

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
                'Data': 'Searchfox failed to index successfully! Last 30 lines of log:\n\n' + log_tail,
            },
        }
    }
)

os.system("sudo /sbin/shutdown -h +5")
