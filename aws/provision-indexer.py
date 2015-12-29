import boto3
from datetime import datetime, timedelta

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

userData = open('indexer-startup.sh').read()

blockDevices = [{'DeviceName': 'xvdf',
                 'Ebs': {
                     'VolumeSize': 30,
                     'DeleteOnTermination': True,
                     'VolumeType': 'gp2',
                 },
             },
            ]

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
