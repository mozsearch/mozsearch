# trigger-provision.py <indexer-provision.sh | web-server-provision.sh>

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

# ubuntu/images/hvm-ssd/ubuntu-xenial-16.04-amd64-server-20190816 - ami-0135f076a31aebe66
image_id = 'ami-0135f076a31aebe66'

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
