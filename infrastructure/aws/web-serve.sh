#!/usr/bin/env bash

exec &> /home/ubuntu/web-serve-log

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..

CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"

# The EBS volume will no longer be mounted at /dev/xvdf but instead at an
# arbitrarily assigned nvme id.  However, since we only have a single EBS volume
# and we dynamically attach it, we're pretty certain what the ID will be:
EBS_NVME_DEV=nvme1n1

set +o pipefail   # The grep command below can return nonzero, so temporarily allow pipefail
for (( i = 0; i < 3600; i++ ))
do
    COUNT=$(lsblk | grep $EBS_NVME_DEV | wc -l)
    if [ $COUNT -eq 1 ]
    then break
    fi
    sleep 1
done
set -o pipefail

echo "Index volume detected"

mkdir ~ubuntu/index
sudo mount /dev/$EBS_NVME_DEV ~ubuntu/index

$MOZSEARCH_PATH/web-server-setup.sh $CONFIG_REPO $CONFIG_INPUT index ~ hsts
$MOZSEARCH_PATH/web-server-run.sh $CONFIG_REPO index ~
