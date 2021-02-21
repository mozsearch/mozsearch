#!/usr/bin/env python3

import boto3
from datetime import datetime, timedelta
import sys
import os.path

# Usage: trigger_blame_rebuild.py <mozsearch-repo> <config-repo> <config-input> <branch> <channel>
#  e.g.: trigger_blame_rebuild.py https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla config1.json master release

def trigger(mozsearch_repo, config_repo, config_input, branch, channel):
    ec2 = boto3.resource('ec2')
    client = boto3.client('ec2')

    running = ec2.instances.filter(Filters=[{'Name': 'tag-key', 'Values': ['blame-builder']},
                                           {'Name': 'tag:channel', 'Values': [channel]},
                                           {'Name': 'instance-state-name', 'Values': ['running']}])
    for instance in running:
        print("Terminating existing running blame-rebuilder %s for channel %s" % (instance.instance_id, channel))
        instance.terminate()

    timeout_hours = 7 * 24 # upper bound on how long we expect the blame-rebuild to take
    user_data = '''#!/usr/bin/env bash

cd ~ubuntu
sudo -i -u ubuntu ./update.sh "{branch}" "{mozsearch_repo}" "{config_repo}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/main.sh rebuild-blame.sh "{timeout_hours}" "{branch}" "{channel}" config "{config_input}"
'''.format(branch=branch, channel=channel, mozsearch_repo=mozsearch_repo, config_repo=config_repo, config_input=config_input, timeout_hours=timeout_hours)

    block_devices = []

    images = client.describe_images(Filters=[{'Name': 'name', 'Values': ['indexer-20.04']}])
    image_id = images['Images'][0]['ImageId']

    launch_spec = {
        'ImageId': image_id,
        'KeyName': 'Main Key Pair',
        'SecurityGroups': ['indexer-secure'],
        'UserData': user_data,
        'InstanceType': 'm5d.2xlarge',
        'BlockDeviceMappings': block_devices,
        'IamInstanceProfile': {
            'Name': 'indexer-role',
        },
        'TagSpecifications': [{
            'ResourceType': 'instance',
            'Tags': [{
                'Key': 'blame-builder',
                'Value': str(datetime.now())
            }, {
                'Key': 'channel',
                'Value': channel,
            }, {
                'Key': 'branch',
                'Value': branch,
            }, {
                'Key': 'mrepo',
                'Value': mozsearch_repo,
            }, {
                'Key': 'crepo',
                'Value': config_repo,
            }, {
                'Key': 'cfile',
                'Value': config_input,
            }],
        }],
    }
    return client.run_instances(MinCount=1, MaxCount=1, **launch_spec)


if __name__ == '__main__':
    mozsearch_repo = sys.argv[1]
    config_repo = sys.argv[2]
    config_input = sys.argv[3]
    branch = sys.argv[4]
    channel = sys.argv[5]

    trigger(mozsearch_repo, config_repo, config_input, branch, channel)
