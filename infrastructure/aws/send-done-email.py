#!/usr/bin/env python2

from __future__ import absolute_import
import sys
import boto3
import os
import subprocess

client = boto3.client('ses')
subj_prefix = sys.argv[1]
dest_email = sys.argv[2]

response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            dest_email,
        ]
    },
    Message={
        'Subject': {
            'Data': subj_prefix + ' Searchfox indexing complete',
        },
        'Body': {
            'Text': {
                'Data': 'Searchfox completed indexing successfully!',
            },
        }
    }
)
