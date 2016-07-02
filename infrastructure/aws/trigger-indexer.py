import boto3
from datetime import datetime, timedelta
import sys
import os.path

# Usage: provision_indexer.py <config-repo> [release|dev]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

config_repo = sys.argv[1]

# 'release' or 'dev'
channel = sys.argv[2]

user_data = '''#!/bin/bash

cd ~ubuntu
./update.sh "{channel}" "{config_repo}"
sudo -i -u ubuntu mozsearch/infrastructure/aws/index.sh "{channel}" "{config_repo}" config
'''.format(channel=channel, config_repo=config_repo)

block_devices = []

images = client.describe_images(Filters=[{'Name': 'name', 'Values': ['indexer']}])
image_id = images['Images'][0]['ImageId']

launch_spec = {
    'ImageId': image_id,
    'KeyName': 'Main Key Pair',
    'SecurityGroups': ['indexer'],
    'UserData': user_data,
    'InstanceType': 'c3.2xlarge',
    'BlockDeviceMappings': block_devices,
    'IamInstanceProfile': {
        'Name': 'indexer-role',
    },
}

validUntil = datetime.now() + timedelta(days=1)

spot = False

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

    print r
else:
    r = client.run_instances(MinCount=1, MaxCount=1, **launch_spec)
    print r
