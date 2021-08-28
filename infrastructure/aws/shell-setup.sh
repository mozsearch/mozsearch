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

echo "Branch is $BRANCH"
echo "Channel is $CHANNEL"
echo ""
echo "For this shell we are NOT creating an EC2 volume, instead you get to use"
echo "/mnt/index-scratch which is the local (fast) SSD.  This got setup in main.sh."
echo
echo "Running indexer setup, but stopping before we upload anything."

$MOZSEARCH_PATH/infrastructure/indexer-setup.sh $CONFIG_REPO_PATH $CONFIG_INPUT /mnt/index-scratch

echo "Setup complete, go crazy."
