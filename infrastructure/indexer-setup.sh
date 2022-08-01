#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <index-path>"
    exit 1
fi

export MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)
export CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"
export WORKING=$(readlink -f $3)

if [ -z "${CLEAN_WORKING:-}" ]; then
    echo "Keeping old contents of $WORKING/. Set CLEAN_WORKING=1 to remove the contents of $WORKING/."
else
    echo "Removing old contents of $WORKING/."
    rm -rf $WORKING/*
fi

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $CONFIG_INPUT $WORKING
CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    mkdir -p $INDEX_ROOT

    $CONFIG_REPO/$TREE_NAME/setup || inhibit_upload || handle_tree_error "tree setup script"
done
