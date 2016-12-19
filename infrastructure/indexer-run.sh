#!/bin/bash

set -e
set -x

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <temp-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

CONFIG_REPO=$(readlink -f $1)
TEMP_PATH=$(readlink -f $2)

CONFIG_FILE=$TEMP_PATH/config.json

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    .  $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME
done

