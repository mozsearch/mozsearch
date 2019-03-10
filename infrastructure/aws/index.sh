#!/usr/bin/env bash

exec &> /home/ubuntu/index-log

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

echo "Branch is $BRANCH"
echo "Channel is $CHANNEL"

export AWS_ROOT=$(realpath $MOZSEARCH_PATH/infrastructure/aws)
VOLUME_ID=$(python $AWS_ROOT/attach-index-volume.py $CHANNEL $EC2_INSTANCE_ID)

set +o pipefail   # The grep command below can return nonzero, so temporarily allow pipefail
while true
do
    COUNT=$(lsblk | grep xvdf | wc -l)
    if [ $COUNT -eq 1 ]
    then break
    fi
    sleep 1
done
set -o pipefail

echo "Index volume detected"

sudo mkfs -t ext4 /dev/xvdf
sudo mkdir /index
sudo mount /dev/xvdf /index
sudo chown ubuntu.ubuntu /index

$MOZSEARCH_PATH/infrastructure/indexer-setup.sh $CONFIG_REPO_PATH $CONFIG_INPUT /index
case "$CHANNEL" in
release | mozilla-releases )
    # Only upload files on release channels.
    $MOZSEARCH_PATH/infrastructure/indexer-upload.sh $CONFIG_REPO_PATH /index
    ;;
* )
    ;;
esac
$MOZSEARCH_PATH/infrastructure/indexer-run.sh $CONFIG_REPO_PATH /index

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
python $AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs `date -Iminutes`

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
