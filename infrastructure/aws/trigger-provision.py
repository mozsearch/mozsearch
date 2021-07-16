#!/usr/bin/env python3

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

# Performing lookup from https://cloud-images.ubuntu.com/locator/ec2/ we get:
# us-west-2	focal	20.04 LTS	amd64	hvm:ebs-ssd	20210129	ami-0928f4202481dfdf6	hvm
# and then clicking on the AMI link (already logged into EC2) and hitting
# "previous" we get the full path:
# ubuntu/images/hvm-ssd/ubuntu-focal-20.04-amd64-server-20210129 - ami-0928f4202481dfdf6
image_id = 'ami-0928f4202481dfdf6'

launch_spec = {
    'ImageId': image_id,
    'KeyName': 'Main Key Pair',
    'SecurityGroups': ['indexer'],
    'UserData': user_data,
    'InstanceType': 'c5d.2xlarge',
    'BlockDeviceMappings': [],
    # In order to be able to automatically have the `aws` command work so that
    # we can resize our root partition, we need to assign an IAM role.
    #
    # This also could potentially let the provisioning process checkpoint itself
    # into a new AMI.
    'IamInstanceProfile': {
        'Name': 'indexer-role',
    },
    'TagSpecifications': [{
        'ResourceType': 'instance',
        'Tags': [{
            'Key': 'provisioner',
            'Value': sys.argv[1],
         }],
    }],
}

client.run_instances(MinCount=1, MaxCount=1, **launch_spec)
