#!/usr/bin/env python3

# trigger-provision.py <kind> <scripts to inline as part of the provisioning process...>
#
# where kind is one of:
# - indexer
# - web-server

from __future__ import absolute_import
import boto3
from datetime import datetime, timedelta
import sys
import os.path

kind = sys.argv[1]
provisioners = sys.argv[2:]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

script = ''
for provisioner in provisioners:
    script += open(provisioner).read() + '\n'

user_data = f'''#!/usr/bin/env bash

cat > ~ubuntu/provision.sh <<"FINAL"
{script}
FINAL

chmod +x ~ubuntu/provision.sh
sudo -i -u ubuntu ~ubuntu/provision.sh > ~ubuntu/provision.log 2>&1
AWS_ROOT=~ubuntu/mozsearch/infrastructure/aws
DEST_EMAIL="searchfox-aws@mozilla.com"
if [[ $? -ne 0 ]]; then
  # In the event of failure it's possible we don't have AWS commands, so
  # schedule our shutdown, which should STOP our EC2 instance, leaving the log
  # intact.  We schedule this to happen after 10 mins to give an opportunity for
  # investigation but also shutdown in a timely fashion if no one was actively
  # watching things.
  sudo shutdown -h +10
  echo "Provisioning failed, shutdown scheduled and sending email." >> ~ubuntu/provision.log
  $AWS_ROOT/send-provision-email.py "[{kind}]" "$DEST_EMAIL" "failed"
  exit 1
fi

echo "Provisioning complete.  Attempting Registration." >> ~ubuntu/provision.log

# AWS commands, etc. should work now if provisioning completed.
INSTANCE_ID=$(ec2metadata --instance-id)
DATE_STAMP=$(date -Idate)
aws ec2 create-image \
    --instance-id $INSTANCE_ID \
    --name "{kind}-$DATE_STAMP" \
    --tag-specifications "ResourceType=image,Tags=[{{Key={kind},Value=$DATE_STAMP}}]" \
    --no-reboot >$JSON_AMI
echo "Registration complete, sending email and scheduling shutdown." >> ~ubuntu/provision.log
sudo shutdown -h +10
$AWS_ROOT/send-provision-email.py "[{kind}]" "$DEST_EMAIL" "succeeded"
'''

# Performing lookup from https://cloud-images.ubuntu.com/locator/ec2/ by
# searching on "20.04 us-west-2 amd64" we get:
#
# us-west-2	focal	20.04 LTS	amd64	hvm:ebs-ssd	20211129	ami-0892d3c7ee96c0bf7	hvm
#
# and then we copy the ami ID into here:
image_id = 'ami-0892d3c7ee96c0bf7'

launch_spec = {
    'ImageId': image_id,
    'KeyName': 'Main Key Pair',
    'SecurityGroups': ['indexer'],
    'UserData': user_data,
    'InstanceType': 'c5d.2xlarge',
    'BlockDeviceMappings': [],
    # In order to be able to automatically have the `aws` command work so that
    # we can resize our root partition and now to create the AMI, we need to
    # assign an IAM role.
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
            'Value': kind,
         }],
    }],
}

client.run_instances(MinCount=1, MaxCount=1, **launch_spec)
