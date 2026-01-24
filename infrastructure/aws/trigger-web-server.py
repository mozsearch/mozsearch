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
# Usage: ./trigger-web-server.py <channel> <mozsearch-repo-url> <mozsearch-rev>
#     <config-repo-url> <config-rev> <config-file-name> <index-volume-id>
#     <check-script> <config-repo-path> <working-dir>
#
# Pass "-" to <check-script> and <working-dir> when using this script
# outside of the indexer.

from __future__ import absolute_import
from __future__ import print_function
import sys
from datetime import datetime, timedelta
import dateutil.parser
import boto3
import awslib
import json
import os
import os.path
import subprocess
import time

channel = sys.argv[1]
mozsearch_repo = sys.argv[2]
mozsearch_rev = sys.argv[3]
config_repo = sys.argv[4]
config_rev = sys.argv[5]
config_file_name = sys.argv[6]
volumeId = sys.argv[7]
check_script = sys.argv[8]
config_repo_path = sys.argv[9]
working_dir = sys.argv[10]
branch_for_display = sys.argv[11]

targetGroup = "%s-target" % channel

ec2_resource = boto3.resource('ec2')
ec2 = boto3.client('ec2')
elb = boto3.client('elbv2')

userData = f'''#!/usr/bin/env bash

cd ~ubuntu

cat > /home/ubuntu/.bashrc_prompt_data <<"FINAL"
MOZSEARCH_PS_KIND="web-server"
MOZSEARCH_PS_CHANNEL="{channel}"
MOZSEARCH_PS_BRANCH="{branch_for_display}"
MOZSEARCH_PS_CONFIG="{config_file_name}"
FINAL

touch web_server_started
sudo -i -u ubuntu ./update.sh "{mozsearch_repo}" "{mozsearch_rev}" "{config_repo}" "{config_rev}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/web-serve.sh config "{config_file_name}" "{volumeId}" "{channel}"
'''

while True:
    volumes = ec2.describe_volumes(VolumeIds=[volumeId])
    index_volume = volumes['Volumes'][0]

    # - Detach the index volume if necessary

    if len(index_volume['Attachments']) > 0:
        print('Detaching the index volume...')

        ec2.detach_volume(
            VolumeId=volumeId,
            Force=True
        )

        print('Waiting before querying again...')
        time.sleep(10)
        continue

    break

availability_zone = index_volume['AvailabilityZone']

# - Start the web server instance, tag it as a web server

print('Starting web server instance...')

images = ec2.describe_images(
    Owners=['self'],
    Filters=[{'Name': 'tag-key', 'Values': ['web-server']}]
)
image_id = images['Images'][0]['ImageId']

# Config files shouldn't be able to do whatever they want.  Instance types must
# first be explicitly allow-listed here.
LEGAL_INSTANCE_TYPES = ['t3.xlarge', 't3.2xlarge']
# Our new default is the 4-core 16GiB t3.xlarge
instance_type = 't3.xlarge'

try:
    config = json.load(open(os.path.join(config_repo_path, config_file_name)))
    maybe_instance_type = config['instance_type']
    if not channel.startswith('release'):
        if maybe_instance_type != instance_type:
            print(f'Non-release channel so using default instance type of {instance_type} instead of {maybe_instance_type}')
        else:
            print(f'Non-release channel using requested (default) instance type of {instance_type}')
    elif maybe_instance_type in LEGAL_INSTANCE_TYPES:
        instance_type = maybe_instance_type
        print(f'Using config file instance type of: "{instance_type}"')
    else:
        print(f'Unknown instance type {maybe_instance_type} requested, falling back to {instance_type}')
except Exception as e:
    print(f'Problem figuring out instance_type from config file: {e}')

r = ec2.run_instances(
    ImageId=image_id,
    MinCount=1,
    MaxCount=1,
    KeyName='Main Key Pair',
    SecurityGroups=['web-server-secure'],
    UserData=userData,
    InstanceType=instance_type,
    Placement={'AvailabilityZone': availability_zone},
    IamInstanceProfile={
        'Name': 'web-server-role',
    },
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
    'Value': config_file_name,
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

if check_script == '-':
    # This branch is for the execution outside of AWS.
    # At this point, the instance is not accessible with HTTP from outside.
    # The equivalent steps of the "else" branch need to be done manually.
    #
    # TODO: Automatically do them by invoking ssh.py ?
    print('Please perform the following steps to ensure the web server is ready:')
    print('  1. Wait until the instance to boot (2-3 minutes)')
    print('  2. SSH into %s' % webServerInstance.id)
    print('  3. Wait until the ~/docroot/status.txt to be present with 2 lines (15min)')
    print('  4. Optionally check the server response')
    print('  5. Hit Enter below to proceed to the next steps')
    print('')
    input('Hit Enter:');
else:
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
                # The time heuristic would catch up to 3 backups, so once we've
                # found a recent-ish back-up to use, stop the others.  This
                # is inherently biased by the sort order so it might be better
                # to create a list of candidates to retain and then pick more
                # deliberately, but this is probably fine given that in steady
                # state we'll always be replacing an instance that was started
                # 12 hours ago so a pathological situation is not possible
                # unless indexers are failing a lot, but this heuristic was
                # never meant to address that.
                elif backups_retained >= 1:
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
