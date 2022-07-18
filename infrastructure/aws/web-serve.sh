#!/usr/bin/env bash

exec &> /home/ubuntu/web-serve-log

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 4 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <volume-id> <channel>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..

CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"

VOLUME_ID=$3
CHANNEL=$4

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

# release1 specific optimizations for mozilla-central to ensure we cache all of
# the important files before starting up.  This can make the difference between
# a 10 second search and a 3 second search.
#
# Note that we're only causing these to be initially cache.  vmtouch does
# support the ability to do `-dl` (daemonize and lock pages into memory), but
# we're not sure that we want to force these things to be cached forever.  We
# just want to give them a good chance to already be cached into memory.  We
# should likely run `vmtouch -v` on the backup servers manually to figure out
# how much ends up staying cached or if we're seeing evictions.
if [[ $CHANNEL == "release1" ]]; then
  date
  vmtouch -t /home/ubuntu/index/mozilla-central/crossref-extra
  date
  vmtouch -t /home/ubuntu/index/mozilla-central/crossref
  date
  vmtouch -t /home/ubuntu/index/mozilla-central/livegrep.idx
  date
fi

$MOZSEARCH_PATH/web-server-setup.sh $CONFIG_REPO $CONFIG_INPUT index ~ hsts $NGINX_CACHE_DIR
$MOZSEARCH_PATH/web-server-run.sh $CONFIG_REPO index ~
