#!/usr/bin/env python2

# trigger-provision.py <indexer-provision.sh | web-server-provision.sh>

from __future__ import absolute_import
import boto3
from datetime import datetime, timedelta
import sys
import os.path

provisioners = sys.argv[1:]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

script = ''
for provisioner in provisioners:
    script += open(provisioner).read() + '\n'

user_data = '''#!/usr/bin/env bash

cat > ~ubuntu/provision.sh <<"FINAL"
{script}
FINAL

chmod +x ~ubuntu/provision.sh
sudo -i -u ubuntu ~ubuntu/provision.sh > ~ubuntu/provision.log 2>&1
echo "Provisioning complete." >> ~ubuntu/provision.log
'''.format(script=script)

# us-west-2	bionic	18.04 LTS	amd64	hvm:ebs-ssd	20191113	ami-0a7d051a1c4b54f65	hvm
# ubuntu/images/hvm-ssd/ubuntu-bionic-18.04-amd64-server-20191113 - ami-0a7d051a1c4b54f65
image_id = 'ami-0a7d051a1c4b54f65'

launch_spec = {
    'ImageId': image_id,
    'KeyName': 'Main Key Pair',
    'SecurityGroups': ['indexer'],
    'UserData': user_data,
    'InstanceType': 'c5d.2xlarge',
    'BlockDeviceMappings': [],
    'TagSpecifications': [{
        'ResourceType': 'instance',
        'Tags': [{
            'Key': 'provisioner',
            'Value': sys.argv[1],
         }],
    }],
}

client.run_instances(MinCount=1, MaxCount=1, **launch_spec)
