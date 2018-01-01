#!/usr/bin/env bash

exec &> /home/ubuntu/web-serve-log

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 1 ]
then
    echo "usage: $0 <config-repo-path>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..

CONFIG_REPO=$(readlink -f $1)

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

mkdir ~ubuntu/index
sudo mount /dev/xvdf ~ubuntu/index

$MOZSEARCH_PATH/web-server-setup.sh $CONFIG_REPO index ~ hsts
$MOZSEARCH_PATH/web-server-run.sh $CONFIG_REPO index ~
