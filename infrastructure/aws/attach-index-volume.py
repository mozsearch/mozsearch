#!/usr/bin/env python3

# Creates an EBS volume for the index and attaches it to a given instance as /dev/xvdf.
# Prints the volume ID on stdout.
# Usage: attach-index-volume.py <channel> <instance-id>

import sys
import boto3
import awslib
from datetime import datetime
from index_volume import volume_size

channel = sys.argv[1]
instanceId = sys.argv[2]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

# Find availability zone
instances = ec2.instances.filter(InstanceIds=[instanceId])
instance = list(instances)[0]

volumeSize = volume_size(channel)

# gp3, with more throughput than its baseline 125 MB/s, which bounds the
# indexer's block-level copy of the index to the volume and web servers'
# copies of it to their SSDs (see index.sh and web-serve.sh): 1000 MB/s, gp3's
# most, which needs at least 4000 IOPS (up to 0.25 MB/s per IOPS).  (Web
# servers without an SSD serve the index from the volume, for which gp3's IOPS
# are also more than gp2's 3 per GiB.)
r = client.create_volume(
    Size=volumeSize,
    VolumeType='gp3',
    Iops=4000,
    Throughput=1000,
    AvailabilityZone=instance.placement['AvailabilityZone'],
)

volumeId = r['VolumeId']
awslib.await_volume(client, volumeId, 'creating', 'available')

client.create_tags(Resources=[volumeId], Tags=[{
    'Key': 'index',
    'Value': str(datetime.now()),
}, {
    'Key': 'channel',
    'Value': channel,
}])


instance.attach_volume(VolumeId=volumeId, Device='xvdf')

awslib.await_volume(client, volumeId, 'available', 'in-use')

instance.modify_attribute(BlockDeviceMappings=[{
    'DeviceName': 'xvdf',
    'Ebs': {
        'DeleteOnTermination': True,
    },
}])

print(volumeId)
