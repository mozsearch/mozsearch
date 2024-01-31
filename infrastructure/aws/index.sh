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
CONFIG_FILE_NAME="$6"

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)

# Find the revisions we actually checked out so that we can pass these to the
# web server because indexer and web server are a matched pair and we don't
# randomly want it running a different revision!
pushd mozsearch
MOZSEARCH_REV=$(git rev-parse HEAD)
popd

pushd config
CONFIG_REV=$(git rev-parse HEAD)
popd

echo "Branch is $BRANCH"
echo "  mozsearch repo $MOZSEARCH_REPO_URL rev is $MOZSEARCH_REV"
echo "  config repo $CONFIG_REPO_URL rev is $CONFIG_REV"
echo "Channel is $CHANNEL"

VOLUME_ID=$($AWS_ROOT/attach-index-volume.py $CHANNEL $EC2_INSTANCE_ID)

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
$MOZSEARCH_PATH/infrastructure/indexer-setup.sh $CONFIG_REPO_PATH $CONFIG_FILE_NAME /mnt/index-scratch
case "$CHANNEL" in
release* )
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

# Because it's possible for shells to be in the "/index" dir and for this to
# cause problems unmounting /index (:asuth has done this a lot...), terminate
# all extant ssh sessions for our normal user as a one-off.  This does not
# preclude logging back in.
#
# pkill returns an exit status of 1 if no processes matched, so we need to make
# sure that we don't care about the exit code as this is just a hygiene measure,
# we don't actually expect for there to usually be sshd instances.
pkill -u ubuntu sshd || true
# And then sleep a little just in case there's some cleanup time required.
sleep 1
sudo umount /index

$AWS_ROOT/detach-volume.py $EC2_INSTANCE_ID $VOLUME_ID
$AWS_ROOT/trigger-web-server.py \
    $CHANNEL \
    $MOZSEARCH_REPO_URL \
    $MOZSEARCH_REV \
    $CONFIG_REPO_URL \
    $CONFIG_REV \
    $CONFIG_FILE_NAME \
    $VOLUME_ID \
    "$MOZSEARCH_PATH/infrastructure/web-server-check.sh" \
    $CONFIG_REPO_PATH \
    "/mnt/index-scratch"

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

$AWS_ROOT/send-warning-email.py "$CHANNEL/$BRANCH" "$DEST_EMAIL"

gzip -k ~ubuntu/index-log
$AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs "index-$(date -Iminutes)_${CHANNEL}_${CONFIG_FILE_NAME%.*}.gz"

case "$CHANNEL" in
release* )
    # Don't send completion email notification for release channel.
    ;;
* )
    $AWS_ROOT/send-done-email.py "$CHANNEL/$BRANCH" "$DEST_EMAIL"
    ;;
esac

# Give logger time to catch up
sleep 30
$AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
