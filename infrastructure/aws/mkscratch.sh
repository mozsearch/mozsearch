#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Create the "index-scratch" directory where each specific tree's indexing
# byproducts live while the indexing is ongoing.  To do this, we need to figure
# out what the device's partition is.
#
# If we run `nvme list -o json` we get output like the following:
#
# {
#   "Devices" : [
#     {
#       "DevicePath" : "/dev/nvme0n1",
#       "Firmware" : "0",
#       "Index" : 0,
#       "ModelNumber" : "Amazon EC2 NVMe Instance Storage",
#       "ProductName" : "Unknown Device",
#       "SerialNumber" : "AWS143416FC5A55CA413",
#       "UsedBytes" : 300000000000,
#       "MaximumLBA" : 585937500,
#       "PhysicalSize" : 300000000000,
#       "SectorSize" : 512
#     },
#     {
#       "DevicePath" : "/dev/nvme1n1",
#       "Firmware" : "1.0",
#       "Index" : 1,
#       "ModelNumber" : "Amazon Elastic Block Store",
#       "ProductName" : "Unknown Device",
#       "SerialNumber" : "vol0222cf21e3b3dfbc4",
#       "UsedBytes" : 0,
#       "MaximumLBA" : 16777216,
#       "PhysicalSize" : 8589934592,
#       "SectorSize" : 512
#     }
#   ]
# }
#
# We are interested in the "Instance Storage" device, and so we can use jq to
# filter this.
INSTANCE_STORAGE_DEV=$(sudo nvme list -o json | jq --raw-output '.Devices[] | select(.ModelNumber | contains("Instance Storage")) | .DevicePath')
sudo mkfs -t ext4 $INSTANCE_STORAGE_DEV
sudo mount $INSTANCE_STORAGE_DEV /mnt
sudo mkdir /mnt/index-scratch
sudo chown ubuntu.ubuntu /mnt/index-scratch
