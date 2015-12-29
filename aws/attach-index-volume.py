# Creates an EBS volume for the index and attaches it to a given instance as /dev/xvdf.
# Prints the volume ID on stdout.
# Usage: attach-index-volume.py <instance-id>

import sys
import boto3
import awslib
from datetime import datetime

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

r = client.create_volume(
    Size=30,
    VolumeType='gp2',
    AvailabilityZone='us-west-2',
)

volumeId = r['VolumeId']
awslib.await_volume(client, volumeId, 'creating', 'available')

client.create_tags(Resources=[volumeId], Tags=[{
    'Key': 'index',
    'Value': str(datetime.now()),
}])

instanceId = sys.argv[1]
instances = ec2.instances.filter(InstanceIds=[instanceId])
instance = list(instances)[0]

instance.attach_volume(VolumeId=volumeId, Device='xvdf')

awslib.await_volume(client, volumeId, 'available', 'in-use')

print volumeId
