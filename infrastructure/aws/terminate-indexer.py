#!/usr/bin/env python2

# Shuts down the indexer instance.
# Usage: terminate-indexer.py <indexer-instance-id>

from __future__ import absolute_import
from __future__ import print_function
import sys
import boto3
from pprint import pprint

client = boto3.client('ec2')

indexerInstanceId = sys.argv[1]

# Note that we don't auto-delete these volumes, because this script
# is invoked by the indexer itself to self-terminate, but in that
# case we don't actually want the volume to be deleted as it will
# continue to be used by the web-server instance.
for volume in client.describe_volumes()['Volumes']:
    for attachment in volume['Attachments']:
        if attachment['InstanceId'] == indexerInstanceId and not attachment['DeleteOnTermination']:
            print(("Volume %s is attached to the indexer and won't be deleted; you may want to delete it if not needed any more" % volume['VolumeId']))
            if len(volume['Attachments']) > 1:
                print("But watch out! The volume is attached to multiple instances")
                pprint(volume['Attachments'])

terminate = [indexerInstanceId]
client.terminate_instances(InstanceIds=terminate)
