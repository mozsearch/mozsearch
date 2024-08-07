#!/usr/bin/env python3

# trigger-provision.py <kind> <scripts to inline as part of the provisioning process...>
#
# where kind is one of:
# - indexer
# - web-server

from __future__ import absolute_import
import boto3
from datetime import datetime, timedelta
import re
import sys
import os.path

# behold the world's most sophisticated and non-hacky argument parsing!
dry_run = '--dry-run' in sys.argv
# argv0 is this script itself
use_args = [x for x in sys.argv[1:] if not x.startswith('--')]

# (we no longer have argv0 here, so we are zero-indexed)
kind = use_args[0]
provisioners = use_args[1:]

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

script = ''
for provisioner in provisioners:
    script += open(provisioner).read() + '\n'

# The stdout/stderr from running the following script that we pass can be found
# on the server in `/var/log/cloud-init-output.log`.  (Note that there's also
# a file `/var/log/cloud-init.log` that is fairly verbose that will describe
# when the script is getting launched, etc.)

user_data = f'''#!/usr/bin/env bash

cat > ~ubuntu/provision.sh <<"FINAL"
{script}
FINAL

AWS_ROOT=~ubuntu/mozsearch/infrastructure/aws
DEST_EMAIL="searchfox-aws@mozilla.com"
chmod +x ~ubuntu/provision.sh
# NOTE! The exit code from the call to provision.sh below is load-bearing,
# please do not add any statements between it and the `if` below it!
sudo -i -u ubuntu ~ubuntu/provision.sh > ~ubuntu/provision.log 2>&1
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
# include the hour and minute for sufficient uniqueness
DATE_STAMP=$(date +"%Y-%m-%d-%H-%M")
AWS_REGION=us-west-2
OLD_JSON_AMI="old-ami-details.json"
NEW_JSON_AMI="new-ami-details.json"

# We get the AMI id almost immediately but the creation takes time.
aws ec2 create-image \
    --region $AWS_REGION \
    --instance-id $INSTANCE_ID \
    --name "{kind}-$DATE_STAMP" \
    --no-reboot >$NEW_JSON_AMI
NEW_AMI_ID=$(jq -r '.ImageId' $NEW_JSON_AMI)

echo "Image $NEW_AMI_ID created, waiting for it to become available." >> ~ubuntu/provision.log

# Wait for the State to become "available"
until [[ "available" == $(aws ec2 describe-images --region $AWS_REGION --image-ids $NEW_AMI_ID | jq -r '.Images[0].State') ]]
do
  sleep 10
done

echo "Image now available, updating tags." >> ~ubuntu/provision.log

# Locate the old / existing AMI (it may not exist)
aws ec2 describe-images \
    --region $AWS_REGION \
    --filters "Name=tag-key,Values={kind}" \
    >$OLD_JSON_AMI
OLD_AMI_ID=$(jq -r '.Images[0].ImageId' $OLD_JSON_AMI)

# Tag our new AMI as usable
aws ec2 create-tags \
    --region $AWS_REGION \
    --resources $NEW_AMI_ID \
    --tags "Key={kind},Value=$DATE_STAMP"

echo "New image tagged, possibly removing tag from $OLD_AMI_ID." >> ~ubuntu/provision.log

# Remove the tag from the old AMI
if [[ $OLD_AMI_ID != "null" ]]; then
    aws ec2 delete-tags \
        --region $AWS_REGION \
        --resources $OLD_AMI_ID \
        --tags "Key={kind}"
    echo "Removed tag from $OLD_AMI_ID." >> ~ubuntu/provision.log
fi

echo "Tagging complete, sending email." >> ~ubuntu/provision.log
# failsafe shutdown, although the termination should take effect first
sudo shutdown -h +10
$AWS_ROOT/send-provision-email.py "[{kind}]" "$DEST_EMAIL" "succeeded"
echo "Email sent.  Sleeping for a minute, then terminating." >> ~ubuntu/provision.log
sleep 60
aws ec2 terminate-instances --region $AWS_REGION --instance-ids $INSTANCE_ID
'''

## Shrink user_data to remain under the AWS 16384 byte user data limit
#
# We uncovered a 16384 byte limit on the user data that we weren't actively
# aware of.  Our current hacky fix is to remove comments from the payload we're
# sending.

# this converts comment lines into empty lines, but uses a negative lookahead
# assertion to leave lines that start with `#!` because those are potentially
# load-bearing.
RE_EXCLUDE_COMMENTS = re.compile('^#(?!!)[^\n]*$', re.MULTILINE)
# this merges multiple empty lines into a single line
RE_MERGE_EMPTY_LINES = re.compile('\n\n\n*')
user_data = RE_EXCLUDE_COMMENTS.sub('', user_data)
user_data = RE_MERGE_EMPTY_LINES.sub('\n\n', user_data)

if dry_run:
    print('User Data is below the line:')
    print(user_data)
    sys.exit(0)

# Performing lookup from https://cloud-images.ubuntu.com/locator/ec2/ by
# searching on "22.04 us-west-2 amd64" we get:
#
# us-west-2	Jammy Jellyfish	22.04 LTS	amd64	hvm:ebs-ssd	20240801	ami-0b33ebbed151cf740	hvm
#
# and then we copy the ami ID into here:
image_id = 'ami-0b33ebbed151cf740'

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
