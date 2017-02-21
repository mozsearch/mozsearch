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

TARGET_GROUPS = {
    'release': 'release-target',
    'dev': 'dev-target',
}

targetGroup = TARGET_GROUPS[channel]

ec2_resource = boto3.resource('ec2')
ec2 = boto3.client('ec2')
elb = boto3.client('elbv2')

userData = '''#!/bin/bash

cd ~ubuntu
touch web_server_started
HOME=/home/ubuntu ./update.sh "{branch}" "{config_repo}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/web-serve.sh config
'''.format(branch=branch, channel=channel, config_repo=config_repo)

volumes = ec2.describe_volumes(VolumeIds=[volumeId])
availability_zone = volumes['Volumes'][0]['AvailabilityZone']

if volumes['Volumes'][0]['Attachments']:
    attachment = volumes['Volumes'][0]['Attachments']
    if attachment['State'] == 'attached':
        instance.detach_volume(VolumeId=volumeId)
        awslib.await_volume(ec2, volumeId, 'in-use', 'available')

# - Start the web server instance, tag it as a web server

print 'Starting web server instance...'

images = ec2.describe_images(Filters=[{'Name': 'name', 'Values': ['web-server-16.04']}])
image_id = images['Images'][0]['ImageId']

r = ec2.run_instances(
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

awslib.await_instance(ec2, webServerInstanceId, 'pending', 'running')

print '  State is running.'

print 'Tagging web server instance...'

instances = ec2_resource.instances.filter(InstanceIds=[webServerInstanceId])
webServerInstance = list(instances)[0]

ec2.create_tags(Resources=[webServerInstanceId], Tags=[{
    'Key': 'web-server',
    'Value': str(datetime.now()),
}, {
    'Key': 'channel',
    'Value': channel,
}])

print 'Attaching index volume to web server instance...'

# - Attach INDEX_VOL to the web server
ec2.attach_volume(VolumeId=volumeId, InstanceId=webServerInstanceId, Device='xvdf')

# - Wait until the web server is ready to serve requests

ip = webServerInstance.public_ip_address

while True:
    try:
        subprocess.check_call(["curl", "-f", "-m", "10.0",
                               "http://%s/mozilla-central/search?q=nsGlobalWindow" % ip])
    except:
        time.sleep(1)
        continue
    break

# - Attach the elastic IP to the new web server

print 'Switching requests to new server...'

r = elb.describe_target_groups(Names=[targetGroup])
targetGroupArn = r['TargetGroups'][0]['TargetGroupArn']

r = elb.describe_target_health(TargetGroupArn=targetGroupArn)
oldTargets = []
for targetInfo in r['TargetHealthDescriptions']:
    oldTargets.append(targetInfo['Target'])

elb.register_targets(TargetGroupArn=targetGroupArn,
                     Targets=[{'Id': webServerInstanceId, 'Port': 80}])

if oldTargets:
    elb.deregister_targets(TargetGroupArn=targetGroupArn,
                           Targets=oldTargets)

# - Shut down any old web server (a web server not equal to the one I've started)

print 'Shutting down old servers...'

r = ec2.describe_instances(Filters=[{'Name': 'tag-key', 'Values': ['web-server']},
                                    {'Name': 'tag:channel', 'Values': [channel]}])
terminate = []
for reservation in r['Reservations']:
    for instance in reservation['Instances']:
        instanceId = instance['InstanceId']
        if instanceId != webServerInstanceId:
            terminate.append(instanceId)

print 'Terminating {}'.format(terminate)

if len(terminate):
    ec2.terminate_instances(InstanceIds=terminate)

for instanceId in terminate:
    awslib.await_instance(ec2, instanceId, None, 'terminated')

# - Delete any old EBS index volumes

print 'Deleting old EBS index volumes...'

volumes = ec2.describe_volumes(Filters=[{'Name': 'tag-key', 'Values': ['index']},
                                        {'Name': 'tag:channel', 'Values': [channel]}])
volumes = volumes['Volumes']
for volume in volumes:
    if volumeId != volume['VolumeId']:
        print 'Deleting {}'.format(volume['VolumeId'])
        ec2.delete_volume(VolumeId=volume['VolumeId'])
