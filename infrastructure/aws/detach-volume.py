#!/usr/bin/env python2

# Detaches an EBS volume from an instance to which it's attached.
#
# Usage: detach-volume.py <instance-id> <volume-id>

import sys
import boto3
import awslib

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

instanceId = sys.argv[1]
instances = ec2.instances.filter(InstanceIds=[instanceId])
instance = list(instances)[0]

# Detach the index EBS volume from the instance.
volumeId = sys.argv[2]
instance.detach_volume(VolumeId=volumeId)

awslib.await_volume(client, volumeId, 'in-use', 'available')
