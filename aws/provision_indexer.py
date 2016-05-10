import boto3
from datetime import datetime, timedelta
import sys
import os.path

# Usage: provision_indexer.py [release|dev]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

# 'release' or 'dev'
channel = sys.argv[1]

userData = open(os.path.join(os.path.dirname(sys.argv[0]), 'indexer-startup.sh')).read()
userData = userData.replace('#SETCHANNEL', 'CHANNEL={}'.format(channel))

blockDevices = []

launchSpec = {
    'ImageId': 'ami-5189a661',
    'KeyName': 'Main Key Pair',
    'SecurityGroups': ['indexer'],
    'UserData': userData,
    'InstanceType': 'c3.2xlarge',
    'BlockDeviceMappings': blockDevices,
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
        LaunchSpecification=launchSpec,
        ValidUntil=validUntil,
    )

    requestId = r['SpotInstanceRequests'][0]['SpotInstanceRequestId']

    print r
else:
    r = client.run_instances(
        ImageId='ami-5189a661', # Ubuntu 14.04
        MinCount=1,
        MaxCount=1,
        KeyName='Main Key Pair',
        SecurityGroups=['indexer'],
        UserData=userData,
        InstanceType='c3.2xlarge',
        BlockDeviceMappings=blockDevices,
        IamInstanceProfile={'Name': 'indexer-role'},
    )

    print r
