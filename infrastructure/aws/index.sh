#!/bin/bash

exec &> /home/ubuntu/index-log

set -e
set -x

if [ $# != 5 ]
then
    echo "usage: $0 <branch> <channel> <mozsearch-repo-url> <config-repo-url> <config-repo-path>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/../..

BRANCH=$1
CHANNEL=$2
MOZSEARCH_REPO_URL=$3
CONFIG_REPO_URL=$4
CONFIG_REPO_PATH=$(readlink -f $5)

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)

echo "Branch is $BRANCH"
echo "Channel is $CHANNEL"

export AWS_ROOT=$(realpath $MOZSEARCH_PATH/infrastructure/aws)
VOLUME_ID=$(python $AWS_ROOT/attach-index-volume.py $CHANNEL $EC2_INSTANCE_ID)

while true
do
    COUNT=$(lsblk | grep xvdf | wc -l)
    if [ $COUNT -eq 1 ]
    then break
    fi
    sleep 1
done

echo "Index volume detected"

sudo mkfs -t ext4 /dev/xvdf
sudo mkdir /index
sudo mount /dev/xvdf /index
sudo chown ubuntu.ubuntu /index

$MOZSEARCH_PATH/infrastructure/indexer-setup.sh $CONFIG_REPO_PATH /index
if [ $CHANNEL == release ]
then
    # Only upload files on release channel.
    $MOZSEARCH_PATH/infrastructure/indexer-upload.sh $CONFIG_REPO_PATH /index
fi
$MOZSEARCH_PATH/infrastructure/indexer-run.sh $CONFIG_REPO_PATH /index

date
echo "Indexing complete"

sudo umount /index

python $AWS_ROOT/detach-volume.py $EC2_INSTANCE_ID $VOLUME_ID
python $AWS_ROOT/trigger-web-server.py $BRANCH $CHANNEL $MOZSEARCH_REPO_URL $CONFIG_REPO_URL $VOLUME_ID

gzip -k ~ubuntu/index-log
python $AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs `date -Iminutes`

if [ $CHANNEL != release ]
then
    # Don't send completion email notification for release channel. For other
    # channels send it to the author of the HEAD commit in the repo
    DEST_EMAIL=$(git --git-dir="$MOZSEARCH_PATH/.git" show --format="%aE" --no-patch HEAD)
    $AWS_ROOT/send-done-email.py "[$CHANNEL/$BRANCH]" "$DEST_EMAIL"
fi

# Give logger time to catch up
sleep 30
python $AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
