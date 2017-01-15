#!/bin/bash

set -e
set -x

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <base-path> <temp-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

export CONFIG_REPO=$(readlink -f $1)
BASE=$(readlink -f $2)
TEMP=$(readlink -f $3)

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $BASE $TEMP

CONFIG_FILE=$TEMP/config.json

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
   .  $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
   mkdir -p $INDEX_ROOT

   export TEMP_PATH=$TEMP/$TREE_NAME
   mkdir -p $TEMP_PATH

    $CONFIG_REPO/$TREE_NAME/setup
done
