#!/usr/bin/env python2

# Deletes the specified EBS volume. If it is in use by an
# instance, we wait until it is detached and in the 'available'
# state before deleting it.
#
# Usage: delete-volume.py <volume-id>

from __future__ import absolute_import
from __future__ import print_function
import sys
import boto3
import awslib

ec2 = boto3.client('ec2')

volumeId = sys.argv[1]
volume = ec2.describe_volumes(VolumeIds=[volumeId])['Volumes'][0]
if volume['State'] != 'available':
    print(("Volume is in state %s, waiting for it to go into state available..." % volume['State']))
    awslib.await_volume(ec2, volumeId, volume['State'], 'available')

ec2.delete_volume(VolumeId=volumeId)
print(("Volume %s deleted" % volumeId))
