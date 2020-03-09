#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 6 ]
then
    echo "usage: $0 <branch> <channel> <mozsearch-repo-url> <config-repo-url> <config-repo-path> <config-file-name>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/../..

BRANCH=$1
CHANNEL=$2
MOZSEARCH_REPO_URL=$3
CONFIG_REPO_URL=$4
CONFIG_REPO_PATH=$(readlink -f $5)
CONFIG_INPUT="$6"

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)


echo "Creating index-scratch on local instance SSD"

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
#       "MaximiumLBA" : 585937500,
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
#       "MaximiumLBA" : 16777216,
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


echo "Branch is $BRANCH"
echo "Channel is $CHANNEL"

export AWS_ROOT=$(realpath $MOZSEARCH_PATH/infrastructure/aws)
VOLUME_ID=$(python $AWS_ROOT/attach-index-volume.py $CHANNEL $EC2_INSTANCE_ID)

# Since we know the volume id and it's exposed as the `SerialNumber` in the JSON
# structure (see above), we can look that up here too.  Note that we need to
# remove the/any dash from the volume id.
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

# Create the "index" directory where the byproducts of indexing will permanently
# live.
sudo mkfs -t ext4 $EBS_NVME_DEV
sudo mkdir /index
sudo mount $EBS_NVME_DEV /index
sudo chown ubuntu.ubuntu /index

# Do indexer setup locally on disk.
$MOZSEARCH_PATH/infrastructure/indexer-setup.sh $CONFIG_REPO_PATH $CONFIG_INPUT /mnt/index-scratch
case "$CHANNEL" in
release | mozilla-releases )
    # Only upload files on release channels.
    $MOZSEARCH_PATH/infrastructure/indexer-upload.sh $CONFIG_REPO_PATH /mnt/index-scratch
    ;;
* )
    ;;
esac
# Now actually run the indexing, telling the scripts to move the data to the
# permanent index directory.
$MOZSEARCH_PATH/infrastructure/indexer-run.sh $CONFIG_REPO_PATH /mnt/index-scratch /index

date
echo "Indexing complete"

# Copy indexing log to index mount so it's easy to get to from the
# web server instance
cp ~ubuntu/index-log /index/index-log

sudo umount /index

python $AWS_ROOT/detach-volume.py $EC2_INSTANCE_ID $VOLUME_ID
python $AWS_ROOT/trigger-web-server.py $BRANCH $CHANNEL $MOZSEARCH_REPO_URL $CONFIG_REPO_URL $CONFIG_INPUT $VOLUME_ID

case "$CHANNEL" in
release | mozilla-releases )
    DEST_EMAIL="searchfox-aws@mozilla.com"
    ;;
* )
    # For dev-channel runs, send emails to the author of the HEAD commit in the
    # repo.
    DEST_EMAIL=$(git --git-dir="$MOZSEARCH_PATH/.git" show --format="%aE" --no-patch HEAD)
    ;;
esac

$AWS_ROOT/send-warning-email.py "[$CHANNEL/$BRANCH]" "$DEST_EMAIL"

gzip -k ~ubuntu/index-log
python $AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs "$(date -Iminutes)_${CHANNEL}_${CONFIG_INPUT%.*}"

case "$CHANNEL" in
release | mozilla-releases )
    # Don't send completion email notification for release channel.
    ;;
* )
    $AWS_ROOT/send-done-email.py "[$CHANNEL/$BRANCH]" "$DEST_EMAIL"
    ;;
esac

# Give logger time to catch up
sleep 30
python $AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
