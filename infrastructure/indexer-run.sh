#!/bin/bash

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <index-path>"
    exit 1
fi

set -e
set -x

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)

CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    .  $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME
done

