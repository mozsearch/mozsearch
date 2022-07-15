#!/usr/bin/env python3

# Responsibilities:
# - Start the web server instance, tag it as a web server
# - Attach INDEX_VOL to the web server
# - Wait until the web server is ready to serve requests
#   I could just make requests until they succeed
# - Attach the elastic IP to the new web server
# - Shut down any old web servers (not equal to the one I've started)
# - Delete any old EBS index volumes
#
# Usage: ./trigger-web-server.py <branch> <channel> <config-repo-url> <config-input> <index-volume-id> <check-script> <config-repo-path> <working-dir>

from __future__ import absolute_import
from __future__ import print_function
import sys
from datetime import datetime, timedelta
import dateutil.parser
import boto3
import awslib
import os
import os.path
import subprocess
import time

branch = sys.argv[1]
channel = sys.argv[2]
mozsearch_repo = sys.argv[3]
config_repo = sys.argv[4]
config_input = sys.argv[5]
volumeId = sys.argv[6]
check_script = sys.argv[7]
config_repo_path = sys.argv[8]
working_dir = sys.argv[9]

targetGroup = "%s-target" % channel

ec2_resource = boto3.resource('ec2')
ec2 = boto3.client('ec2')
elb = boto3.client('elbv2')

userData = '''#!/usr/bin/env bash

cd ~ubuntu
touch web_server_started
sudo -i -u ubuntu ./update.sh "{branch}" "{mozsearch_repo}" "{config_repo}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/web-serve.sh config "{config_input}" "{volume_id}"
'''.format(branch=branch, channel=channel, mozsearch_repo=mozsearch_repo, config_repo=config_repo, config_input=config_input, volume_id=volumeId)

volumes = ec2.describe_volumes(VolumeIds=[volumeId])
availability_zone = volumes['Volumes'][0]['AvailabilityZone']

# - Start the web server instance, tag it as a web server

print('Starting web server instance...')

images = ec2.describe_images(Filters=[{'Name': 'tag-key', 'Values': ['web-server']}])
# TODO: sort/pick the highest datestamp-y "web-server" tag Value.
image_id = images['Images'][0]['ImageId']

r = ec2.run_instances(
    ImageId=image_id,
    MinCount=1,
    MaxCount=1,
    KeyName='Main Key Pair',
    SecurityGroups=['web-server-secure'],
    UserData=userData,
    # t3 gets us "nitro" NVME EBS and is cheaper than t2.
    #
    # We're now upgrading from t3.large (2 cores) to t3.xlarge (4 cores) because
    # we're seeing CPU-limited codesearch queries.  The increase from 8GiB to
    # 16GiB is also expected to be overall good for performance as caching is
    # arguably good for performance as well!
    InstanceType='t3.xlarge',
    Placement={'AvailabilityZone': availability_zone},
)

webServerInstanceId = r['Instances'][0]['InstanceId']

awslib.await_instance(ec2, webServerInstanceId, 'pending', 'running')

print('  State is running.')

print('Tagging web server instance...')

instances = ec2_resource.instances.filter(InstanceIds=[webServerInstanceId])
webServerInstance = list(instances)[0]

ec2.create_tags(Resources=[webServerInstanceId], Tags=[{
    'Key': 'web-server',
    'Value': str(datetime.now()),
}, {
    'Key': 'channel',
    'Value': channel,
}, {
    'Key': 'cfile',
    'Value': config_input,
}])

print('Attaching index volume to web server instance...')

# - Attach INDEX_VOL to the web server
ec2.attach_volume(VolumeId=volumeId, InstanceId=webServerInstanceId, Device='xvdf')

# - Wait for it to be attached, and then mark it as DeleteOnTermination
awslib.await_volume(ec2, volumeId, 'available', 'in-use')
webServerInstance.modify_attribute(BlockDeviceMappings=[{
    'DeviceName': 'xvdf',
    'Ebs': {
        'DeleteOnTermination': True,
    },
}])

# - Wait until the web server is ready to serve requests

ip = webServerInstance.private_ip_address

print('Pinging web-server at %s to check readiness...' % ip)

while True:
    try:
        status = subprocess.check_output(
            ["curl", "-f", "-s", "-m", "10.0", "http://%s/status.txt" % ip])
        print('Got status.txt: [%s]' % status)
        if len(status.splitlines()) < 2:
            time.sleep(10)
            continue
    except:
        time.sleep(10)
        continue
    break

# - Run the sanity checks on the web server to ensure it is serving things fine

print('Checking web-server at %s to ensure served data seems reasonable...' % ip)

subprocess.run([check_script, config_repo_path, working_dir, "http://%s/" % ip],
               check=True)

# - Attach the elastic IP to the new web server

print('Switching requests to new server...')

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

print('Shutting down old servers...')

r = ec2.describe_instances(Filters=[{'Name': 'tag-key', 'Values': ['web-server']},
                                    {'Name': 'tag:channel', 'Values': [channel]}])
terminate = []
backups_retained = 0
for reservation in r['Reservations']:
    for instance in reservation['Instances']:
        instanceId = instance['InstanceId']
        if instanceId == webServerInstanceId:
            # Don't kill the one we just started
            continue
        tags = instance['Tags']
        kill = False
        for tag in tags:
            if tag['Key'] == 'web-server':
                t = dateutil.parser.parse(tag['Value'])
                # Leave old release1-channel servers around so we can switch
                # to them in an emergency or for quick testing.
                if channel != "release1" or datetime.now() - t >= timedelta(1.5):
                    kill = True
                # we're starting with 2 backups for now because this is a
                # decrease from 3, but we will move this to 1 once we're sure
                # I didn't mess this up too much.
                elif backups_retained >= 2:
                    kill = True
                else:
                    backups_retained += 1

        if kill:
            terminate.append(instanceId)

print('Terminating {}'.format(terminate))

if len(terminate):
    ec2.terminate_instances(InstanceIds=terminate)

for instanceId in terminate:
    awslib.await_instance(ec2, instanceId, None, 'terminated')

# - Report any old EBS unattached index volumes.
# Since they are marked as DeleteOnTermination we shouldn't need to delete
# volumes explicitly, but let's report if we find any that are older than half
# a day. This is because within the first half-day of a volume's life it may
# be temporarily unattached (on creation, or during transfer from indexer to
# web-server).

print('Checking for old EBS index volumes...')

volumes = ec2_resource.volumes.filter(
    Filters=[{'Name': 'tag-key', 'Values': ['index']},
             {'Name': 'tag:channel', 'Values': [channel]},
             {'Name': 'status', 'Values': ['available']}])
for volume in volumes:
    for tag in volume.tags:
        if tag['Key'] == 'index':
            t = dateutil.parser.parse(tag['Value'])
            if datetime.now() - t >= timedelta(0.5):
                print('WARNING: Found stray index volume %s created on %s' % (volume.volume_id, tag['Value']))
