#!/usr/bin/env python

import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
log_tail = subprocess.check_output(["tail", "-n", "30", "/home/ubuntu/index-log"])

response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            'searchfox-aws@mozilla.com',
        ]
    },
    Message={
        'Subject': {
            'Data': 'Searchfox indexing error',
        },
        'Body': {
            'Text': {
                'Data': 'Searchfox failed to index successfully! Last 30 lines of log:\n\n' + log_tail,
            },
        }
    }
)

os.system("sudo /sbin/shutdown -h +5")
