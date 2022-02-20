#!/usr/bin/env bash

exec &> /home/ubuntu/web-serve-log

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <volume-id>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..

CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"

VOLUME_ID=$3

# The EBS volume will no longer be mounted at /dev/xvdf but instead at an
# arbitrarily assigned nvme id.
#
# If we run `nvme list -o json` we get output like the following (note that the
# below is from an indexer with an attached disk, not a web server, but you get
# the idea of the structure):
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
# Note that the volume id is exposed as the serial number, so we can use jq to
# locate the given device.  (We do need to remove any dashes, however.)
JQ_QUERY=".Devices[] | select(.SerialNumber == \"${VOLUME_ID/-/}\") | .DevicePath"

set +o pipefail   # The grep command below can return nonzero, so temporarily allow pipefail
for (( i = 0; i < 3600; i++ ))
do
    EBS_NVME_DEV=$(sudo nvme list -o json | jq --raw-output "$JQ_QUERY")
    if [[ $EBS_NVME_DEV ]]
    then break
    fi
    sleep 1
done
set -o pipefail

echo "Index volume detected"

mkdir ~ubuntu/index
sudo mount $EBS_NVME_DEV ~ubuntu/index

# Create a writable directory for nginx caching purposes on the indexer's EBS
# store.  We choose this spot because:
# - It has more free space than our instance's root FS (~3.2G of 7.7G avail.)
# - It's bigger and hence also gets more EBS IO ops.
NGINX_CACHE_DIR=/home/ubuntu/index/nginx-cache
mkdir $NGINX_CACHE_DIR
sudo chown www-data:www-data $NGINX_CACHE_DIR

$MOZSEARCH_PATH/web-server-setup.sh $CONFIG_REPO $CONFIG_INPUT index ~ hsts $NGINX_CACHE_DIR
$MOZSEARCH_PATH/web-server-run.sh $CONFIG_REPO index ~
