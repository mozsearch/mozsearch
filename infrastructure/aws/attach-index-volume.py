#!/usr/bin/env python3

# Creates an EBS volume for the index and attaches it to a given instance as /dev/xvdf.
# Prints the volume ID on stdout.
# Usage: attach-index-volume.py <channel> <instance-id> <config-file>

import sys
import boto3
import awslib
from datetime import datetime
from index_volume import volume_size

channel = sys.argv[1]
instanceId = sys.argv[2]
configFile = sys.argv[3]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

# Find availability zone
instances = ec2.instances.filter(InstanceIds=[instanceId])
instance = list(instances)[0]

volumeSize = volume_size(channel, configFile)
print(f'Creating a {volumeSize} GiB index volume', file=sys.stderr)

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
