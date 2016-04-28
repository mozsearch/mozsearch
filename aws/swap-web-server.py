# Responsibilities:
# - Start the web server instance, tag it as a web server
# - Attach INDEX_VOL to the web server
# - Wait until the web server is ready to serve requests
#   I could just make requests until they succeed
# - Attach the elastic IP to the new web server
# - Shut down any old web servers (not equal to the one I've started)
# - Delete any old EBS index volumes
#
# Usage: swap-web-server.py <channel> <indexer-instance-id> <index-volume-id>

import sys
from datetime import datetime
import boto3
import awslib
import os
import os.path

channel = sys.argv[1]
indexerInstanceId = sys.argv[2]
volumeId = sys.argv[3]

ELASTIC_IPS = {
    'release': '52.32.131.4',
    'dev': '52.33.247.22',
}

elasticIp = ELASTIC_IPS[channel]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

# - Start the web server instance, tag it as a web server

print 'Starting web server instance...'

userData = open(os.path.join(os.path.dirname(sys.argv[0]), 'web-server-startup.sh')).read()

instances = ec2.instances.filter(InstanceIds=[indexerInstanceId])
indexerInstance = list(instances)[0]

r = client.run_instances(
    ImageId='ami-5189a661', # Ubuntu 14.04
    MinCount=1,
    MaxCount=1,
    KeyName='Main Key Pair',
    SecurityGroups=['web-server'],
    UserData=userData,
    InstanceType='t2.medium',
    Placement={'AvailabilityZone': indexerInstance.placement['AvailabilityZone']},
)

webServerInstanceId = r['Instances'][0]['InstanceId']

awslib.await_instance(client, webServerInstanceId, 'pending', 'running')

print '  State is running.'

print 'Tagging web server instance...'

instances = ec2.instances.filter(InstanceIds=[webServerInstanceId])
webServerInstance = list(instances)[0]

client.create_tags(Resources=[webServerInstanceId], Tags=[{
    'Key': 'web-server',
    'Value': str(datetime.now()),
}, {
    'Key': 'channel',
    'Value': channel,
}])

print 'Attaching index volume to web server instance...'

# - Attach INDEX_VOL to the web server
client.attach_volume(VolumeId=volumeId, InstanceId=webServerInstanceId, Device='xvdf')

# - Wait until the web server is ready to serve requests
#   I could just make requests until they succeed

ip = webServerInstance.public_ip_address

#FIXME!!!!!!!

# - Attach the elastic IP to the new web server

print 'Switching elastic IP address...'

client.associate_address(InstanceId=webServerInstanceId, PublicIp=elasticIp, AllowReassociation=True)

# - Shut down any old web server (a web server not equal to the one I've started)

print 'Shutting down old servers...'

r = client.describe_instances(Filters=[{'Name': 'tag-key', 'Values': ['web-server']},
                                       {'Name': 'tag:channel', 'Values': [channel]}])
terminate = []
for reservation in r['Reservations']:
    for instance in reservation['Instances']:
        instanceId = instance['InstanceId']
        if instanceId != webServerInstanceId:
            terminate.append(instanceId)

print 'Terminating {}'.format(terminate)

if len(terminate):
    client.terminate_instances(InstanceIds=terminate)

for instanceId in terminate:
    awslib.await_instance(client, instanceId, None, 'terminated')

# - Delete any old EBS index volumes

print 'Deleting old EBS index volumes...'

volumes = client.describe_volumes(Filters=[{'Name': 'tag-key', 'Values': ['index']},
                                           {'Name': 'tag:channel', 'Values': [channel]}])
volumes = volumes['Volumes']
for volume in volumes:
    if volumeId != volume['VolumeId']:
        print 'Deleting {}'.format(volume['VolumeId'])
        client.delete_volume(VolumeId=volume['VolumeId'])
