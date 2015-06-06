#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

PATH=$PATH:$PWD/env/bin

INSTANCE=$(aws ec2 run-instances \
    --user-data file://indexer-startup.sh \
    --image-id ami-5189a661 \
    --count 1 \
    --instance-type m3.large \
    --key-name "Main Key Pair" \
    --security-groups launch-wizard-1 \
    --block-device-mappings "[{\"DeviceName\":\"/dev/xvdc\", \"Ebs\": {\"VolumeSize\":30, \"DeleteOnTermination\":false,\"VolumeType\":\"gp2\"}}]" \
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

