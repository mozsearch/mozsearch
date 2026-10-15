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
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/../..

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
EBS_JQ_QUERY=".Devices[] | select(.SerialNumber == \"${VOLUME_ID/-/}\") | .DevicePath"

set +o pipefail   # The grep command below can return nonzero, so temporarily allow pipefail
for (( i = 0; i < 3600; i++ ))
do
    EBS_NVME_DEV=$(sudo nvme list -o json | jq --raw-output "$EBS_JQ_QUERY")
    if [[ $EBS_NVME_DEV ]]
    then break
    fi
    sleep 1
done
set -o pipefail

echo "Index volume detected"

LOCAL_JQ_QUERY=".Devices[] | select(.ModelNumber == \"Amazon EC2 NVMe Instance Storage\") | .DevicePath"
LOCAL_NVME_DEV=$(sudo nvme list -o json | jq --raw-output "$LOCAL_JQ_QUERY")

sudo mkdir /index
if [[ $LOCAL_NVME_DEV ]]; then
    # Copy the index to the instance's SSD, at the block level if the index
    # volume's filesystem fits on it: e2image copies the filesystem's used
    # blocks, in big sequential reads, which only the volume's throughput
    # bounds, and resize2fs then grows the copy to fill the SSD.  Copying the
    # index's files one by one is bound by the volume's IOPS instead, since
    # the index has millions of small files: rsync took half an hour for
    # config1's 3.5 million files, and over 2 hours for config2's index.
    date

    EBS_FS_INFO=$(sudo dumpe2fs -h $EBS_NVME_DEV 2>/dev/null || true)
    EBS_FS_BLOCKS=$(sed -n 's/^Block count: *//p' <<< "$EBS_FS_INFO")
    EBS_FS_BLOCK_SIZE=$(sed -n 's/^Block size: *//p' <<< "$EBS_FS_INFO")
    EBS_FS_BYTES=$(( ${EBS_FS_BLOCKS:-0} * ${EBS_FS_BLOCK_SIZE:-0} ))
    LOCAL_BYTES=$(sudo blockdev --getsize64 $LOCAL_NVME_DEV)
    if (( EBS_FS_BYTES > 0 && EBS_FS_BYTES <= LOCAL_BYTES )) &&
        sudo e2image -ra $EBS_NVME_DEV $LOCAL_NVME_DEV; then
        sudo mount $LOCAL_NVME_DEV /index
        # (Online, since the copy's filesystem is clean but unchecked.  It
        # works at the volume's size too.)
        sudo resize2fs $LOCAL_NVME_DEV ||
            echo "WARNING: The index's filesystem couldn't be grown to fill the SSD."
    else
        echo "The index volume's filesystem can't be copied to the SSD at the block level, so copying its files."
        sudo mkfs -t ext4 -F $LOCAL_NVME_DEV
        sudo mount $LOCAL_NVME_DEV /index
        sudo mkdir /index-ebs
        sudo mount $EBS_NVME_DEV /index-ebs
        sudo rsync -a /index-ebs/ /index/
        sudo umount /index-ebs
        sudo rmdir /index-ebs
    fi

    date
else
    sudo mount $EBS_NVME_DEV /index
fi
sudo chown ubuntu:ubuntu /index

# Swap on the instance's SSD, so that briefly needing more memory than the
# instance has (ex: the pipeline-server's growth, which web-server-run.sh
# allows for, or a try push's build) slows it down rather than having the OOM
# killer kill a server: SWAP_GIB GiB, 16 by default.  (Not on the EBS index
# volume of an instance without an SSD.)
if [[ $LOCAL_NVME_DEV ]]; then
    SWAP_FILE=/index/swapfile
    sudo fallocate -l "${SWAP_GIB:-16}G" $SWAP_FILE
    sudo chmod 600 $SWAP_FILE
    sudo mkswap $SWAP_FILE
    sudo swapon $SWAP_FILE
fi

# Create a writable directory for nginx caching purposes on the indexer's EBS
# store.  We choose this spot because:
# - It has more free space than our instance's root FS (~3.2G of 7.7G avail.)
# - It's bigger and hence also gets more EBS IO ops.
# In case the index volume is reused (e.g. migrating from other instance),
# clear the cache to avoid unexpected behavior.
NGINX_CACHE_DIR=/index/nginx-cache
rm -rf $NGINX_CACHE_DIR
mkdir $NGINX_CACHE_DIR

LOG_DIR=/index/logs
rm -rf $LOG_DIR
mkdir $LOG_DIR

# Redirect port 80 to port 16995, where our root-less nginx listens
# From https://askubuntu.com/a/595955
sudo iptables -t nat -I PREROUTING -p tcp --dport 80 -j REDIRECT --to-ports 16995
sudo iptables -t nat -I OUTPUT -p tcp -o lo --dport 80 -j REDIRECT --to-ports 16995

case "$CHANNEL" in
release* )
    DEST_EMAIL="searchfox-aws@mozilla.com"
    ;;
* )
    # For dev-channel runs, send emails to the author of the HEAD commit in the
    # repo.
    DEST_EMAIL=$(git --git-dir="$MOZSEARCH_PATH/.git" show --format="%aE" --no-patch HEAD)
    ;;
esac

INDEX_PATH=/index
SERVER_ROOT=~

$MOZSEARCH_PATH/infrastructure/web-server-setup.sh \
    $CONFIG_REPO $CONFIG_INPUT $INDEX_PATH $SERVER_ROOT $LOG_DIR \
    hsts $NGINX_CACHE_DIR
$MOZSEARCH_PATH/infrastructure/web-server-run.sh \
    $CONFIG_REPO $INDEX_PATH $SERVER_ROOT $LOG_DIR \
    $CHANNEL $DEST_EMAIL
