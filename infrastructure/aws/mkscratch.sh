#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

#
#
# Create the "/index" directory where each specific tree's indexing
# byproducts live while the indexing is ongoing.  To do this, we need to figure
# out what the device's partition is.
#
# If we run `nvme list -o json` on an m5d.2xlarge we get output like the
# following (as of Aug 7th, 2024):
#
# {
#   "Devices" : [
#     {
#       "NameSpace" : 1,
#       "DevicePath" : "/dev/nvme0n1",
#       "Firmware" : "1.0",
#       "Index" : 0,
#       "ModelNumber" : "Amazon Elastic Block Store",
#       "ProductName" : "Non-Volatile memory controller: Amazon.com, Inc. NVMe EBS Controller",
#       "SerialNumber" : "vol0bbec15c8360404c3",
#       "UsedBytes" : 21474836480,
#       "MaximumLBA" : 41943040,
#       "PhysicalSize" : 21474836480,
#       "SectorSize" : 512
#     },
#     {
#       "NameSpace" : 1,
#       "DevicePath" : "/dev/nvme1n1",
#       "Firmware" : "0",
#       "Index" : 1,
#       "ModelNumber" : "Amazon EC2 NVMe Instance Storage",
#       "ProductName" : "Non-Volatile memory controller: Amazon.com, Inc. Me SSD Controller",
#       "SerialNumber" : "AWS271A679A97BB1AC18",
#       "UsedBytes" : 300000000000,
#       "MaximumLBA" : 585937500,
#       "PhysicalSize" : 300000000000,
#       "SectorSize" : 512
#     }
#   ]
# }
#
# If we run it on an m5d.4xlarge which has 2 instance SSD's attached (as of
# Aug 7th, 2024), we get:
#
# {
#   "Devices" : [
#     {
#       "NameSpace" : 1,
#       "DevicePath" : "/dev/nvme0n1",
#       "Firmware" : "1.0",
#       "Index" : 0,
#       "ModelNumber" : "Amazon Elastic Block Store",
#       "ProductName" : "Non-Volatile memory controller: Amazon.com, Inc. NVMe EBS Controller",
#       "SerialNumber" : "vol0469084c4103e93f6",
#       "UsedBytes" : 21474836480,
#       "MaximumLBA" : 41943040,
#       "PhysicalSize" : 21474836480,
#       "SectorSize" : 512
#     },
#     {
#       "NameSpace" : 1,
#       "DevicePath" : "/dev/nvme1n1",
#       "Firmware" : "0",
#       "Index" : 1,
#       "ModelNumber" : "Amazon EC2 NVMe Instance Storage",
#       "ProductName" : "Non-Volatile memory controller: Amazon.com, Inc. Me SSD Controller",
#       "SerialNumber" : "AWS3874D9799F2AE3EBE",
#       "UsedBytes" : 300000000000,
#       "MaximumLBA" : 585937500,
#       "PhysicalSize" : 300000000000,
#       "SectorSize" : 512
#     },
#     {
#       "NameSpace" : 1,
#       "DevicePath" : "/dev/nvme2n1",
#       "Firmware" : "0",
#       "Index" : 2,
#       "ModelNumber" : "Amazon EC2 NVMe Instance Storage",
#       "ProductName" : "Non-Volatile memory controller: Amazon.com, Inc. Me SSD Controller",
#       "SerialNumber" : "AWS2DECD522BEB58C35D",
#       "UsedBytes" : 300000000000,
#       "MaximumLBA" : 585937500,
#       "PhysicalSize" : 300000000000,
#       "SectorSize" : 512
#     }
#   ]
# }
#
# We are interested in the "Instance Storage" device, and so we can use jq to
# filter this.  Note that on larger instances like m5d.4xlarge, there will be
# multiple instance SSDs and currently we only want the 1st one we find.
#
# TODO: In the future we might consider doing some kind of performance RAID when
# there are multiple SSDs.
INSTANCE_STORAGE_DEV=$(sudo nvme list -o json | jq --raw-output 'first(.Devices[] | select(.ModelNumber | contains("Instance Storage")) | .DevicePath)')
sudo mkfs -t ext4 $INSTANCE_STORAGE_DEV
sudo mkdir /index
sudo mount $INSTANCE_STORAGE_DEV /index
sudo chown ubuntu:ubuntu /index

# For swap purposes, let's see if there was a 2nd instance storage; we use nth(1; ...)
# for this.  If there is no 2nd entry, we will get an empty string.
#
# FIXME: On jq 1.6, nth() prints the last item instead of an empty string
#        even if the index is out of range, which can be the same device as
#        the first one, used by INSTANCE_STORAGE_DEV.
#        We should bump jq to 1.7, but it's not available on ubuntu jammy.
SWAP_STORAGE_DEV=$(sudo nvme list -o json | jq --raw-output 'nth(1; .Devices[] | select(.ModelNumber | contains("Instance Storage")) | .DevicePath)')

# FIXME: Once jq is bumped to 1.7+, the comparison against INSTANCE_STORAGE_DEV
#        should be removed.
if [[ $SWAP_STORAGE_DEV && $SWAP_STORAGE_DEV != $INSTANCE_STORAGE_DEV ]]; then
  sudo mkswap $SWAP_STORAGE_DEV
  sudo swapon $SWAP_STORAGE_DEV
else
  SWAP_FILE=/index/swapfile
  # 8 GiB swap
  sudo dd if=/dev/zero of=$SWAP_FILE bs=128M count=64
  sudo chmod 600 $SWAP_FILE
  sudo mkswap $SWAP_FILE
  sudo swapon $SWAP_FILE
fi
