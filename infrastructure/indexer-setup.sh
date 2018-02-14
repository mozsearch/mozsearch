#!/bin/bash

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <index-path>"
    exit 1
fi

set -e
set -x

export MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)
export CONFIG_REPO=$(readlink -f $1)
export WORKING=$(readlink -f $2)

if [ -z "$KEEP_WORKING" ]; then
    echo "Removing old contents of $WORKING/"
    rm -rf $WORKING/*
else
    echo "Keeping old contents of $WORKING/"
fi

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $WORKING
CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    mkdir -p $INDEX_ROOT

    $CONFIG_REPO/$TREE_NAME/setup
done
