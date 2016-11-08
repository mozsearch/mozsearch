#!/usr/bin/env python

import sys
import boto3

client = boto3.client('ses')

response = client.send_email(
    Source='daemon@searchfox.org',
    Destination={
        'ToAddresses': [
            'wmccloskey@mozilla.com',
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
