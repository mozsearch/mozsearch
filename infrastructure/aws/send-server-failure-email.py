#!/usr/bin/env python3

from __future__ import absolute_import
import sys
import os
import subprocess

channel = sys.argv[1]
server_name = sys.argv[2]
dest_email = sys.argv[3]
log_location = sys.argv[4]
log_lines = 100

result = subprocess.run(['tail', '-{}'.format(log_lines), log_location], stdout=subprocess.PIPE)

output = result.stdout.decode('utf-8', 'replace')
body = 'Searchfox {} continuously failed! The last {} lines of the output:\n\n'.format(server_name, log_lines) + output

if dest_email == 'NO_EMAIL':
    print('')
    print('================================')
    print(body)
    print('================================')
    sys.exit(0)

import boto3
client = boto3.client('ses', region_name='us-west-2')
response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            dest_email,
        ]
    },
    Message={
        'Subject': {
            'Data': '{} Searchfox {} continuously failed'.format(channel, server_name),
        },
        'Body': {
            'Text': {
                'Data': body,
            },
        }
    }
)
