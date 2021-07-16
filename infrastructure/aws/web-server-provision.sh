#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# We want the NVME tools, that's how EBS gets mounted now on "nitro" instances.
sudo apt-get install -y nvme-cli

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

