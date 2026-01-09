#!/usr/bin/env python3

# Creates an EBS volume for the index and attaches it to a given instance as /dev/xvdf.
# Prints the volume ID on stdout.
# Usage: attach-index-volume.py <channel> <instance-id>

from __future__ import absolute_import
from __future__ import print_function
import sys
import boto3
import awslib
from datetime import datetime

channel = sys.argv[1]
instanceId = sys.argv[2]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

# Find availability zone
instances = ec2.instances.filter(InstanceIds=[instanceId])
instance = list(instances)[0]

# We've been using 300G for quite some time but with the addition of esr140 to
# config2, we hit the 300G limit; the math indicates we probably need at least
# 305G before the nginx cache which we cap at 20G.  To provide headroom, I'm
# going to use 400 for release2 but we can revisit as we rebalance things.
#
# release3 also takes ~300GB, where each mozilla-esr* take 25GB to 40GB,
# and there are 8 repositories. comm-* don't take much space.
volumeSize = 300
if channel == 'release2' or channel == 'release3':
    volumeSize = 400

r = client.create_volume(
    Size=volumeSize,
    VolumeType='gp2',
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
