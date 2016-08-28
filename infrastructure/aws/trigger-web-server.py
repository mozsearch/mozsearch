# Responsibilities:
# - Start the web server instance, tag it as a web server
# - Attach INDEX_VOL to the web server
# - Wait until the web server is ready to serve requests
#   I could just make requests until they succeed
# - Attach the elastic IP to the new web server
# - Shut down any old web servers (not equal to the one I've started)
# - Delete any old EBS index volumes
#
# Usage: swap-web-server.py <branch> <channel> <config-repo> <index-volume-id>

import sys
from datetime import datetime
import boto3
import awslib
import os
import os.path
import subprocess
import time

branch = sys.argv[1]
channel = sys.argv[2]
config_repo = sys.argv[3]
volumeId = sys.argv[4]

ELASTIC_IPS = {
    'release': '52.32.131.4',
    'dev': '52.33.247.22',
}

elasticIp = ELASTIC_IPS[channel]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

userData = '''#!/bin/bash

cd ~ubuntu
touch web_server_started
./update.sh "{branch}" "{config_repo}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/web-serve.sh config
'''.format(branch=branch, channel=channel, config_repo=config_repo)

volumes = client.describe_volumes(VolumeIds=[volumeId])
availability_zone = volumes['Volumes'][0]['AvailabilityZone']

if volumes['Volumes'][0]['Attachments']:
    attachment = volumes['Volumes'][0]['Attachments']
    if attachment['State'] == 'attached':
        instance.detach_volume(VolumeId=volumeId)
        awslib.await_volume(client, volumeId, 'in-use', 'available')

# - Start the web server instance, tag it as a web server

print 'Starting web server instance...'

images = client.describe_images(Filters=[{'Name': 'name', 'Values': ['web-server-16.04']}])
image_id = images['Images'][0]['ImageId']

r = client.run_instances(
    ImageId=image_id,
    MinCount=1,
    MaxCount=1,
    KeyName='Main Key Pair',
    SecurityGroups=['web-server'],
    UserData=userData,
    InstanceType='t2.large',
    Placement={'AvailabilityZone': availability_zone},
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

ip = webServerInstance.public_ip_address

while True:
    try:
        subprocess.check_call(["curl", "-f", "-m", "5.0",
                               "http://%s/mozilla-central/search?q=nsGlobalWindow" % ip])
    except:
        time.sleep(1)
        continue
    break

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
