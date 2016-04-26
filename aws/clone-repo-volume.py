# Usage: clone-repo-volume.py <channel> <instance-id>

import sys
import boto3
import awslib
from datetime import datetime

channel = sys.argv[1]
instanceId = sys.argv[2]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

snapshots = client.describe_snapshots(Filters=[{'Name': 'tag-key', 'Values': ['repo']},
                                               {'Name': 'tag:channel', 'Values': [channel]}])
snapshots = list(volumes['Snapshots'])
snapshotId = snapshots[0]['SnapshotId']

# Find availability zone
instances = ec2.instances.filter(InstanceIds=[instanceId])
instance = list(instances)[0]

r = client.create_volume(
    SnapshotId=snapshotId,
    VolumeType='gp2',
    AvailabilityZone=instance.placement['AvailabilityZone'],
)

volumeId = r['VolumeId']
awslib.await_volume(client, volumeId, 'creating', 'available')

client.create_tags(Resources=[volumeId], Tags=[{
    'Key': 'repo',
    'Value': str(datetime.now()),
}, {
    'Key': 'channel',
    'Value': channel,
}])

instance.attach_volume(VolumeId=volumeId, Device='xvdg')

awslib.await_volume(client, volumeId, 'available', 'in-use')

print volumeId
