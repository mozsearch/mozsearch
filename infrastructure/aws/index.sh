#!/bin/bash

exec &> /home/ubuntu/index-log

set -e
set -x

if [ $# != 2 ]
then
    echo "usage: $0 <channel> <config-repo-path>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..

CHANNEL=$1
CONFIG_REPO=$(readlink -f $2)

sudo mkdir -p /mnt/tmp
sudo chown ubuntu.ubuntu /mnt/tmp

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)

mkdir ~/.aws
cat > ~/.aws/config <<"STOP"
[default]
region = us-west-2
STOP

export INDEX_TMP=/mnt/tmp

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

$MOZSEARCH_PATH/indexer-setup.sh $CONFIG_REPO /index /mnt/tmp
$MOZSEARCH_PATH/indexer-run.sh $CONFIG_REPO /mnt/tmp

date
echo "Indexing complete"

sudo umount /index

python $AWS_ROOT/detach-volume.py $EC2_INSTANCE_ID $VOLUME_ID
python $AWS_ROOT/trigger-web-server.py $CHANNEL $VOLUME_ID

gzip -k ~ubuntu/index-log
python $AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs `date -Iminutes`

# Give logger time to catch up
sleep 30
python $AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
popd
