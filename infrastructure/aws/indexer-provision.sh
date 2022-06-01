#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

date

# as of Ubuntu 22.04 /home/ubuntu is no longer o+rx so we need to manually do it.
chmod a+rx ~

# ## Script Ordering
#
# This script now gets run before the non-AWS provisioner so that we can
# increase the size of the partition now and have the rest of the process
# benefit from the increased partition size.  This does mean that we do some
# redundant things necessary to make this script work independently of that
# script.

# We need to know about our packages below...
sudo apt-get update

# We want the NVME tools, that's how EBS gets mounted now on "nitro" instances.
sudo apt-get install -y nvme-cli

# In order to do the re-partitioning again, we need jq now, even though we'll
# also try and install it in the non-AWS scripts.
sudo apt-get install -y jq

# Need pip3 to get the awscli
sudo apt-get install -y python3-pip

# Need AWS client too.
sudo pip3 install boto3 awscli rich

date

# Size up our root partition to 12G
#
# To this end we need to know the volume id in order to issue an EBS resizing
# command.  Note that the select constraint here is intended more as a check
# that our assumption about partition sizes hasn't changed, as when provisioning
# there should only be this single EBS mount.
ROOT_VOL_ID=$(sudo nvme list -o json | jq --raw-output '.Devices[] | select(.PhysicalSize < 9000000000) | .SerialNumber | sub("^vol"; "vol-")')
AWS_REGION=us-west-2
# The size is in gigs.
aws ec2 modify-volume --region ${AWS_REGION} --volume-id ${ROOT_VOL_ID} --size 12
# Re: hardcoded devices: The devices should currently be stable.
#
# We use an until loop because it can take some time for the change to
# propagate to this VM.  The error will look like:
#   "NOCHANGE: partition 1 is size 16775135. it cannot be grown"
# And success will look like:
#   "CHANGED: partition=1 start=2048 old: size=16775135 end=16777183 new: size=25163743 end=25165791"
#
# The 5 is arbitrary in both cases.
sleep 5
until sudo growpart /dev/nvme0n1 1
do
  sleep 5
done
sudo resize2fs /dev/nvme0n1p1

date
