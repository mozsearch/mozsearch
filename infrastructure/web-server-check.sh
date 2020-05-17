#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root>"
    exit 1
fi

export MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)
export CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)
CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
  . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
  $MOZSEARCH_PATH/scripts/check-index.sh $CONFIG_FILE $TREE_NAME "filesystem" "http://localhost/"
done