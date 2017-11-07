#!/usr/bin/env python

import sys
import boto3
import os

client = boto3.client('ses')

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
                'Data': 'Searchfox failed to index successfully!',
            },
        }
    }
)

os.system("sudo /sbin/shutdown -h +5")
