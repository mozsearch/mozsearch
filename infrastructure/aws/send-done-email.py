#!/usr/bin/env python

import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
dest_email = sys.argv[1]

response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            dest_email,
        ]
    },
    Message={
        'Subject': {
            'Data': 'Searchfox indexing complete',
        },
        'Body': {
            'Text': {
                'Data': 'Searchfox completed indexing successfully!',
            },
        }
    }
)
