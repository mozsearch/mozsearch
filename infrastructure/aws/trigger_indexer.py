import boto3
from datetime import datetime, timedelta
import sys
import os.path

# Usage: trigger_indexer.py <mozsearch-repo> <config-repo> <config-input> <branch> [release|dev]

def trigger(mozsearch_repo, config_repo, config_input, branch, channel, spot=False):
    ec2 = boto3.resource('ec2')
    client = boto3.client('ec2')

    user_data = '''#!/usr/bin/env bash

cd ~ubuntu
sudo -i -u ubuntu ./update.sh "{branch}" "{mozsearch_repo}" "{config_repo}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/main.sh "{branch}" "{channel}" "{mozsearch_repo}" "{config_repo}" config "{config_input}"
'''.format(branch=branch, channel=channel, mozsearch_repo=mozsearch_repo, config_repo=config_repo, config_input=config_input)

    block_devices = []

    images = client.describe_images(Filters=[{'Name': 'name', 'Values': ['indexer-18.04']}])
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
                'Value': config_repo
            }, {
                'Key': 'cfile',
                'Value': config_input
            }],
        }],
    }

    validUntil = datetime.now() + timedelta(days=1)

    if spot:
        r = client.request_spot_instances(
            SpotPrice='0.10',
            InstanceCount=1,
            Type='one-time',
            BlockDurationMinutes=300,
            LaunchSpecification=launch_spec,
            ValidUntil=validUntil,
        )

        requestId = r['SpotInstanceRequests'][0]['SpotInstanceRequestId']
    else:
        r = client.run_instances(MinCount=1, MaxCount=1, **launch_spec)
    return r


if __name__ == '__main__':
    mozsearch_repo = sys.argv[1]
    config_repo = sys.argv[2]
    config_input = sys.argv[3]
    branch = sys.argv[4]

    # 'release' or 'dev'
    channel = sys.argv[5]

    trigger(mozsearch_repo, config_repo, config_input, branch, channel, spot=False)
