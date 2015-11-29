#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

PATH=$PATH:$PWD/env/bin

INSTANCE=$(aws ec2 run-instances \
    --user-data file://web-server-startup.sh \
    --image-id ami-5189a661 \
    --count 1 \
    --instance-type t2.medium \
    --key-name "Main Key Pair" \
    --security-groups web-server \
    --placement "AvailabilityZone=us-west-2a" \
    --query 'Instances[0].InstanceId')

eval INSTANCE=$INSTANCE
echo Instance is $INSTANCE

while true
do
    IP=$(aws ec2 describe-instances \
	--output text \
	--query 'Reservations[0].Instances[0].PublicIpAddress' \
	--instance-ids $INSTANCE)
    echo IP is $IP
    if [ $IP != "None" ]
    then break
    fi

    sleep 1
done

echo $IP

VOLUME=$(aws ec2 describe-volumes \
    --filters Name=status,Values=available \
    --query Volumes[0].VolumeId)

eval VOLUME=$VOLUME
echo Volume is $VOLUME

aws ec2 attach-volume \
    --volume-id $VOLUME \
    --instance-id $INSTANCE \
    --device /dev/xvdf

