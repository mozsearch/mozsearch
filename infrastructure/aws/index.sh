#!/bin/bash

exec &> /home/ubuntu/index-log

set -e
set -x

if [ $# != 4 ]
then
    echo "usage: $0 <branch> <channel> <config-url> <config-repo-path>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/../..

BRANCH=$1
CHANNEL=$2
CONFIG_URL=$3
CONFIG_REPO_PATH=$(readlink -f $4)

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)

mkdir -p ~/.aws
cat > ~/.aws/config <<"STOP"
[default]
region = us-west-2
STOP

# Create a crontab entry to send email if indexing takes too long.
$MOZSEARCH_PATH/infrastructure/aws/make-crontab.py

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
python $AWS_ROOT/trigger-web-server.py $BRANCH $CHANNEL $CONFIG_URL $VOLUME_ID

gzip -k ~ubuntu/index-log
python $AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs `date -Iminutes`

# Give logger time to catch up
sleep 30
python $AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
popd
