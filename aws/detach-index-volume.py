# Detaches an EBS volume (this index) from the indexer instance.
#
# Usage: detach-index-volume.py <indexer-instance-id> <index-volume-id>

import sys
import boto3
import awslib

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

indexerInstanceId = sys.argv[1]
instances = ec2.instances.filter(InstanceIds=[indexerInstanceId])
indexerInstance = list(instances)[0]

# - Detach the index EBS volume from the indexer
volumeId = sys.argv[2]
indexerInstance.detach_volume(VolumeId=volumeId)

awslib.await_volume(client, volumeId, 'in-use', 'available')
