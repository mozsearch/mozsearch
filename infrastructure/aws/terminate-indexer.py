#!/usr/bin/env python3

# Shuts down the indexer instance.
# Usage: terminate-indexer.py <indexer-instance-id>

from __future__ import absolute_import
import sys
import boto3

client = boto3.client('ec2')

indexerInstanceId = sys.argv[1]
terminate = [indexerInstanceId]
client.terminate_instances(InstanceIds=terminate)
