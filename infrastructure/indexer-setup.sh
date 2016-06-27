#!/bin/bash

set -e
set -x

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <base-path> <temp-path>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..
MOZSEARCH_ROOT=$MOZSEARCH_PATH

CONFIG_REPO=$(readlink -f $1)
BASE=$(readlink -f $2)
TEMP=$(readlink -f $3)

CONFIG_FILE=$TEMP/config.json

export MOZSEARCH_PATH
export BASE
export TEMP
envsubst < $CONFIG_REPO/config.json > $CONFIG_FILE

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE repos)
do
   .  $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
   mkdir -p $INDEX_ROOT

   export TEMP_PATH=$TEMP/$TREE_NAME
   mkdir -p $TEMP_PATH

    $CONFIG_REPO/$TREE_NAME/setup
    $CONFIG_REPO/$TREE_NAME/update
done
