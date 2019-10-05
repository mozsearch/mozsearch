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

if [ -z "${KEEP_WORKING:-}" ]; then
    echo "Removing old contents of $WORKING/"
    rm -rf $WORKING/*
else
    echo "Keeping old contents of $WORKING/"
fi

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $CONFIG_INPUT $WORKING
CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    mkdir -p $INDEX_ROOT

    $CONFIG_REPO/$TREE_NAME/setup
done
