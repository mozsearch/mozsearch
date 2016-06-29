# Uploads data to an S3 bucket
#
# Usage: detach-volume.py <filename> <bucket> <key>

import sys
import boto3
import awslib

filename = sys.argv[1]
bucket = sys.argv[2]
key = sys.argv[3]

s3 = boto3.resource('s3')

data = open(filename, 'rb')
s3.Bucket(bucket).put_object(Key=key, Body=data)
